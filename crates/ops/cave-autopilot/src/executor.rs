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
    fn assemble_does_not_double_license_header() {
        let already = "// SPDX-License-Identifier: AGPL-3.0-or-later\npub fn a()->i32{1}\n#[test] fn t(){assert_eq!(a(),1);}";
        let fs = assemble_crate("c", already);
        let lib = fs.files.iter().find(|f| f.path.ends_with("src/lib.rs")).unwrap();
        assert_eq!(lib.content.matches("SPDX-License-Identifier").count(), 1);
    }
}
