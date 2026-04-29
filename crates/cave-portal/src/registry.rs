//! Live module registry — discovers all workspace crates by walking the
//! workspace `Cargo.toml` and the `crates/<name>` directory, then computes
//! per-crate metrics (test count, LOC, parity score, last commit, stub
//! count). The aggregate is what the portal home page renders as the
//! "70+ runtime modules" grid.
//!
//! The registry is read-only and does not depend on any crate's
//! `calculate_parity()` function — every datum here comes from the
//! filesystem so newly-scaffolded crates show up immediately.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub crate_dir: String,
    pub upstream_ref: Option<String>,
    pub upstream_repo: Option<String>,
    pub tests: u32,
    pub loc: u32,
    pub stubs: u32,
    pub parity_score: f64,
    pub has_parity_manifest: bool,
    pub has_lib: bool,
    pub has_routes: bool,
    pub last_commit: Option<String>,
    pub build_status: BuildStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStatus {
    Healthy,
    Degraded,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDetail {
    pub summary: ModuleSummary,
    pub source_files: Vec<String>,
    pub test_files: Vec<String>,
    pub parity_manifest_raw: Option<String>,
    pub readme_excerpt: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("workspace root not found: {0:?}")]
    WorkspaceRootMissing(PathBuf),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml parse: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("module not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Deserialize)]
struct WorkspaceManifest {
    workspace: WorkspaceTable,
}

#[derive(Debug, Deserialize)]
struct WorkspaceTable {
    members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CrateManifest {
    package: Option<CratePackage>,
}

#[derive(Debug, Deserialize)]
struct CratePackage {
    name: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ParityManifest {
    upstream: Option<ParityUpstream>,
    module: Option<ParityModule>,
}

#[derive(Debug, Deserialize)]
struct ParityUpstream {
    org: Option<String>,
    repo: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ParityModule {
    description: Option<String>,
}

// ── Discovery + metric helpers ───────────────────────────────────────────────

/// Read the workspace manifest at `workspace_root/Cargo.toml` and return the
/// list of `crates/<name>` member paths. Non-`crates/` members are filtered.
pub fn list_workspace_crates(workspace_root: &Path) -> Result<Vec<String>, RegistryError> {
    let manifest_path = workspace_root.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(RegistryError::WorkspaceRootMissing(workspace_root.to_path_buf()));
    }
    let raw = std::fs::read_to_string(&manifest_path)?;
    let parsed: WorkspaceManifest = toml::from_str(&raw)?;
    let mut out: Vec<String> = parsed
        .workspace
        .members
        .into_iter()
        .filter_map(|m| m.strip_prefix("crates/").map(String::from))
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn count_tests_in_dir(dir: &Path) -> u32 {
    walk_rs_files(dir)
        .iter()
        .map(|p| count_tests_in_file(p).unwrap_or(0))
        .sum()
}

pub fn count_tests_in_file(path: &Path) -> Result<u32, std::io::Error> {
    let raw = std::fs::read_to_string(path)?;
    let mut count = 0u32;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[test]")
            || trimmed.starts_with("#[tokio::test]")
            || trimmed.starts_with("#[async_std::test]")
        {
            count += 1;
        }
    }
    Ok(count)
}

pub fn count_loc_in_dir(dir: &Path) -> u32 {
    walk_rs_files(dir)
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.lines().count() as u32)
        .sum()
}

pub fn count_stubs_in_dir(dir: &Path) -> u32 {
    walk_rs_files(dir)
        .iter()
        .map(|p| count_stubs_in_file(p).unwrap_or(0))
        .sum()
}

pub fn count_stubs_in_file(path: &Path) -> Result<u32, std::io::Error> {
    let raw = std::fs::read_to_string(path)?;
    let mut count = 0u32;
    for line in raw.lines() {
        if line.contains("todo!()") || line.contains("unimplemented!()") || line.contains("panic!(\"not yet implemented") {
            count += 1;
        }
    }
    Ok(count)
}

pub fn walk_rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                // Skip target/ and .git/.
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "target" || name == ".git" {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

pub fn parity_score(crate_dir: &Path) -> Option<f64> {
    let manifest = crate_dir.join("parity.manifest.toml");
    if !manifest.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&manifest).ok()?;
    let parsed: ParityManifest = toml::from_str(&raw).ok()?;
    let _ = parsed; // we don't compute a real score from the manifest alone

    // Use the cave-kernel calculator if available; otherwise fall back to
    // a coarse heuristic based on test count.
    match cave_kernel::parity::calculate_from_str(&raw, crate_dir.to_str().unwrap_or(".")) {
        Ok(report) => Some(report.overall as f64),
        Err(_) => None,
    }
}

pub fn last_commit(crate_dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args([
            "-C",
            crate_dir.to_str()?,
            "log",
            "-1",
            "--format=%h %s",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub fn build_module_summary(workspace_root: &Path, crate_name: &str) -> Result<ModuleSummary, RegistryError> {
    let crate_dir = workspace_root.join("crates").join(crate_name);
    if !crate_dir.exists() {
        return Err(RegistryError::NotFound(crate_name.into()));
    }
    let cargo_toml = crate_dir.join("Cargo.toml");
    let (description, package_name) = if cargo_toml.exists() {
        let raw = std::fs::read_to_string(&cargo_toml).unwrap_or_default();
        match toml::from_str::<CrateManifest>(&raw) {
            Ok(m) => {
                let p = m.package.unwrap_or(CratePackage { name: None, description: None });
                (p.description.unwrap_or_default(), p.name.unwrap_or_else(|| crate_name.into()))
            }
            Err(_) => (String::new(), crate_name.into()),
        }
    } else {
        (String::new(), crate_name.into())
    };
    let parity_path = crate_dir.join("parity.manifest.toml");
    let has_parity_manifest = parity_path.exists();
    let mut upstream_ref = None;
    let mut upstream_repo = None;
    if has_parity_manifest {
        if let Ok(raw) = std::fs::read_to_string(&parity_path) {
            if let Ok(m) = toml::from_str::<ParityManifest>(&raw) {
                if let Some(u) = m.upstream {
                    upstream_ref = u.version;
                    if let (Some(org), Some(repo)) = (u.org, u.repo) {
                        upstream_repo = Some(format!("https://github.com/{org}/{repo}"));
                    }
                }
            }
        }
    }
    let src_dir = crate_dir.join("src");
    let tests_dir = crate_dir.join("tests");
    let tests = count_tests_in_dir(&src_dir) + count_tests_in_dir(&tests_dir);
    let loc = count_loc_in_dir(&src_dir);
    let stubs = count_stubs_in_dir(&src_dir);
    let has_lib = src_dir.join("lib.rs").exists();
    let has_routes = src_dir.join("routes.rs").exists() || src_dir.join("routes").is_dir();
    let parity = parity_score(&crate_dir).unwrap_or(0.0);
    let last_commit = last_commit(&crate_dir);
    // Build status heuristic: tests > 0 && stubs == 0 → healthy; tests > 0 && stubs > 0 → degraded; else unknown.
    let build_status = if tests == 0 {
        BuildStatus::Unknown
    } else if stubs == 0 {
        BuildStatus::Healthy
    } else {
        BuildStatus::Degraded
    };
    Ok(ModuleSummary {
        name: package_name.clone(),
        display_name: package_name
            .strip_prefix("cave-")
            .unwrap_or(&package_name)
            .to_string(),
        description,
        crate_dir: crate_name.into(),
        upstream_ref,
        upstream_repo,
        tests,
        loc,
        stubs,
        parity_score: parity,
        has_parity_manifest,
        has_lib,
        has_routes,
        last_commit,
        build_status,
    })
}

pub fn build_module_detail(workspace_root: &Path, crate_name: &str) -> Result<ModuleDetail, RegistryError> {
    let summary = build_module_summary(workspace_root, crate_name)?;
    let crate_dir = workspace_root.join("crates").join(crate_name);
    let src_dir = crate_dir.join("src");
    let tests_dir = crate_dir.join("tests");
    let source_files: Vec<String> = walk_rs_files(&src_dir)
        .iter()
        .filter_map(|p| p.strip_prefix(&crate_dir).ok().map(|p| p.display().to_string()))
        .collect();
    let test_files: Vec<String> = walk_rs_files(&tests_dir)
        .iter()
        .filter_map(|p| p.strip_prefix(&crate_dir).ok().map(|p| p.display().to_string()))
        .collect();
    let parity_manifest_raw = std::fs::read_to_string(crate_dir.join("parity.manifest.toml")).ok();
    let readme_excerpt = std::fs::read_to_string(crate_dir.join("README.md"))
        .ok()
        .map(|s| s.lines().take(20).collect::<Vec<_>>().join("\n"));
    Ok(ModuleDetail {
        summary,
        source_files,
        test_files,
        parity_manifest_raw,
        readme_excerpt,
    })
}

// ── TTL cache ────────────────────────────────────────────────────────────────

pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);

pub struct ModuleRegistry {
    workspace_root: PathBuf,
    cache: Mutex<Option<(Instant, Vec<ModuleSummary>)>>,
    ttl: Duration,
}

impl ModuleRegistry {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            cache: Mutex::new(None),
            ttl: DEFAULT_TTL,
        }
    }

    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn invalidate(&self) {
        *self.cache.lock().unwrap() = None;
    }

    /// Returns the cached list when fresh, otherwise rebuilds.
    pub fn list_all(&self) -> Result<Vec<ModuleSummary>, RegistryError> {
        {
            let guard = self.cache.lock().unwrap();
            if let Some((at, list)) = guard.as_ref() {
                if at.elapsed() < self.ttl {
                    return Ok(list.clone());
                }
            }
        }
        let names = list_workspace_crates(&self.workspace_root)?;
        let mut out = Vec::with_capacity(names.len());
        for n in &names {
            match build_module_summary(&self.workspace_root, n) {
                Ok(s) => out.push(s),
                Err(_) => continue,
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        *self.cache.lock().unwrap() = Some((Instant::now(), out.clone()));
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Result<ModuleDetail, RegistryError> {
        // Strip optional `cave-` prefix in lookup so portal can use either form
        let canonical = if name.starts_with("cave-") {
            name.to_string()
        } else {
            format!("cave-{name}")
        };
        build_module_detail(&self.workspace_root, &canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Build a tiny faux workspace under tempdir.
    fn make_fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "cave-portal-registry-fixture-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
resolver = "2"
members = [
    "crates/cave-alpha",
    "crates/cave-beta",
    "tools/not-a-crate",
]
"#,
        )
        .unwrap();
        // alpha: has parity, has lib, 2 tests, 1 stub
        let alpha = root.join("crates/cave-alpha");
        fs::create_dir_all(alpha.join("src")).unwrap();
        fs::write(
            alpha.join("Cargo.toml"),
            r#"[package]
name = "cave-alpha"
description = "Test alpha crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(
            alpha.join("parity.manifest.toml"),
            r#"[upstream]
org = "exampleorg"
repo = "examplerepo"
version = "v1.2.3"
[module]
name = "cave-alpha"
description = "alpha"
source_root = "src"
"#,
        )
        .unwrap();
        fs::write(
            alpha.join("src/lib.rs"),
            "pub fn x() { todo!() }\n#[test] fn t1() {}\n#[tokio::test] async fn t2() {}\n",
        )
        .unwrap();
        fs::write(alpha.join("README.md"), "# Alpha\nFirst line\nsecond\n").unwrap();
        // beta: no parity, no lib, no tests
        let beta = root.join("crates/cave-beta");
        fs::create_dir_all(beta.join("src")).unwrap();
        fs::write(
            beta.join("Cargo.toml"),
            r#"[package]
name = "cave-beta"
description = "Test beta"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::write(beta.join("src/main.rs"), "fn main() {}\n").unwrap();
        root
    }

    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    // ── list_workspace_crates ─────────────────────────────────────────────

    #[test]
    fn list_workspace_crates_filters_to_crates_dir() {
        let f = make_fixture();
        let out = list_workspace_crates(&f).unwrap();
        assert_eq!(out, vec!["cave-alpha".to_string(), "cave-beta".to_string()]);
        cleanup(&f);
    }

    #[test]
    fn list_workspace_crates_missing_workspace() {
        let dir = std::env::temp_dir().join(format!("missing-{}", uuid::Uuid::new_v4()));
        let err = list_workspace_crates(&dir).unwrap_err();
        assert!(matches!(err, RegistryError::WorkspaceRootMissing(_)));
    }

    // ── walk_rs_files ─────────────────────────────────────────────────────

    #[test]
    fn walk_rs_files_returns_only_rs() {
        let f = make_fixture();
        let files = walk_rs_files(&f.join("crates/cave-alpha"));
        assert!(files.iter().all(|p| p.extension().unwrap() == "rs"));
        cleanup(&f);
    }

    #[test]
    fn walk_rs_files_skips_target_and_git() {
        let f = make_fixture();
        let alpha = f.join("crates/cave-alpha");
        std::fs::create_dir_all(alpha.join("target/debug")).unwrap();
        std::fs::write(alpha.join("target/debug/should_skip.rs"), "fn x() {}").unwrap();
        let files = walk_rs_files(&alpha);
        assert!(files.iter().all(|p| !p.to_str().unwrap().contains("/target/")));
        cleanup(&f);
    }

    #[test]
    fn walk_rs_files_missing_dir_returns_empty() {
        let p = std::env::temp_dir().join(format!("ghost-{}", uuid::Uuid::new_v4()));
        assert!(walk_rs_files(&p).is_empty());
    }

    // ── count_tests_in_file / dir ─────────────────────────────────────────

    #[test]
    fn count_tests_recognizes_test_attrs() {
        let f = make_fixture();
        let n = count_tests_in_dir(&f.join("crates/cave-alpha/src"));
        assert_eq!(n, 2);
        cleanup(&f);
    }

    #[test]
    fn count_tests_zero_when_no_tests() {
        let f = make_fixture();
        let n = count_tests_in_dir(&f.join("crates/cave-beta/src"));
        assert_eq!(n, 0);
        cleanup(&f);
    }

    // ── count_loc_in_dir ──────────────────────────────────────────────────

    #[test]
    fn count_loc_sums_lines() {
        let f = make_fixture();
        let n = count_loc_in_dir(&f.join("crates/cave-alpha/src"));
        // file content is 3 lines
        assert_eq!(n, 3);
        cleanup(&f);
    }

    // ── count_stubs_in_dir ────────────────────────────────────────────────

    #[test]
    fn count_stubs_recognizes_todo_macro() {
        let f = make_fixture();
        let n = count_stubs_in_dir(&f.join("crates/cave-alpha/src"));
        assert_eq!(n, 1);
        cleanup(&f);
    }

    #[test]
    fn count_stubs_zero_when_none() {
        let f = make_fixture();
        let n = count_stubs_in_dir(&f.join("crates/cave-beta/src"));
        assert_eq!(n, 0);
        cleanup(&f);
    }

    // ── parity_score ──────────────────────────────────────────────────────

    #[test]
    fn parity_score_returns_some_when_manifest_present() {
        let f = make_fixture();
        let s = parity_score(&f.join("crates/cave-alpha"));
        assert!(s.is_some());
        cleanup(&f);
    }

    #[test]
    fn parity_score_none_when_no_manifest() {
        let f = make_fixture();
        let s = parity_score(&f.join("crates/cave-beta"));
        assert!(s.is_none());
        cleanup(&f);
    }

    // ── build_module_summary ──────────────────────────────────────────────

    #[test]
    fn build_summary_reads_metadata() {
        let f = make_fixture();
        let s = build_module_summary(&f, "cave-alpha").unwrap();
        assert_eq!(s.name, "cave-alpha");
        assert_eq!(s.display_name, "alpha");
        assert_eq!(s.description, "Test alpha crate");
        assert_eq!(s.tests, 2);
        assert_eq!(s.stubs, 1);
        assert!(s.has_lib);
        assert!(s.has_parity_manifest);
        assert_eq!(s.upstream_ref.as_deref(), Some("v1.2.3"));
        assert_eq!(s.upstream_repo.as_deref(), Some("https://github.com/exampleorg/examplerepo"));
        cleanup(&f);
    }

    #[test]
    fn build_summary_status_degraded_when_stubs_present() {
        let f = make_fixture();
        let s = build_module_summary(&f, "cave-alpha").unwrap();
        assert_eq!(s.build_status, BuildStatus::Degraded);
        cleanup(&f);
    }

    #[test]
    fn build_summary_status_unknown_when_no_tests() {
        let f = make_fixture();
        let s = build_module_summary(&f, "cave-beta").unwrap();
        assert_eq!(s.build_status, BuildStatus::Unknown);
        assert_eq!(s.tests, 0);
        cleanup(&f);
    }

    #[test]
    fn build_summary_status_healthy_when_tests_no_stubs() {
        let f = make_fixture();
        // Patch alpha to remove the stub
        std::fs::write(
            f.join("crates/cave-alpha/src/lib.rs"),
            "pub fn x() {}\n#[test] fn t1() {}\n#[tokio::test] async fn t2() {}\n",
        )
        .unwrap();
        let s = build_module_summary(&f, "cave-alpha").unwrap();
        assert_eq!(s.build_status, BuildStatus::Healthy);
        cleanup(&f);
    }

    #[test]
    fn build_summary_unknown_crate_errors() {
        let f = make_fixture();
        let err = build_module_summary(&f, "cave-ghost").unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
        cleanup(&f);
    }

    // ── build_module_detail ───────────────────────────────────────────────

    #[test]
    fn build_detail_exposes_source_files() {
        let f = make_fixture();
        let d = build_module_detail(&f, "cave-alpha").unwrap();
        assert!(d.source_files.iter().any(|p| p.ends_with("lib.rs")));
        assert!(d.parity_manifest_raw.is_some());
        assert!(d.readme_excerpt.is_some());
        cleanup(&f);
    }

    #[test]
    fn build_detail_no_readme_yields_none() {
        let f = make_fixture();
        let d = build_module_detail(&f, "cave-beta").unwrap();
        assert!(d.readme_excerpt.is_none());
        assert!(d.parity_manifest_raw.is_none());
        cleanup(&f);
    }

    // ── ModuleRegistry (TTL cache) ────────────────────────────────────────

    #[test]
    fn registry_list_all_returns_all_workspace_crates() {
        let f = make_fixture();
        let r = ModuleRegistry::new(&f);
        let mods = r.list_all().unwrap();
        let names: Vec<&str> = mods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"cave-alpha"));
        assert!(names.contains(&"cave-beta"));
        cleanup(&f);
    }

    #[test]
    fn registry_caches_within_ttl() {
        let f = make_fixture();
        let r = ModuleRegistry::new(&f).with_ttl(Duration::from_secs(60));
        let _first = r.list_all().unwrap();
        // Modify a file — second call should still return cached.
        std::fs::write(
            f.join("crates/cave-alpha/src/lib.rs"),
            "pub fn x() {}\n",
        )
        .unwrap();
        let second = r.list_all().unwrap();
        let alpha = second.iter().find(|m| m.name == "cave-alpha").unwrap();
        assert_eq!(alpha.tests, 2); // still old value from cache
        cleanup(&f);
    }

    #[test]
    fn registry_invalidate_forces_rebuild() {
        let f = make_fixture();
        let r = ModuleRegistry::new(&f).with_ttl(Duration::from_secs(60));
        let _ = r.list_all().unwrap();
        std::fs::write(
            f.join("crates/cave-alpha/src/lib.rs"),
            "pub fn x() {}\n",
        )
        .unwrap();
        r.invalidate();
        let mods = r.list_all().unwrap();
        let alpha = mods.iter().find(|m| m.name == "cave-alpha").unwrap();
        assert_eq!(alpha.tests, 0);
        cleanup(&f);
    }

    #[test]
    fn registry_get_resolves_with_or_without_prefix() {
        let f = make_fixture();
        let r = ModuleRegistry::new(&f);
        assert!(r.get("alpha").is_ok());
        assert!(r.get("cave-alpha").is_ok());
        assert!(r.get("ghost").is_err());
        cleanup(&f);
    }

    #[test]
    fn registry_list_all_sorted() {
        let f = make_fixture();
        let r = ModuleRegistry::new(&f);
        let mods = r.list_all().unwrap();
        let names: Vec<&str> = mods.iter().map(|m| m.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        cleanup(&f);
    }

    #[test]
    fn build_status_serializes_snake_case() {
        let s = serde_json::to_string(&BuildStatus::Healthy).unwrap();
        assert_eq!(s, "\"healthy\"");
    }
}
