// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Code-generation contract between the LLM tiers and the filesystem.
//!
//! The local coder (L2) and Claude (L3) are asked to emit a strict JSON
//! [`FileSet`] — an array of `{path, content}` objects — optionally wrapped in
//! a ```` ```json ```` fence. [`FileSet::parse_llm_output`] extracts and parses
//! it; [`FileSet::apply`] writes the files into a worktree. This is a real,
//! testable contract, not free-form patch guessing.
//!
//! [`scaffold_cave_test`] produces a deterministic FileSet for the end-to-end
//! mock task ("scaffold cave-test crate"), so the worktree → build → test →
//! commit → merge pipeline can be exercised without depending on a live LLM.

use crate::error::{AutopilotError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One file the model wants written, relative to the crate/worktree root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

/// A set of files produced in one generation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSet {
    pub files: Vec<GeneratedFile>,
}

impl FileSet {
    /// Extract a JSON FileSet from raw LLM output. Tolerates a leading prose
    /// preamble and a ```` ```json … ``` ```` fence by locating the first
    /// balanced top-level JSON object/array.
    pub fn parse_llm_output(raw: &str) -> Result<FileSet> {
        let json = extract_json_block(raw)
            .ok_or_else(|| AutopilotError::Llm("no JSON FileSet found in output".into()))?;
        let fs: FileSet = serde_json::from_str(&json)
            .map_err(|e| AutopilotError::Llm(format!("FileSet parse: {e}")))?;
        if fs.files.is_empty() {
            return Err(AutopilotError::Llm("FileSet has no files".into()));
        }
        // Reject path traversal — files must stay within the worktree.
        for f in &fs.files {
            if f.path.contains("..") || f.path.starts_with('/') {
                return Err(AutopilotError::Llm(format!("unsafe path: {}", f.path)));
            }
        }
        Ok(fs)
    }

    /// Write every file under `root`, creating parent dirs. Returns the written
    /// paths.
    pub fn apply(&self, root: &Path) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for f in &self.files {
            let full = root.join(&f.path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, &f.content)?;
            written.push(full);
        }
        Ok(written)
    }
}

