//! Charter v2 gate — verifies an auto-port commit before merge.
//!
//! Gates (every one must pass):
//!
//!   1. `cargo check --workspace --tests` exits 0.
//!   2. `cargo test -p <crate> --include-ignored` exits 0.
//!   3. `parity_ratio` for the affected crate is STRICTLY greater
//!      AFTER than BEFORE (compared via on-disk
//!      `parity.manifest.toml::fill_ratio`).
//!   4. Zero NEW stubs introduced — `unimplemented!()` / `todo!()` /
//!      `#[ignore = "impl pending"]` counts MUST be ≤ baseline.
//!
//! Sequencing: the dispatcher passes a `CharterContext` carrying the
//! BEFORE values (parity_ratio + stub counts) snapshot at the time
//! the auto-port task was dispatched. The gate compares AFTER values
//! (live from the workspace) against those.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use std::process::Command;

#[derive(Debug, Error)]
pub enum GateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cargo invocation failed: {0}")]
    Cargo(String),
    #[error("parity manifest not found: {0}")]
    NoManifest(String),
    #[error("workspace root missing")]
    NoWorkspace,
}

/// The state the dispatcher snapshots BEFORE the task runs. The
/// gate compares the post-task workspace against these numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharterBaseline {
    pub crate_name: String,
    pub commit_sha_before: String,
    pub fill_ratio_before: f64,
    pub workspace_stub_count_before: u64,
}

/// Verification report. `overall_pass = true` only when every gate
/// passes. The dispatcher writes this back to `dispatched.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyResult {
    pub crate_name: String,
    pub commit_sha_after: String,
    pub tests_pass: bool,
    pub cargo_check_pass: bool,
    pub fill_ratio_before: f64,
    pub fill_ratio_after: f64,
    pub parity_ratio_delta: f64,
    pub stub_count_before: u64,
    pub stub_count_after: u64,
    pub no_new_stubs: bool,
    pub no_breaking_change: bool,
    pub overall_pass: bool,
    pub notes: Vec<String>,
}

#[async_trait]
pub trait CharterGate: Send + Sync {
    /// Run every gate against `commit_sha_after` (which the dispatcher
    /// has already checked out) and produce a report.
    async fn verify(
        &self,
        baseline: &CharterBaseline,
        commit_sha_after: &str,
    ) -> Result<VerifyResult, GateError>;
}

/// Production gate — shells out to `cargo`.
#[derive(Debug, Clone)]
pub struct CharterV2Gate {
    pub workspace_root: PathBuf,
    pub cargo_path: PathBuf,
    /// When `true`, skip the cargo invocations — used in tests so
    /// the gate can be unit-tested without a real workspace. The
    /// other gates (ratio + stubs) still run against the manifest.
    pub skip_cargo: bool,
}

