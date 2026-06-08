// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LLM-driven task executor.
//!
//! This is the half of the pipeline the foundation deferred: instead of the
//! deterministic [`crate::codegen::scaffold_cave_test`] mock, the executor asks
//! the local coder (L2) to *actually write* the crate's `src/lib.rs` — a real
//! function plus a `#[test]` — then assembles it onto a deterministic crate
//! scaffold (Cargo.toml + .gitignore), builds, and runs `cargo test`.
//!
//! The split is deliberate. The boilerplate that has nothing to do with the
//! port (manifest, gitignore) stays deterministic; only the *code under test*
//! comes from the model. That keeps a single-shot local generation reliable
//! enough to pass `cargo test` while still being a genuine LLM → compile → test
//! loop, not a canned scaffold.
//!
//! Prompt construction, lib-source extraction, Charter validation, and crate
//! assembly are pure and unit-tested here; [`LlmSmokeExecutor::run`] adds the
//! Ollama call + `cargo test` I/O and is proven by the live smoke sub-command.

use crate::charter;
use crate::codegen::{FileSet, GeneratedFile};
use crate::error::{AutopilotError, Result};
use crate::ollama::OllamaClient;
use crate::worktree::WorktreeJob;
use std::path::Path;

const LICENSE_HEADER: &str = "// SPDX-License-Identifier: AGPL-3.0-or-later\n\
                              // Copyright 2026 Cave Runtime contributors\n";

/// What the smoke executor should build.
#[derive(Debug, Clone)]
pub struct SmokeSpec {
    /// Crate (and directory) name to scaffold, e.g. `cave-test-autopilot`.
    pub crate_name: String,
    /// One-line description of the function the model should implement+test.
    pub task_desc: String,
    /// Local-LLM retries before giving up (the model may emit invalid JSON or
    /// failing code on a given attempt).
    pub max_retries: u32,
}

/// Outcome of a smoke run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmokeOutcome {
    pub crate_name: String,
    pub model: String,
    /// Attempts actually consumed (1-based on success).
    pub attempts: u32,
    /// The model returned a parseable, scope-valid lib.rs.
    pub generated: bool,
    /// `cargo test` reported a pass.
    pub passed: bool,
    pub detail: String,
}

/// System prompt for the local coder during a smoke generation.
pub fn smoke_system_prompt() -> &'static str {
    "You are a Rust engineer. Implement the requested function AND a unit test \
     for it, following TDD discipline. Output ONLY a JSON object of the form \
     {\"files\":[{\"path\":\"<crate>/src/lib.rs\",\"content\":\"<rust source>\"}]} \
     containing exactly one file. The source must be complete, compile, and the \
     test must pass. Never emit todo!() or unimplemented!() — no stubs."
}

/// Build the smoke generation prompt, pinning the exact target file path.
pub fn build_smoke_prompt(crate_name: &str, task_desc: &str) -> String {
    format!(
        "Create the library source for a standalone Rust crate named \
         `{crate_name}`. Implement {task_desc}, plus a `#[cfg(test)]` module with \
         at least one `#[test]` that exercises it with concrete assertions. \
         Return a JSON FileSet whose single file is `{crate_name}/src/lib.rs`."
    )
}

/// Pull the `src/lib.rs` content out of the model's FileSet for this crate.
pub fn extract_lib_source(fs: &FileSet, crate_name: &str) -> Result<String> {
    let want = format!("{crate_name}/src/lib.rs");
    fs.files
        .iter()
        .find(|f| f.path == want || f.path.ends_with("src/lib.rs"))
        .map(|f| f.content.clone())
        .ok_or_else(|| AutopilotError::Llm(format!("model FileSet missing {want}")))
}

/// Charter validation of generated library source: stub-free and carries a real
/// test (a `#[test]` attribute). Returns the reason on failure.
pub fn validate_lib_source(src: &str) -> Result<()> {
    let stubs = charter::scan_for_stubs(src);
    if !stubs.is_empty() {
        return Err(AutopilotError::Charter(format!(
            "generated lib.rs contains {} stub placeholder(s)",
            stubs.len()
        )));
    }
    if !src.contains("#[test]") {
        return Err(AutopilotError::Charter(
            "generated lib.rs has no #[test] — TDD requires a test".into(),
        ));
    }
    Ok(())
}