/// Find the first balanced top-level `{...}` or `[...]` block in `raw`.
/// Shared with [`crate::router`], which parses the L1 routing JSON the same way.
pub(crate) fn extract_json_block(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{' || b == b'[')?;
    let open = bytes[start];
    let close = if open == b'{' { b'}' } else { b']' };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(raw[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// System prompt shared by the local coder and Claude: emit a JSON FileSet,
/// strict TDD, no stubs.
pub fn codegen_system_prompt() -> &'static str {
    "You are a Rust engineer working in the Cave Runtime monorepo. \
     Output ONLY a JSON object of the form {\"files\":[{\"path\":\"...\",\"content\":\"...\"}]}. \
     Follow strict TDD: include a failing test first, then the implementation that makes it pass. \
     Never emit todo!() or unimplemented!() — no stubs. Match surrounding code style. \
     License every new file with the AGPL-3.0-or-later SPDX header."
}

/// Build the L2/L3 code-generation prompt for a task.
pub fn build_codegen_prompt(subsystem: &str, crate_dir: &str, upstream: Option<&str>) -> String {
    let up = upstream.unwrap_or("(no single upstream — see parity.manifest.toml)");
    format!(
        "Crate `{subsystem}` at `{crate_dir}` is below parity. Upstream: {up}.\n\
         Port one cohesive, well-scoped upstream surface into Rust with a failing \
         test first, then the implementation. Return a JSON FileSet (paths relative \
         to the repo root)."
    )
}

/// Build the L1 router prompt: analyse a task and decide scope + context size.
pub fn build_router_prompt(subsystem: &str, completion: f64, upstream: Option<&str>) -> String {
    let up = upstream.unwrap_or("unknown");
    format!(
        "Task: advance `{subsystem}` (currently {completion:.2} complete, upstream {up}).\n\
         Pick the single highest-value upstream surface to port next and estimate the \
         context window needed. Reply with one short paragraph."
    )
}

/// Deterministic FileSet for the end-to-end mock: a self-contained `cave-test`
/// crate with a real function and a passing unit test. Standalone (its own
/// Cargo.toml), so it builds/tests in isolation without touching the workspace.
pub fn scaffold_cave_test(crate_name: &str) -> FileSet {
    let cargo = format!(
        "# SPDX-License-Identifier: AGPL-3.0-or-later\n\
         [package]\n\
         name = \"{crate_name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         license = \"AGPL-3.0-or-later\"\n\n\
         [dependencies]\n"
    );
    let lib = "// SPDX-License-Identifier: AGPL-3.0-or-later\n\
         // Copyright 2026 Cave Runtime contributors\n\
         //! Scaffolded by cave-autopilot as an end-to-end pipeline smoke test.\n\n\
         /// Returns the autopilot greeting. Trivial on purpose: this crate exists\n\
         /// to prove the worktree → build → test → commit → merge loop works.\n\
         pub fn greeting() -> &'static str {\n\
         \x20   \"cave-autopilot online\"\n\
         }\n\n\
         #[cfg(test)]\n\
         mod tests {\n\
         \x20   use super::*;\n\n\
         \x20   #[test]\n\
         \x20   fn greeting_is_stable() {\n\
         \x20       assert_eq!(greeting(), \"cave-autopilot online\");\n\
         \x20   }\n\
         }\n";
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
                content: lib.to_string(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_json() {
        let raw = r#"{"files":[{"path":"src/a.rs","content":"fn a(){}"}]}"#;
        let fs = FileSet::parse_llm_output(raw).unwrap();
        assert_eq!(fs.files.len(), 1);
        assert_eq!(fs.files[0].path, "src/a.rs");
    }

    #[test]
    fn parse_fenced_json_with_preamble() {
        let raw = "Here is the change:\n```json\n{\"files\":[{\"path\":\"x.rs\",\"content\":\"// hi\"}]}\n```\nDone.";
        let fs = FileSet::parse_llm_output(raw).unwrap();
        assert_eq!(fs.files[0].path, "x.rs");
    }

    #[test]
    fn parse_handles_braces_inside_strings() {
        let raw = r#"{"files":[{"path":"m.rs","content":"fn m() { let s = \"}\"; }"}]}"#;
        let fs = FileSet::parse_llm_output(raw).unwrap();
        assert!(fs.files[0].content.contains("let s"));
    }

    #[test]
    fn rejects_path_traversal() {
        let raw = r#"{"files":[{"path":"../etc/passwd","content":"x"}]}"#;
        assert!(FileSet::parse_llm_output(raw).is_err());
        let abs = r#"{"files":[{"path":"/tmp/x","content":"x"}]}"#;
        assert!(FileSet::parse_llm_output(abs).is_err());
    }

    #[test]
    fn rejects_empty_fileset() {
        assert!(FileSet::parse_llm_output(r#"{"files":[]}"#).is_err());
    }

    #[test]
    fn apply_writes_files() {
        let dir = tempfile::tempdir().unwrap();
        let fs = scaffold_cave_test("cave-test");
        let written = fs.apply(dir.path()).unwrap();
        assert_eq!(written.len(), 3);
        assert!(dir.path().join("cave-test/.gitignore").exists());
        assert!(dir.path().join("cave-test/Cargo.toml").exists());
        assert!(dir.path().join("cave-test/src/lib.rs").exists());
    }

    #[test]
    fn scaffold_is_stub_free_and_has_test() {
        let fs = scaffold_cave_test("cave-test");
        let lib = &fs
            .files
            .iter()
            .find(|f| f.path.ends_with("src/lib.rs"))
            .unwrap()
            .content;
        assert!(crate::charter::scan_for_stubs(lib).is_empty());
        assert!(lib.contains("#[test]"));
        assert!(lib.contains("SPDX-License-Identifier: AGPL-3.0-or-later"));
    }

    #[test]
    fn prompts_mention_tdd_and_subsystem() {
        assert!(codegen_system_prompt().contains("TDD"));
        assert!(build_codegen_prompt("cave-etcd", "crates/cave-etcd", Some("etcd-io/etcd")).contains("cave-etcd"));
        assert!(build_router_prompt("cave-etcd", 0.5, None).contains("cave-etcd"));
    }
}