impl CharterV2Gate {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            cargo_path: PathBuf::from("cargo"),
            skip_cargo: false,
        }
    }

    /// Read the on-disk `parity.manifest.toml` for `crate_name` and
    /// extract the `[parity] fill_ratio` value. `Result::Ok(None)`
    /// when the manifest exists but no ratio is set (honest 0.0 is
    /// returned as `Ok(Some(0.0))`).
    pub fn read_fill_ratio(&self, crate_name: &str) -> Result<Option<f64>, GateError> {
        let path = self.workspace_root.join("crates").join(crate_name).join("parity.manifest.toml");
        if !path.is_file() {
            return Err(GateError::NoManifest(path.display().to_string()));
        }
        let text = std::fs::read_to_string(&path)?;
        let mut in_section = false;
        for line in text.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("[parity]") {
                in_section = true;
                continue;
            }
            if in_section {
                if trimmed.starts_with('[') && !trimmed.starts_with("[parity]") && !trimmed.starts_with('#') {
                    break;
                }
                let value_part = trimmed.split('#').next().unwrap_or(trimmed);
                if let Some(rest) = value_part
                    .strip_prefix("fill_ratio")
                    .or_else(|| value_part.strip_prefix("ratio"))
                {
                    if let Some(after_eq) = rest.split_once('=') {
                        if let Ok(v) = after_eq.1.trim().parse::<f64>() {
                            return Ok(Some(v));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Count stubs across the workspace. Pure text scan over
    /// `crates/*/src/**/*.rs`; cheap and deterministic. We DO NOT
    /// count occurrences inside `#[cfg(test)]` blocks because tests
    /// legitimately use `todo!()` as a placeholder; the workspace
    /// stub count we care about is production code.
    pub fn count_workspace_stubs(&self) -> Result<u64, GateError> {
        let crates_dir = self.workspace_root.join("crates");
        if !crates_dir.is_dir() {
            return Err(GateError::NoWorkspace);
        }
        let mut total: u64 = 0;
        for entry in walk_rs_files(&crates_dir) {
            let text = std::fs::read_to_string(&entry)?;
            total += count_stubs_in_source(&text);
        }
        Ok(total)
    }

    fn run_cargo(&self, args: &[&str]) -> Result<bool, GateError> {
        let out = Command::new(&self.cargo_path)
            .current_dir(&self.workspace_root)
            .args(args)
            .output()?;
        if !out.status.success() {
            tracing::warn!(
                cargo_args = ?args,
                stderr = %String::from_utf8_lossy(&out.stderr).chars().take(800).collect::<String>(),
                "cargo exited non-zero"
            );
        }
        Ok(out.status.success())
    }
}

/// Pure helper. Public so tests can hit it directly.
pub fn count_stubs_in_source(text: &str) -> u64 {
    let mut count: u64 = 0;
    let mut in_test_module = false;
    let mut brace_depth: i32 = 0;
    for line in text.lines() {
        let trimmed = line.trim();
        // Tracking nesting: when we enter a `#[cfg(test)]` mod
        // block we bump in_test_module true; we leave when the
        // matching close-brace dedents back to module level. This
        // is approximate but matches every cave src layout.
        if trimmed.contains("#[cfg(test)]") {
            // Next non-comment line opens the module.
            // Set a flag; the brace tracker below will close it.
            in_test_module = true;
            brace_depth = 0;
            continue;
        }
        if in_test_module {
            for c in trimmed.chars() {
                if c == '{' {
                    brace_depth += 1;
                }
                if c == '}' {
                    brace_depth -= 1;
                    if brace_depth <= 0 {
                        in_test_module = false;
                        brace_depth = 0;
                        break;
                    }
                }
            }
            continue;
        }
        if trimmed.contains("unimplemented!()") {
            count += 1;
        }
        if trimmed.contains("todo!()") {
            count += 1;
        }
        if trimmed.contains("#[ignore = \"impl pending\"]") {
            count += 1;
        }
    }
    count
}

fn walk_rs_files(root: &std::path::Path) -> Vec<PathBuf> {
    fn inner(p: &std::path::Path, out: &mut Vec<PathBuf>) {
        let Ok(read) = std::fs::read_dir(p) else { return; };
        for ent in read.flatten() {
            let path = ent.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "target" || name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                inner(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                // Skip benches/tests/examples dirs at the top level.
                let parents: Vec<&str> = path
                    .components()
                    .filter_map(|c| c.as_os_str().to_str())
                    .collect();
                if parents.iter().any(|c| matches!(*c, "benches" | "tests" | "examples")) {
                    continue;
                }
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    inner(root, &mut out);
    out
}

#[async_trait]
impl CharterGate for CharterV2Gate {
    async fn verify(
        &self,
        baseline: &CharterBaseline,
        commit_sha_after: &str,
    ) -> Result<VerifyResult, GateError> {
        let mut notes: Vec<String> = Vec::new();

        let cargo_check_pass = if self.skip_cargo {
            notes.push("cargo check skipped (skip_cargo=true)".into());
            true
        } else {
            let pass = self.run_cargo(&["check", "--workspace", "--tests"])?;
            if !pass {
                notes.push("cargo check --workspace --tests FAILED".into());
            }
            pass
        };

        let tests_pass = if self.skip_cargo {
            notes.push("cargo test skipped (skip_cargo=true)".into());
            true
        } else {
            let pass = self.run_cargo(&["test", "-p", &baseline.crate_name, "--include-ignored"])?;
            if !pass {
                notes.push(format!(
                    "cargo test -p {} --include-ignored FAILED",
                    baseline.crate_name
                ));
            }
            pass
        };

        let fill_ratio_after = self
            .read_fill_ratio(&baseline.crate_name)?
            .unwrap_or(0.0);
        let parity_ratio_delta = fill_ratio_after - baseline.fill_ratio_before;
        if parity_ratio_delta <= 0.0 {
            notes.push(format!(
                "fill_ratio did not increase (before={} after={} delta={})",
                baseline.fill_ratio_before, fill_ratio_after, parity_ratio_delta,
            ));
        }

        let stub_count_after = self.count_workspace_stubs()?;
        let no_new_stubs = stub_count_after <= baseline.workspace_stub_count_before;
        if !no_new_stubs {
            notes.push(format!(
                "stub count rose ({} → {})",
                baseline.workspace_stub_count_before, stub_count_after
            ));
        }

        // For "no breaking change" we treat `tests_pass` (which
        // includes the affected crate's full test suite) as
        // sufficient. A wider workspace-test pass would be slower
        // and is left as an optional follow-up.
        let no_breaking_change = tests_pass;

        let overall_pass = cargo_check_pass
            && tests_pass
            && parity_ratio_delta > 0.0
            && no_new_stubs
            && no_breaking_change;

        Ok(VerifyResult {
            crate_name: baseline.crate_name.clone(),
            commit_sha_after: commit_sha_after.to_string(),
            tests_pass,
            cargo_check_pass,
            fill_ratio_before: baseline.fill_ratio_before,
            fill_ratio_after,
            parity_ratio_delta,
            stub_count_before: baseline.workspace_stub_count_before,
            stub_count_after,
            no_new_stubs,
            no_breaking_change,
            overall_pass,
            notes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn workspace_with(crate_name: &str, fill_ratio: f64) -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        let crate_dir = d.path().join("crates").join(crate_name);
        fs::create_dir_all(&crate_dir).unwrap();
        let manifest = format!(
            r#"[parity]
fill_ratio = {fill_ratio}
last_audit = "2026-05-13"
"#,
        );
        fs::write(crate_dir.join("parity.manifest.toml"), manifest).unwrap();
        d
    }

    fn gate(d: &tempfile::TempDir) -> CharterV2Gate {
        CharterV2Gate {
            workspace_root: d.path().to_path_buf(),
            cargo_path: PathBuf::from("cargo"),
            skip_cargo: true,
        }
    }

    fn baseline(crate_name: &str, fill_before: f64, stubs_before: u64) -> CharterBaseline {
        CharterBaseline {
            crate_name: crate_name.into(),
            commit_sha_before: "0".repeat(40),
            fill_ratio_before: fill_before,
            workspace_stub_count_before: stubs_before,
        }
    }

    // ── ratio reader ──────────────────────────────────────────

    #[test]
    fn read_fill_ratio_parses_value() {
        let d = workspace_with("cave-x", 0.75);
        let g = gate(&d);
        let r = g.read_fill_ratio("cave-x").unwrap();
        assert_eq!(r, Some(0.75));
    }

    #[test]
    fn read_fill_ratio_missing_manifest_errors() {
        let d = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(d.path().join("crates")).unwrap();
        let g = CharterV2Gate {
            workspace_root: d.path().to_path_buf(),
            cargo_path: PathBuf::from("cargo"),
            skip_cargo: true,
        };
        assert!(matches!(g.read_fill_ratio("cave-x"), Err(GateError::NoManifest(_))));
    }

    #[test]
    fn read_fill_ratio_returns_none_when_no_ratio_line_present() {
        let d = tempfile::TempDir::new().unwrap();
        let crate_dir = d.path().join("crates").join("cave-x");
        fs::create_dir_all(&crate_dir).unwrap();
        fs::write(
            crate_dir.join("parity.manifest.toml"),
            "[parity]\nlast_audit = \"2026-05-13\"\n",
        )
        .unwrap();
        let g = gate(&d);
        assert_eq!(g.read_fill_ratio("cave-x").unwrap(), None);
    }

    // ── stub counter ──────────────────────────────────────────

    #[test]
    fn count_stubs_in_source_picks_up_all_three_markers() {
        let src = r#"
pub fn x() { unimplemented!() }
pub fn y() { todo!() }
#[ignore = "impl pending"]
fn z() { }
"#;
        assert_eq!(count_stubs_in_source(src), 3);
    }

    #[test]
    fn count_stubs_in_source_excludes_cfg_test_module() {
        let src = r#"
pub fn x() { unimplemented!() }

#[cfg(test)]
mod tests {
    fn helper() { todo!() }
    fn helper2() { unimplemented!() }
}
"#;
        // Only the production `unimplemented!()` should count.
        assert_eq!(count_stubs_in_source(src), 1);
    }

    #[test]
    fn count_stubs_in_source_returns_zero_for_clean_file() {
        let src = "pub fn x() { 42 }\n";
        assert_eq!(count_stubs_in_source(src), 0);
    }

    // ── verify() flow ─────────────────────────────────────────

    #[tokio::test]
    async fn verify_passes_when_ratio_rises_and_no_new_stubs() {
        // Workspace before: fill_ratio=0.7. After we bump the
        // manifest to 0.8 the verify should pass.
        let d = workspace_with("cave-x", 0.8);
        let g = gate(&d);
        let b = baseline("cave-x", 0.7, 0);
        let r = g.verify(&b, "abc1234").await.unwrap();
        assert!(r.overall_pass);
        assert!(r.parity_ratio_delta > 0.0);
        assert_eq!(r.fill_ratio_before, 0.7);
        assert_eq!(r.fill_ratio_after, 0.8);
    }

    #[tokio::test]
    async fn verify_fails_when_ratio_unchanged() {
        let d = workspace_with("cave-x", 0.7);
        let g = gate(&d);
        let b = baseline("cave-x", 0.7, 0);
        let r = g.verify(&b, "sha").await.unwrap();
        assert!(!r.overall_pass);
        assert!(r.notes.iter().any(|n| n.contains("fill_ratio did not increase")));
    }

    #[tokio::test]
    async fn verify_fails_when_ratio_dropped() {
        let d = workspace_with("cave-x", 0.5);
        let g = gate(&d);
        let b = baseline("cave-x", 0.7, 0);
        let r = g.verify(&b, "sha").await.unwrap();
        assert!(!r.overall_pass);
        assert!(r.parity_ratio_delta < 0.0);
    }

    #[tokio::test]
    async fn verify_fails_when_workspace_stubs_grew() {
        // Add a source file with 2 stubs to the workspace.
        let d = workspace_with("cave-x", 0.8);
        let src_dir = d.path().join("crates").join("cave-x").join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("lib.rs"),
            "pub fn a() { unimplemented!() }\npub fn b() { todo!() }",
        )
        .unwrap();
        let g = gate(&d);
        let b = baseline("cave-x", 0.7, 0);
        let r = g.verify(&b, "sha").await.unwrap();
        assert_eq!(r.stub_count_after, 2);
        assert!(!r.no_new_stubs);
        assert!(!r.overall_pass);
        assert!(r.notes.iter().any(|n| n.contains("stub count rose")));
    }

    #[tokio::test]
    async fn verify_returns_report_with_every_field_populated() {
        let d = workspace_with("cave-x", 0.8);
        let g = gate(&d);
        let b = baseline("cave-x", 0.7, 0);
        let r = g.verify(&b, "deadbeef").await.unwrap();
        assert_eq!(r.commit_sha_after, "deadbeef");
        assert_eq!(r.crate_name, "cave-x");
        assert!(r.cargo_check_pass); // skip_cargo=true → defaults to true
        assert!(r.tests_pass);
    }

    #[tokio::test]
    async fn verify_skip_cargo_records_notes_about_skipping() {
        let d = workspace_with("cave-x", 0.8);
        let g = gate(&d);
        let b = baseline("cave-x", 0.7, 0);
        let r = g.verify(&b, "sha").await.unwrap();
        assert!(r.notes.iter().any(|n| n.contains("cargo check skipped")));
        assert!(r.notes.iter().any(|n| n.contains("cargo test skipped")));
    }

    #[test]
    fn count_workspace_stubs_walks_every_crate_src_file() {
        let d = workspace_with("cave-x", 0.8);
        let src = d.path().join("crates").join("cave-x").join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.rs"), "fn x() { unimplemented!() }").unwrap();
        fs::write(src.join("b.rs"), "fn y() { todo!() }").unwrap();
        let g = gate(&d);
        assert_eq!(g.count_workspace_stubs().unwrap(), 2);
    }

    #[test]
    fn count_workspace_stubs_ignores_tests_dir() {
        let d = workspace_with("cave-x", 0.8);
        let crate_dir = d.path().join("crates").join("cave-x");
        fs::create_dir_all(crate_dir.join("tests")).unwrap();
        fs::write(crate_dir.join("tests").join("t.rs"), "fn x() { todo!() }").unwrap();
        let g = gate(&d);
        // Tests directory excluded → 0 stubs.
        assert_eq!(g.count_workspace_stubs().unwrap(), 0);
    }
}