/// Assemble a buildable standalone crate from the model's lib source plus a
/// deterministic Cargo.toml + .gitignore. The lib source is licensed if the
/// model omitted the SPDX header (idempotent — never doubled).
pub fn assemble_crate(crate_name: &str, lib_src: &str) -> FileSet {
    let cargo = format!(
        "# SPDX-License-Identifier: AGPL-3.0-or-later\n\
         [package]\n\
         name = \"{crate_name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         license = \"AGPL-3.0-or-later\"\n\n\
         [dependencies]\n\n\
         # Stand alone even when scaffolded inside a parent cargo workspace\n\
         # (e.g. the autopilot worktree root under cave-runtime).\n\
         [workspace]\n"
    );
    let lib = if lib_src.contains("SPDX-License-Identifier") {
        lib_src.to_string()
    } else {
        format!("{LICENSE_HEADER}{lib_src}")
    };
    FileSet {
        files: vec![
            GeneratedFile {
                path: format!("{crate_name}/.gitignore"),
                content: "/target\nCargo.lock\n".to_string(),
            },
            GeneratedFile {
                path: format!("{crate_name}/Cargo.toml"),
                content: cargo,
            },
            GeneratedFile {
                path: format!("{crate_name}/src/lib.rs"),
                content: lib,
            },
        ],
    }
}

/// LLM-driven smoke executor: real local-model generation → assemble → build →
/// `cargo test`, retrying on invalid/failing output.
pub struct LlmSmokeExecutor {
    ollama: OllamaClient,
    model: String,
}

impl LlmSmokeExecutor {
    pub fn new(ollama: OllamaClient, model: impl Into<String>) -> Self {
        Self {
            ollama,
            model: model.into(),
        }
    }

    /// Run the full loop inside `workdir`: each attempt asks the model for the
    /// lib source, validates + assembles it, writes the crate, and runs
    /// `cargo test`. Returns as soon as a build's tests pass, else after the
    /// retry budget is spent.
    pub async fn run(&self, spec: &SmokeSpec, workdir: &Path) -> Result<SmokeOutcome> {
        let crate_dir = workdir.join(&spec.crate_name);
        let prompt = build_smoke_prompt(&spec.crate_name, &spec.task_desc);
        let max = spec.max_retries.max(1);
        let mut last_detail = String::new();
        let mut generated_any = false;

        for attempt in 1..=max {
            let raw = self
                .ollama
                .generate(&self.model, Some(smoke_system_prompt()), &prompt)
                .await?;
            let lib_src = match FileSet::parse_llm_output(&raw)
                .and_then(|fs| extract_lib_source(&fs, &spec.crate_name))
            {
                Ok(s) => s,
                Err(e) => {
                    last_detail = format!("attempt {attempt}: {e}");
                    continue;
                }
            };
            if let Err(e) = validate_lib_source(&lib_src) {
                last_detail = format!("attempt {attempt}: {e}");
                continue;
            }
            generated_any = true;

            // Fresh crate dir each attempt so a prior failure can't linger.
            let _ = std::fs::remove_dir_all(&crate_dir);
            assemble_crate(&spec.crate_name, &lib_src).apply(workdir)?;

            let test = std::process::Command::new("cargo")
                .arg("test")
                .current_dir(&crate_dir)
                .output()
                .map_err(|e| AutopilotError::Worktree(format!("cargo test: {e}")))?;
            let out = format!(
                "{}{}",
                String::from_utf8_lossy(&test.stdout),
                String::from_utf8_lossy(&test.stderr)
            );
            if WorktreeJob::tests_passed(&out) {
                return Ok(SmokeOutcome {
                    crate_name: spec.crate_name.clone(),
                    model: self.model.clone(),
                    attempts: attempt,
                    generated: true,
                    passed: true,
                    detail: format!("generated + cargo test passed on attempt {attempt}"),
                });
            }
            last_detail = format!(
                "attempt {attempt}: cargo test did not pass\n{}",
                out.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
            );
        }

        Ok(SmokeOutcome {
            crate_name: spec.crate_name.clone(),
            model: self.model.clone(),
            attempts: max,
            generated: generated_any,
            passed: false,
            detail: last_detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_demands_tdd_json_no_stubs() {
        let s = smoke_system_prompt();
        assert!(s.contains("JSON"));
        assert!(s.contains("TDD") || s.to_lowercase().contains("test"));
        assert!(s.to_lowercase().contains("no stub") || s.contains("todo!"));
    }

    #[test]
    fn smoke_prompt_names_crate_and_task_and_target_path() {
        let p = build_smoke_prompt("cave-test-autopilot", "an integer add(a,b) function");
        assert!(p.contains("cave-test-autopilot"));
        assert!(p.contains("add(a,b)"));
        // Pins the exact file the model must return.
        assert!(p.contains("cave-test-autopilot/src/lib.rs"));
    }

    #[test]
    fn extract_lib_source_finds_the_file() {
        let fs = crate::codegen::FileSet {
            files: vec![crate::codegen::GeneratedFile {
                path: "cave-test-autopilot/src/lib.rs".into(),
                content: "pub fn add(a: i64, b: i64) -> i64 { a + b }".into(),
            }],
        };
        let src = extract_lib_source(&fs, "cave-test-autopilot").unwrap();
        assert!(src.contains("fn add"));
    }

    #[test]
    fn extract_lib_source_errors_when_absent() {
        let fs = crate::codegen::FileSet {
            files: vec![crate::codegen::GeneratedFile {
                path: "cave-test-autopilot/src/main.rs".into(),
                content: "fn main(){}".into(),
            }],
        };
        assert!(extract_lib_source(&fs, "cave-test-autopilot").is_err());
    }

    #[test]
    fn validate_lib_accepts_real_fn_with_test() {
        let src = "pub fn add(a: i64, b: i64) -> i64 { a + b }\n\
                   #[cfg(test)]\nmod t { use super::*; #[test] fn w(){ assert_eq!(add(2,3),5); } }";
        assert!(validate_lib_source(src).is_ok());
    }

    #[test]
    fn validate_lib_rejects_stub() {
        let src = "pub fn add(a: i64, b: i64) -> i64 { todo!() }\n#[test] fn w(){}";
        assert!(validate_lib_source(src).is_err());
    }

    #[test]
    fn validate_lib_rejects_missing_test() {
        let src = "pub fn add(a: i64, b: i64) -> i64 { a + b }";
        assert!(validate_lib_source(src).is_err());
    }

    #[test]
    fn assemble_crate_produces_buildable_fileset() {
        let src = "pub fn add(a: i64, b: i64) -> i64 { a + b }\n#[test] fn w(){ assert_eq!(add(1,1),2); }";
        let fs = assemble_crate("cave-test-autopilot", src);
        // gitignore + Cargo.toml + lib.rs
        assert_eq!(fs.files.len(), 3);
        let cargo = fs
            .files
            .iter()
            .find(|f| f.path.ends_with("Cargo.toml"))
            .unwrap();
        assert!(cargo.content.contains("name = \"cave-test-autopilot\""));
        assert!(cargo.content.contains("AGPL-3.0"));
        let lib = fs
            .files
            .iter()
            .find(|f| f.path.ends_with("src/lib.rs"))
            .unwrap();
        // The model's source is licensed even if the model forgot the header.
        assert!(lib.content.contains("SPDX-License-Identifier: AGPL-3.0-or-later"));
        assert!(lib.content.contains("fn add"));
        // Every path is scoped under the crate dir — no escapes.
        assert!(fs.files.iter().all(|f| f.path.starts_with("cave-test-autopilot/")));
    }

    #[test]
    fn assembled_crate_opts_out_of_parent_workspace() {
        // The smoke crate is written under the repo's worktree root, which sits
        // inside the cave-runtime cargo workspace. Without an empty [workspace]
        // table cargo refuses to build it ("believes it's in a workspace when
        // it's not"). The generated manifest must stand alone anywhere.
        let src = "pub fn a()->i32{1}\n#[test] fn t(){assert_eq!(a(),1);}";
        let fs = assemble_crate("cave-test-autopilot", src);
        let cargo = fs
            .files
            .iter()
            .find(|f| f.path.ends_with("Cargo.toml"))
            .unwrap();
        assert!(cargo.content.contains("[workspace]"));
    }

    #[test]
    fn assemble_does_not_double_license_header() {
        let already = "// SPDX-License-Identifier: AGPL-3.0-or-later\npub fn a()->i32{1}\n#[test] fn t(){assert_eq!(a(),1);}";
        let fs = assemble_crate("c", already);
        let lib = fs.files.iter().find(|f| f.path.ends_with("src/lib.rs")).unwrap();
        assert_eq!(lib.content.matches("SPDX-License-Identifier").count(), 1);
    }
}
