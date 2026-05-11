//! `/admin/compliance` view — Charter compliance + 4-track audit dashboard.
//!
//! Replaces the manual "audit each crate by hand" loop with a live
//! matrix the maintainer can refresh. Data is computed from the
//! filesystem at request time:
//!
//! * Backend signal      — `crates/<c>/src/**/*.rs` line count + `#[test]`
//!                         and `#[ignore]` and `unimplemented!()` tallies
//! * Portal signal       — `crates/cave-portal/src/admin/<short>.rs` exists
//! * cavectl signal      — `crates/cave-cli/src/main.rs` mentions the crate
//! * Observability sig.  — `observability/{alerts,dashboards}/<c>.{yml,json}`
//! * Upstream version    — `crates/<c>/parity.manifest.toml` `[upstream]`
//!
//! The 4-track score is the percentage of the four tracks that are present
//! (backend always counts as present here — every member crate has src).
//!
//! Scope notes (not in this view yet — deliberately deferred):
//! * No background refresh task. Each request rescans the filesystem.
//! * No JSON snapshot endpoint in `cave-portal-api`.
//! * No drill-down per-crate detail page.
//! These show up in the doc-comment so a future commit can fill them in.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::types::Cite;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ComplianceViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("workspace root {0:?} does not exist")]
    BadRoot(PathBuf),
}

/// Charter golden rule quote shown at the top of the page.
pub const GOLDEN_RULE: &str = "Cave Runtime upstream-mirror crates follow the \
golden rule: line-by-line TDD reimpl in Rust, no stubs, no fake tests, no \
hidden compile-only gates. The 4-track minimum (Backend + Portal + cavectl + \
Observability) is a contract, not an aspiration.";

/// Per-crate compliance snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateCompliance {
    pub name: String,
    pub upstream_version: Option<String>,
    pub upstream_org_repo: Option<String>,
    pub backend_loc: u64,
    pub backend_test_count: u32,
    pub ignored_test_count: u32,
    pub unimplemented_count: u32,
    pub todo_count: u32,
    pub portal_admin_present: bool,
    pub cavectl_subcommand_present: bool,
    pub obs_alerts_present: bool,
    pub obs_dashboard_present: bool,
    pub four_track_score: u8,
    /// Infrastructure-only crates (CLI tool, shared primitives, etc.) are
    /// not expected to ship a Portal admin page or cavectl subcommand;
    /// they are excluded from the aggregate score and grade.
    pub infra_only: bool,
}

/// Fallback list of crates that are infrastructure tooling, shared
/// primitives, or runtime support — not Tier-1 upstream-mirror modules.
/// The source of truth is each crate's `parity.manifest.toml` under
/// `[parity] infra_only = true`. This list is consulted only when a
/// manifest is missing or doesn't carry the field, so newly-added infra
/// crates can opt in declaratively without touching this file.
const INFRA_ONLY_FALLBACK: &[&str] = &[
    "cave-cli",
    "cave-core",
    "cave-changelog",
    "cave-types",
    "cave-utils",
    "cave-cost-alloc",
    "cave-kernel",
    "cave-ebpf-common",
    "cave-runtime",
    "cave-portal",
    "cave-portal-api",
    "cave-portal-web",
    "cave-desktop",
    "cave-scaffold",
    "cave-docs",
    "cave-docs-site",
    "cave-runbook",
    "cave-lint",
    "cave-pki",
    "cave-db",
    "cave-acme",
    "cave-techdocs",
    "cave-registry",
    "cave-tracing",
    "cave-sign",
    "cave-pii",
    "cave-flags",
    "cave-status",
    "cave-profiler",
];

/// Fallback predicate used when no manifest declares `[parity] infra_only`.
pub fn is_infra_only_fallback(name: &str) -> bool {
    INFRA_ONLY_FALLBACK.contains(&name)
}

/// Aggregated compliance state for the whole workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceSnapshot {
    pub crates: Vec<CrateCompliance>,
}

impl ComplianceSnapshot {
    /// Aggregate score over tier-1 (non-infra) crates only. Infra crates
    /// are excluded — they don't ship Portal/cavectl/obs by contract.
    pub fn aggregate_score(&self) -> u8 {
        let scored: Vec<&CrateCompliance> = self.crates.iter().filter(|c| !c.infra_only).collect();
        if scored.is_empty() {
            return 0;
        }
        let total: u32 = scored.iter().map(|c| u32::from(c.four_track_score)).sum();
        ((total / scored.len() as u32).min(100)) as u8
    }

    /// Stub indicators across all crates (infra included — fake tests
    /// don't get a pass just because a crate is infra).
    pub fn total_stub_count(&self) -> u32 {
        self.crates
            .iter()
            .map(|c| c.unimplemented_count + c.todo_count + c.ignored_test_count)
            .sum()
    }

    /// Number of crates that contribute to the aggregate (non-infra).
    pub fn tier1_count(&self) -> usize {
        self.crates.iter().filter(|c| !c.infra_only).count()
    }

    /// Number of infra-only crates (shown separately on the dashboard).
    pub fn infra_count(&self) -> usize {
        self.crates.iter().filter(|c| c.infra_only).count()
    }

    pub fn grade(&self) -> char {
        compliance_grade_letter(self.aggregate_score())
    }
}

/// Map a 0-100 score to an A-F letter grade.
pub fn compliance_grade_letter(score: u8) -> char {
    match score {
        90..=u8::MAX => 'A',
        80..=89 => 'B',
        70..=79 => 'C',
        60..=69 => 'D',
        _ => 'F',
    }
}

/// Compute the 4-track score (0-100) from the four boolean signals.
/// Backend is implicit (every workspace member has src); the score is
/// the % of (Portal, cavectl, Obs alerts, Obs dashboard) that are
/// present, plus a baseline 25 for backend itself.
pub fn compute_four_track_score(
    portal: bool,
    cavectl: bool,
    obs_alerts: bool,
    obs_dashboard: bool,
) -> u8 {
    let mut score: u8 = 25; // backend baseline
    if portal {
        score += 25;
    }
    if cavectl {
        score += 25;
    }
    if obs_alerts && obs_dashboard {
        score += 25;
    } else if obs_alerts || obs_dashboard {
        score += 12;
    }
    score
}

/// Parsed view of the bits of `parity.manifest.toml` the dashboard cares about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParityManifest {
    pub upstream_version: Option<String>,
    pub upstream_org_repo: Option<String>,
    /// `[parity] infra_only = true` — opt-in flag that the crate is
    /// infrastructure tooling and exempt from the 4-track contract.
    /// `None` means the field was absent (caller should fall back).
    pub infra_only: Option<bool>,
}

/// Parse `[upstream]` and `[parity]` tables from a parity manifest.
/// Tolerant of missing or malformed manifests — unknown sections are skipped.
pub fn parse_parity_manifest_full(content: &str) -> ParityManifest {
    let mut section: &str = "";
    let mut org: Option<String> = None;
    let mut repo: Option<String> = None;
    let mut version: Option<String> = None;
    let mut infra_only: Option<bool> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = match rest {
                "upstream" => "upstream",
                "parity" => "parity",
                _ => "",
            };
            continue;
        }
        match section {
            "upstream" => {
                if let Some(rest) = trimmed.strip_prefix("version") {
                    version = extract_string_value(rest);
                } else if let Some(rest) = trimmed.strip_prefix("org") {
                    org = extract_string_value(rest);
                } else if let Some(rest) = trimmed.strip_prefix("repo") {
                    repo = extract_string_value(rest);
                }
            }
            "parity" => {
                if let Some(rest) = trimmed.strip_prefix("infra_only") {
                    infra_only = extract_bool_value(rest);
                }
            }
            _ => {}
        }
    }
    let upstream_org_repo = match (org, repo) {
        (Some(o), Some(r)) => Some(format!("{o}/{r}")),
        _ => None,
    };
    ParityManifest {
        upstream_version: version,
        upstream_org_repo,
        infra_only,
    }
}

/// Legacy shim: returns the `(version, org/repo)` pair the older callers
/// were built around. Prefer [`parse_parity_manifest_full`] for new code.
pub fn parse_parity_manifest(content: &str) -> (Option<String>, Option<String>) {
    let m = parse_parity_manifest_full(content);
    (m.upstream_version, m.upstream_org_repo)
}

fn extract_string_value(rest: &str) -> Option<String> {
    let after_eq = rest.split_once('=')?.1.trim();
    let unquoted = after_eq.trim_matches('"');
    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}

fn extract_bool_value(rest: &str) -> Option<bool> {
    let after_eq = rest.split_once('=')?.1.trim();
    match after_eq {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Walk a crate's `src/` tree and tally LOC + test markers + stub markers.
fn scan_backend(src_root: &Path) -> (u64, u32, u32, u32, u32) {
    let mut loc: u64 = 0;
    let mut tests: u32 = 0;
    let mut ignored: u32 = 0;
    let mut unimpl: u32 = 0;
    let mut todos: u32 = 0;
    walk_rs_files(src_root, &mut |content| {
        loc += content.lines().count() as u64;
        for line in content.lines() {
            let l = line.trim_start();
            if l.starts_with("#[test]") || l.starts_with("#[tokio::test]") {
                tests += 1;
            }
            if l.starts_with("#[ignore") {
                ignored += 1;
            }
            // We count textual occurrences. Strict counting would parse
            // the AST; for a dashboard signal this is good enough.
            unimpl += line.matches("unimplemented!").count() as u32;
            unimpl += line.matches("todo!()").count() as u32;
            todos += line.matches("// TODO").count() as u32;
        }
    });
    (loc, tests, ignored, unimpl, todos)
}

fn walk_rs_files(root: &Path, visit: &mut impl FnMut(&str)) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files(&path, visit);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(content) = fs::read_to_string(&path) {
                visit(&content);
            }
        }
    }
}

fn walk_rs_files_with_path(root: &Path, base: &Path, visit: &mut impl FnMut(&str, &Path)) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_rs_files_with_path(&path, base, visit);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(content) = fs::read_to_string(&path) {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                visit(&content, rel);
            }
        }
    }
}

/// One ignored test surfaced by the drill-down view: the source file
/// (workspace-relative), the line number of the `#[ignore]` attribute,
/// and the test function name (best-effort).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredTest {
    pub file: String,
    pub line: u32,
    pub name: String,
}

/// Scan a crate's `src/` tree for `#[ignore]` annotations. Returns the
/// file (workspace-relative), line, and the next visible `fn name` so
/// the drill-down can render a clickable list.
pub fn scan_ignored_tests(workspace_root: &Path, crate_name: &str) -> Vec<IgnoredTest> {
    let src_root = workspace_root.join("crates").join(crate_name).join("src");
    let mut out = Vec::new();
    walk_rs_files_with_path(&src_root, workspace_root, &mut |content, rel| {
        let lines: Vec<&str> = content.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("#[ignore") {
                // Look ahead up to 5 lines for `fn <name>`.
                let mut fn_name = String::from("<unknown>");
                for j in (idx + 1)..lines.len().min(idx + 6) {
                    if let Some(after) = lines[j].trim_start().strip_prefix("fn ") {
                        let name: String = after
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        if !name.is_empty() {
                            fn_name = name;
                            break;
                        }
                    } else if let Some(after) = lines[j].trim_start().strip_prefix("async fn ") {
                        let name: String = after
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        if !name.is_empty() {
                            fn_name = name;
                            break;
                        }
                    }
                }
                out.push(IgnoredTest {
                    file: rel.display().to_string(),
                    line: (idx as u32) + 1,
                    name: fn_name,
                });
            }
        }
    });
    out
}

/// One commit touching a crate, as surfaced by `git log -- crates/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRow {
    pub sha: String,
    pub subject: String,
}

/// Read the last `limit` commits that touched this crate's directory.
/// Uses a plain `git log` subprocess — sufficient for an admin dashboard
/// and avoids pulling in `git2`/`gix` just for this view. Returns an
/// empty vec on git failure (workspace not a repo, git not on PATH, etc.).
pub fn recent_commits_for_crate(
    workspace_root: &Path,
    crate_name: &str,
    limit: u32,
) -> Vec<CommitRow> {
    use std::process::Command;
    let path_spec = format!("crates/{crate_name}");
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("log")
        .arg(format!("-n{limit}"))
        .arg("--pretty=format:%h %s")
        .arg("--")
        .arg(&path_spec)
        .output();
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&stdout);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let sha = parts.next()?.to_string();
            let subject = parts.next().unwrap_or("").to_string();
            if sha.is_empty() {
                None
            } else {
                Some(CommitRow { sha, subject })
            }
        })
        .collect()
}

/// Detail-page bundle: the per-crate row from the dashboard plus the
/// extra data (ignored test list, manifest content, recent commits)
/// that the drill-down surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateDetail {
    pub compliance: CrateCompliance,
    pub ignored_tests: Vec<IgnoredTest>,
    pub parity_manifest_raw: Option<String>,
    pub recent_commits: Vec<CommitRow>,
}

/// Gather everything needed to render `/admin/compliance/<crate>`.
pub fn build_crate_detail(
    workspace_root: &Path,
    crate_name: &str,
) -> Result<CrateDetail, ComplianceViewError> {
    let compliance = analyse_crate(workspace_root, crate_name)?;
    let ignored_tests = scan_ignored_tests(workspace_root, crate_name);
    let manifest_path = workspace_root
        .join("crates")
        .join(crate_name)
        .join("parity.manifest.toml");
    let parity_manifest_raw = fs::read_to_string(&manifest_path).ok();
    let recent_commits = recent_commits_for_crate(workspace_root, crate_name, 10);
    Ok(CrateDetail {
        compliance,
        ignored_tests,
        parity_manifest_raw,
        recent_commits,
    })
}

/// Compute compliance for one crate by walking its directory.
pub fn analyse_crate(
    workspace_root: &Path,
    crate_name: &str,
) -> Result<CrateCompliance, ComplianceViewError> {
    let crate_root = workspace_root.join("crates").join(crate_name);
    if !crate_root.exists() {
        return Err(ComplianceViewError::BadRoot(crate_root));
    }
    let src_root = crate_root.join("src");
    let (loc, tests, ignored, unimpl, todos) = scan_backend(&src_root);
    let manifest_path = crate_root.join("parity.manifest.toml");
    let manifest = if manifest_path.exists() {
        parse_parity_manifest_full(&fs::read_to_string(&manifest_path).unwrap_or_default())
    } else {
        ParityManifest::default()
    };
    let upstream_version = manifest.upstream_version.clone();
    let upstream_org_repo = manifest.upstream_org_repo.clone();
    let infra_only = manifest
        .infra_only
        .unwrap_or_else(|| is_infra_only_fallback(crate_name));
    let short = crate_name.strip_prefix("cave-").unwrap_or(crate_name);
    // Portal admin module name uses snake_case (kebab → underscore).
    let admin_module = short.replace('-', "_");
    let portal_admin_present = workspace_root
        .join("crates/cave-portal/src/admin")
        .join(format!("{admin_module}.rs"))
        .exists()
        || workspace_root
            .join("crates/cave-portal/src/admin")
            .join(format!("{short}.rs"))
            .exists();
    let cavectl_path = workspace_root.join("crates/cave-cli/src/main.rs");
    let cavectl_subcommand_present = match fs::read_to_string(&cavectl_path) {
        Ok(c) => c.contains(&format!("/api/{crate_name}/")) || c.contains(&format!("/api/{short}/")),
        Err(_) => false,
    };
    let obs_alerts_present = workspace_root
        .join("observability/alerts")
        .join(format!("{crate_name}.yml"))
        .exists();
    let obs_dashboard_present = workspace_root
        .join("observability/dashboards")
        .join(format!("{crate_name}.json"))
        .exists();
    let four_track_score = compute_four_track_score(
        portal_admin_present,
        cavectl_subcommand_present,
        obs_alerts_present,
        obs_dashboard_present,
    );
    Ok(CrateCompliance {
        name: crate_name.to_string(),
        upstream_version,
        upstream_org_repo,
        backend_loc: loc,
        backend_test_count: tests,
        ignored_test_count: ignored,
        unimplemented_count: unimpl,
        todo_count: todos,
        portal_admin_present,
        cavectl_subcommand_present,
        obs_alerts_present,
        obs_dashboard_present,
        four_track_score,
        infra_only,
    })
}

/// Build a compliance snapshot for the given crate name list.
pub fn build_snapshot(
    workspace_root: &Path,
    crate_names: &[&str],
) -> Result<ComplianceSnapshot, ComplianceViewError> {
    let mut crates: Vec<CrateCompliance> = crate_names
        .iter()
        .filter_map(|n| analyse_crate(workspace_root, n).ok())
        .collect();
    // Worst-compliance first — drives the maintainer's eye.
    crates.sort_by(|a, b| match a.four_track_score.cmp(&b.four_track_score) {
        Ordering::Equal => a.name.cmp(&b.name),
        ord => ord,
    });
    Ok(ComplianceSnapshot { crates })
}

/// Discover every directory under `crates/` that has a Cargo.toml — used by
/// the live handler so the matrix grows automatically as new crates are added.
pub fn discover_crate_names(workspace_root: &Path) -> Vec<String> {
    let crates_dir = workspace_root.join("crates");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("Cargo.toml").is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out.sort();
    out
}

/// Locate the workspace root by walking up from `CARGO_MANIFEST_DIR` (the
/// crate this code lives in) until we find a directory that contains a
/// `crates/` subdirectory and a top-level `Cargo.toml`. Override at runtime
/// with `CAVE_WORKSPACE_ROOT`.
pub fn workspace_root() -> PathBuf {
    if let Ok(env) = std::env::var("CAVE_WORKSPACE_ROOT") {
        return PathBuf::from(env);
    }
    let mut here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if here.join("crates").is_dir() && here.join("Cargo.toml").is_file() {
            return here;
        }
        if !here.pop() {
            return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        }
    }
}

/// One-shot snapshot for the live admin handler. Always walks the
/// filesystem — prefer [`cached_snapshot_or_refresh`] in production so
/// concurrent requests share a 5-minute cache.
pub fn live_snapshot() -> ComplianceSnapshot {
    let root = workspace_root();
    let names = discover_crate_names(&root);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    build_snapshot(&root, &refs).unwrap_or(ComplianceSnapshot { crates: vec![] })
}

/// Wraps a [`ComplianceSnapshot`] with the wall-clock instant it was
/// materialised. The handler uses the timestamp to decide whether the
/// cached copy is still fresh or needs a re-walk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedSnapshot {
    pub snapshot: ComplianceSnapshot,
    pub cached_at: DateTime<Utc>,
}

impl CachedSnapshot {
    pub fn new(snapshot: ComplianceSnapshot, cached_at: DateTime<Utc>) -> Self {
        Self { snapshot, cached_at }
    }

    /// Returns `true` when `now - cached_at > max_age`.
    pub fn is_stale(&self, now: DateTime<Utc>, max_age: Duration) -> bool {
        match now.signed_duration_since(self.cached_at).to_std() {
            Ok(elapsed) => elapsed > max_age,
            // Future timestamps (clock skew) are treated as fresh — the
            // cache is only invalidated by elapsed wall-clock time forward.
            Err(_) => false,
        }
    }
}

/// Process-wide cache shared by HTML handler, JSON endpoint, and the
/// background refresher. `OnceLock` so we initialise lazily without a
/// global init step.
fn cache_cell() -> &'static Mutex<Option<CachedSnapshot>> {
    static CELL: OnceLock<Mutex<Option<CachedSnapshot>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Default cache freshness window — the JSON endpoint advertises the
/// same value via Cache-Control, and the background refresher uses it
/// as its interval.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

/// Return the cached snapshot when fresh, otherwise rebuild + cache.
/// Concurrent callers see a consistent value: the mutex is held only
/// across the walk if it actually runs, so the next caller benefits.
pub fn cached_snapshot_or_refresh() -> ComplianceSnapshot {
    cached_snapshot_or_refresh_at(Utc::now(), DEFAULT_CACHE_TTL)
}

/// Testable variant: callers inject `now` + `max_age`.
pub fn cached_snapshot_or_refresh_at(
    now: DateTime<Utc>,
    max_age: Duration,
) -> ComplianceSnapshot {
    let cell = cache_cell();
    {
        let guard = cell.lock().expect("compliance cache poisoned");
        if let Some(entry) = guard.as_ref() {
            if !entry.is_stale(now, max_age) {
                return entry.snapshot.clone();
            }
        }
    }
    // Cache miss or stale — walk the filesystem outside the lock so
    // concurrent readers aren't blocked by the (slow) walk.
    let fresh = live_snapshot();
    let mut guard = cell.lock().expect("compliance cache poisoned");
    *guard = Some(CachedSnapshot::new(fresh.clone(), now));
    fresh
}

/// Force-invalidate the cache. Returns the previous timestamp, if any,
/// so callers can log the refresh delta.
pub fn invalidate_cache() -> Option<DateTime<Utc>> {
    let mut guard = cache_cell().lock().expect("compliance cache poisoned");
    let prev = guard.as_ref().map(|c| c.cached_at);
    *guard = None;
    prev
}

/// Force a refresh now, regardless of cache state. Returns the new snapshot.
pub fn force_refresh() -> CachedSnapshot {
    force_refresh_at(Utc::now())
}

/// Testable variant of [`force_refresh`].
pub fn force_refresh_at(now: DateTime<Utc>) -> CachedSnapshot {
    let fresh = live_snapshot();
    let entry = CachedSnapshot::new(fresh, now);
    let mut guard = cache_cell().lock().expect("compliance cache poisoned");
    *guard = Some(entry.clone());
    entry
}

/// Render the live `/admin/compliance` page using the cached snapshot.
pub fn render_cached(ctx: &RequestCtx) -> Result<String, ComplianceViewError> {
    let snap = cached_snapshot_or_refresh();
    render(&snap, ctx)
}

/// Render the manual-refresh acknowledgement page. Authorised by
/// `Permission::AdminComplianceRefresh` — a strictly stronger right
/// than the view permission used by `render`.
pub fn handle_refresh(ctx: &RequestCtx) -> Result<String, ComplianceViewError> {
    ctx.authorise(Permission::AdminComplianceRefresh)?;
    let entry = force_refresh();
    let body = format!(
        r#"<section class="p-4 bg-green-100 rounded mb-4">
  <p>Compliance cache refreshed at <strong>{ts}</strong> — {n} crates rescanned.</p>
  <p><a class="text-blue-700 underline" href="/admin/compliance">Back to dashboard</a></p>
</section>"#,
        ts = entry.cached_at.to_rfc3339(),
        n = entry.snapshot.crates.len(),
    );
    Ok(page_shell(
        &format!("compliance refresh · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// Spawn the background refresh loop. Runs in a detached tokio task that
/// ticks every `interval`, calling [`force_refresh`] on each tick. The
/// `JoinHandle` is returned so tests can drive the cancellation token to
/// shut it down cleanly; production callers can drop the handle.
pub fn spawn_background_refresh(interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // Skip the immediate first tick so we don't double-walk on startup
        // when `cached_snapshot_or_refresh` already populated the cache.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let _ = force_refresh();
        }
    })
}

fn cell_color(value: u8) -> &'static str {
    match value {
        80..=u8::MAX => "bg-green-100 text-green-900",
        50..=79 => "bg-yellow-100 text-yellow-900",
        _ => "bg-red-100 text-red-900",
    }
}

fn check(b: bool) -> &'static str {
    if b {
        "✓"
    } else {
        "—"
    }
}

pub fn render(
    snapshot: &ComplianceSnapshot,
    ctx: &RequestCtx,
) -> Result<String, ComplianceViewError> {
    ctx.authorise(Permission::AdminComplianceView)?;
    let total = snapshot.crates.len();
    let tier1 = snapshot.tier1_count();
    let infra = snapshot.infra_count();
    let avg = snapshot.aggregate_score();
    let stubs = snapshot.total_stub_count();
    let grade = snapshot.grade();
    let avg_color = cell_color(avg);

    let rows: Vec<Vec<String>> = snapshot
        .crates
        .iter()
        .map(|c| {
            let score_html = if c.infra_only {
                format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-200 text-gray-600" title="infra-only">infra</span>"#,
                )
            } else {
                format!(
                    r#"<span class="px-2 py-1 rounded {color}">{score}</span>"#,
                    color = cell_color(c.four_track_score),
                    score = c.four_track_score,
                )
            };
            vec![
                format!(
                    r#"<a class="text-blue-700 underline" href="/admin/compliance/{name}?tenant_id={tenant}">{label}</a>"#,
                    name = escape(&c.name),
                    tenant = escape(ctx.tenant.as_str()),
                    label = escape(&c.name),
                ),
                c.upstream_version.clone().unwrap_or_else(|| "—".into()),
                c.backend_loc.to_string(),
                c.backend_test_count.to_string(),
                c.ignored_test_count.to_string(),
                c.unimplemented_count.to_string(),
                check(c.portal_admin_present).into(),
                check(c.cavectl_subcommand_present).into(),
                check(c.obs_alerts_present).into(),
                check(c.obs_dashboard_present).into(),
                score_html,
            ]
        })
        .collect();

    let body = format!(
        r#"<section class="mb-6 p-4 bg-gray-100 rounded">
  <p class="italic text-gray-700">{quote}</p>
</section>
<section class="grid grid-cols-5 gap-4 mb-6">
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">TIER-1 CRATES</div><div class="text-3xl font-bold">{tier1}</div><div class="text-xs text-gray-400">+ {infra} infra</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">AVG 4-TRACK</div><div class="text-3xl font-bold {avg_color} px-2 rounded">{avg}</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL STUBS</div><div class="text-3xl font-bold">{stubs}</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">GRADE</div><div class="text-3xl font-bold">{grade}</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL</div><div class="text-3xl font-bold">{total}</div></div>
</section>
<section><h2 class="text-lg font-semibold mb-2">Per-crate matrix</h2>{tbl}</section>"#,
        quote = escape(GOLDEN_RULE),
        total = total,
        tier1 = tier1,
        infra = infra,
        avg = avg,
        avg_color = avg_color,
        stubs = stubs,
        grade = grade,
        tbl = table(
            &[
                "crate", "upstream", "loc", "tests", "ignored", "unimpl!",
                "portal", "cavectl", "alerts", "dash", "score",
            ],
            &rows,
        ),
    );
    Ok(page_shell(
        &format!("compliance · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/tech-insights/src/components/Scorecards/ScorecardsPage.tsx",
    "ScorecardsPage",
);

/// Render the drill-down `/admin/compliance/<crate>` page.
pub fn render_detail(
    detail: &CrateDetail,
    ctx: &RequestCtx,
) -> Result<String, ComplianceViewError> {
    ctx.authorise(Permission::AdminComplianceView)?;
    let c = &detail.compliance;
    let upstream = c
        .upstream_org_repo
        .as_deref()
        .map(|or| {
            let version = c.upstream_version.as_deref().unwrap_or("—");
            format!("{or} @ {version}")
        })
        .unwrap_or_else(|| "—".into());

    let track_html = format!(
        r#"<ul class="space-y-1">
  <li>Backend (LOC {loc}, tests {tests}): {backend}</li>
  <li>Portal admin page: {portal}</li>
  <li>cavectl subcommand: {cavectl}</li>
  <li>Observability alerts: {alerts}</li>
  <li>Observability dashboard: {dash}</li>
</ul>"#,
        loc = c.backend_loc,
        tests = c.backend_test_count,
        backend = check(true),
        portal = check(c.portal_admin_present),
        cavectl = check(c.cavectl_subcommand_present),
        alerts = check(c.obs_alerts_present),
        dash = check(c.obs_dashboard_present),
    );

    let ignored_html = if detail.ignored_tests.is_empty() {
        r#"<p class="text-gray-500">No <code>#[ignore]</code> tests — Charter compliant.</p>"#
            .to_string()
    } else {
        let rows: Vec<String> = detail
            .ignored_tests
            .iter()
            .map(|t| {
                format!(
                    "<tr><td class=\"px-2 py-1\"><code>{name}</code></td>\
                     <td class=\"px-2 py-1\">{file}</td>\
                     <td class=\"px-2 py-1 text-right\">{line}</td></tr>",
                    name = escape(&t.name),
                    file = escape(&t.file),
                    line = t.line,
                )
            })
            .collect();
        format!(
            r#"<table class="min-w-full text-sm">
  <thead><tr class="border-b"><th class="text-left px-2">test</th><th class="text-left px-2">file</th><th class="text-right px-2">line</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#,
            rows = rows.join("")
        )
    };

    let manifest_html = match detail.parity_manifest_raw.as_deref() {
        Some(raw) => format!(
            "<pre class=\"text-xs p-3 bg-gray-50 border rounded overflow-x-auto\">{}</pre>",
            escape(raw)
        ),
        None => r#"<p class="text-gray-500">No parity.manifest.toml — first-party crate or scaffold missing.</p>"#.to_string(),
    };

    let commits_html = if detail.recent_commits.is_empty() {
        r#"<p class="text-gray-500">No commits touch this crate yet.</p>"#.to_string()
    } else {
        let rows: Vec<String> = detail
            .recent_commits
            .iter()
            .map(|cm| {
                format!(
                    "<tr><td class=\"px-2 py-1 font-mono\">{sha}</td>\
                     <td class=\"px-2 py-1\">{subj}</td></tr>",
                    sha = escape(&cm.sha),
                    subj = escape(&cm.subject),
                )
            })
            .collect();
        format!(
            r#"<table class="min-w-full text-sm">
  <thead><tr class="border-b"><th class="text-left px-2">sha</th><th class="text-left px-2">subject</th></tr></thead>
  <tbody>{rows}</tbody>
</table>"#,
            rows = rows.join("")
        )
    };

    let infra_badge = if c.infra_only {
        r#" <span class="ml-2 px-2 py-1 text-xs rounded bg-gray-200 text-gray-600">infra-only</span>"#
    } else {
        ""
    };

    let body = format!(
        r#"<p class="mb-4"><a class="text-blue-700 underline" href="/admin/compliance">← back to dashboard</a></p>
<header class="mb-6">
  <h1 class="text-2xl font-bold">{name}{badge}</h1>
  <p class="text-gray-600">upstream: {upstream}</p>
</header>
<section class="grid grid-cols-3 gap-4 mb-6">
  <div class="p-4 bg-white rounded shadow">
    <div class="text-xs text-gray-500">4-TRACK SCORE</div>
    <div class="text-3xl font-bold {score_color} px-2 inline-block rounded">{score}</div>
  </div>
  <div class="p-4 bg-white rounded shadow">
    <div class="text-xs text-gray-500">STUB MARKERS</div>
    <div class="text-3xl font-bold">{stubs}</div>
    <div class="text-xs text-gray-400">unimpl {unimpl} · todo {todo} · ignored {ignored}</div>
  </div>
  <div class="p-4 bg-white rounded shadow">
    <div class="text-xs text-gray-500">BACKEND</div>
    <div class="text-3xl font-bold">{loc}</div>
    <div class="text-xs text-gray-400">LOC across src/ · {tests} tests</div>
  </div>
</section>
<section class="mb-6"><h2 class="text-lg font-semibold mb-2">4-track breakdown</h2>{track}</section>
<section class="mb-6"><h2 class="text-lg font-semibold mb-2">Ignored tests</h2>{ignored_block}</section>
<section class="mb-6"><h2 class="text-lg font-semibold mb-2">parity.manifest.toml</h2>{manifest}</section>
<section class="mb-6"><h2 class="text-lg font-semibold mb-2">Recent commits (last 10)</h2>{commits}</section>
"#,
        name = escape(&c.name),
        badge = infra_badge,
        upstream = escape(&upstream),
        score = c.four_track_score,
        score_color = cell_color(c.four_track_score),
        stubs = c.unimplemented_count + c.todo_count + c.ignored_test_count,
        unimpl = c.unimplemented_count,
        todo = c.todo_count,
        ignored = c.ignored_test_count,
        loc = c.backend_loc,
        tests = c.backend_test_count,
        track = track_html,
        ignored_block = ignored_html,
        manifest = manifest_html,
        commits = commits_html,
    );
    Ok(page_shell(
        &format!("compliance · {} · {}", escape(ctx.tenant.as_str()), escape(&c.name)),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn compute_four_track_score_combines_signals() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Score.tsx",
            "scoreCard",
            "acme"
        );
        // All four present → 100.
        assert_eq!(compute_four_track_score(true, true, true, true), 100);
        // Backend only → 25.
        assert_eq!(compute_four_track_score(false, false, false, false), 25);
        // Portal + cavectl only → 75.
        assert_eq!(compute_four_track_score(true, true, false, false), 75);
        // Half-obs (alerts but no dashboard) → 25 + 25 + 25 + 12 = 87.
        assert_eq!(compute_four_track_score(true, true, true, false), 87);
    }

    #[test]
    fn compliance_grade_letter_maps_buckets() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Letter.tsx",
            "gradeLetter",
            "acme"
        );
        assert_eq!(compliance_grade_letter(100), 'A');
        assert_eq!(compliance_grade_letter(90), 'A');
        assert_eq!(compliance_grade_letter(85), 'B');
        assert_eq!(compliance_grade_letter(75), 'C');
        assert_eq!(compliance_grade_letter(65), 'D');
        assert_eq!(compliance_grade_letter(40), 'F');
        assert_eq!(compliance_grade_letter(0), 'F');
    }

    #[test]
    fn parse_parity_manifest_extracts_upstream_version_and_org_repo() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Manifest.tsx",
            "manifestParser",
            "acme"
        );
        let manifest = r#"
# parity.manifest.toml — cave-cache
[upstream]
org     = "redis"
repo    = "redis"
version = "7.2.0"

[module]
name = "cave-cache"
"#;
        let (v, or) = parse_parity_manifest(manifest);
        assert_eq!(v.unwrap(), "7.2.0");
        assert_eq!(or.unwrap(), "redis/redis");
    }

    #[test]
    fn parse_parity_manifest_returns_none_for_missing_upstream() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Manifest.tsx",
            "missingUpstream",
            "acme"
        );
        let manifest = r#"[module]
name = "cave-x"
"#;
        let (v, or) = parse_parity_manifest(manifest);
        assert!(v.is_none());
        assert!(or.is_none());
    }

    #[test]
    fn aggregate_score_averages_per_crate_scores() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Aggregate.tsx",
            "aggregate",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![
                stub_compliance("a", 100),
                stub_compliance("b", 50),
                stub_compliance("c", 75),
            ],
        };
        // (100 + 50 + 75) / 3 = 75
        assert_eq!(snap.aggregate_score(), 75);
        assert_eq!(snap.grade(), 'C');
    }

    #[test]
    fn aggregate_score_excludes_infra_only_crates() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/InfraExempt.tsx",
            "InfraExempt",
            "acme"
        );
        // 1 tier-1 crate at 50 + 2 infra crates at 25 each.
        // Aggregate should be 50 (from the single tier-1), not (50+25+25)/3 = 33.
        let mut t1 = stub_compliance("cave-keda", 50);
        t1.infra_only = false;
        let mut i1 = stub_compliance("cave-cli", 25);
        i1.infra_only = true;
        let mut i2 = stub_compliance("cave-core", 25);
        i2.infra_only = true;
        let snap = ComplianceSnapshot { crates: vec![t1, i1, i2] };
        assert_eq!(snap.aggregate_score(), 50);
        assert_eq!(snap.tier1_count(), 1);
        assert_eq!(snap.infra_count(), 2);
    }

    #[test]
    fn is_infra_only_fallback_recognises_canonical_names() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/InfraList.tsx",
            "InfraList",
            "acme"
        );
        assert!(is_infra_only_fallback("cave-cli"));
        assert!(is_infra_only_fallback("cave-core"));
        assert!(is_infra_only_fallback("cave-runtime"));
        assert!(is_infra_only_fallback("cave-portal"));
        assert!(!is_infra_only_fallback("cave-keda"));
        assert!(!is_infra_only_fallback("cave-policy"));
        assert!(!is_infra_only_fallback("cave-rdbms-operator"));
    }

    #[test]
    fn parse_parity_manifest_full_reads_infra_only_flag_true() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Manifest.tsx",
            "manifestInfraTrue",
            "acme"
        );
        let manifest = r#"
[upstream]
org     = "cave-runtime"
repo    = "cave-runtime"
version = "v0.1.0"

[module]
name = "cave-cli"

[parity]
infra_only = true
"#;
        let m = parse_parity_manifest_full(manifest);
        assert_eq!(m.infra_only, Some(true));
        assert_eq!(m.upstream_version.as_deref(), Some("v0.1.0"));
    }

    #[test]
    fn parse_parity_manifest_full_reads_infra_only_flag_false() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Manifest.tsx",
            "manifestInfraFalse",
            "acme"
        );
        let manifest = r#"
[upstream]
org = "redis"
repo = "redis"
version = "7.2.0"

[parity]
infra_only = false
"#;
        let m = parse_parity_manifest_full(manifest);
        assert_eq!(m.infra_only, Some(false));
    }

    #[test]
    fn parse_parity_manifest_full_omits_infra_only_when_absent() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Manifest.tsx",
            "manifestInfraAbsent",
            "acme"
        );
        let manifest = r#"
[upstream]
org = "redis"
repo = "redis"
version = "7.2.0"
"#;
        let m = parse_parity_manifest_full(manifest);
        assert!(m.infra_only.is_none());
        assert_eq!(m.upstream_org_repo.as_deref(), Some("redis/redis"));
    }

    #[test]
    fn analyse_crate_picks_up_manifest_infra_only_override() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Manifest.tsx",
            "analyseCrateInfra",
            "acme"
        );
        let workspace = locate_workspace_root();
        // cave-cli has [parity] infra_only = true in its manifest.
        let c = analyse_crate(&workspace, "cave-cli").unwrap();
        assert!(c.infra_only);
        // cave-keda is not infra-only (no flag, not in fallback list).
        let k = analyse_crate(&workspace, "cave-keda").unwrap();
        assert!(!k.infra_only);
    }

    #[test]
    fn aggregate_score_for_empty_snapshot_is_zero() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/EmptyState.tsx",
            "EmptyState",
            "acme"
        );
        let snap = ComplianceSnapshot { crates: vec![] };
        assert_eq!(snap.aggregate_score(), 0);
        assert_eq!(snap.grade(), 'F');
    }

    #[test]
    fn total_stub_count_sums_unimpl_todo_ignored() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/StubTotal.tsx",
            "StubTotal",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![
                CrateCompliance {
                    name: "a".into(),
                    upstream_version: None,
                    upstream_org_repo: None,
                    backend_loc: 10,
                    backend_test_count: 1,
                    ignored_test_count: 3,
                    unimplemented_count: 5,
                    todo_count: 2,
                    portal_admin_present: false,
                    cavectl_subcommand_present: false,
                    obs_alerts_present: false,
                    obs_dashboard_present: false,
                    four_track_score: 25,
                    infra_only: false,
                },
            ],
        };
        assert_eq!(snap.total_stub_count(), 10);
    }

    #[test]
    fn build_snapshot_walks_real_workspace_and_orders_worst_first() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/ScorecardsPage.tsx",
            "buildSnapshot",
            "acme"
        );
        let workspace = locate_workspace_root();
        let snap = build_snapshot(&workspace, &["cave-cache", "cave-docdb", "cave-streams"])
            .unwrap();
        assert_eq!(snap.crates.len(), 3);
        // Lowest score first.
        for w in snap.crates.windows(2) {
            assert!(w[0].four_track_score <= w[1].four_track_score);
        }
        // cave-cache exists with substantial src + parity manifest.
        let cache = snap.crates.iter().find(|c| c.name == "cave-cache").unwrap();
        assert!(cache.backend_loc > 0);
        assert!(cache.upstream_version.is_some());
    }

    #[test]
    fn analyse_crate_errors_on_missing_directory() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Errors.tsx",
            "missingCrate",
            "acme"
        );
        let workspace = locate_workspace_root();
        let err = analyse_crate(&workspace, "cave-this-does-not-exist").unwrap_err();
        assert!(matches!(err, ComplianceViewError::BadRoot(_)));
    }

    #[test]
    fn render_refuses_without_view_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "viewGate",
            "acme"
        );
        let snap = ComplianceSnapshot { crates: vec![] };
        let err = render(&snap, &ctx(&[])).unwrap_err();
        assert!(matches!(err, ComplianceViewError::Auth(_)));
    }

    #[test]
    fn render_includes_golden_rule_quote_and_grade_card() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/ScorecardsPage.tsx",
            "RenderPage",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![stub_compliance("cave-x", 90)],
        };
        let html = render(&snap, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html.contains("line-by-line TDD"));
        assert!(html.contains("GRADE"));
        assert!(html.contains(">A<"));
        assert!(html.contains("cave-x"));
    }

    fn stub_compliance(name: &str, score: u8) -> CrateCompliance {
        CrateCompliance {
            name: name.into(),
            upstream_version: None,
            upstream_org_repo: None,
            backend_loc: 0,
            backend_test_count: 0,
            ignored_test_count: 0,
            unimplemented_count: 0,
            todo_count: 0,
            portal_admin_present: false,
            cavectl_subcommand_present: false,
            obs_alerts_present: false,
            obs_dashboard_present: false,
            four_track_score: score,
            infra_only: false,
        }
    }

    /// Find the workspace root by walking up from CARGO_MANIFEST_DIR until
    /// we hit a directory that contains both `crates/` and `Cargo.toml`.
    fn locate_workspace_root() -> PathBuf {
        let mut here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        loop {
            if here.join("crates").is_dir() && here.join("Cargo.toml").is_file() {
                return here;
            }
            if !here.pop() {
                panic!("workspace root not found");
            }
        }
    }

    // ── P3 cache tests ────────────────────────────────────────────────────────
    //
    // These tests share a process-wide cache via `OnceLock`, so they
    // serialise on the same mutex to avoid step-on-each-other ordering.

    use std::sync::Mutex as TestMutex;
    static CACHE_TEST_LOCK: TestMutex<()> = TestMutex::new(());

    fn reset_cache() {
        invalidate_cache();
    }

    #[test]
    fn cached_snapshot_returns_same_value_within_ttl() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Cache.tsx",
            "cacheHit",
            "acme"
        );
        let _serial = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_cache();
        let t0 = Utc::now();
        let first = cached_snapshot_or_refresh_at(t0, Duration::from_secs(300));
        // 30s later — still within the 5-min window, cache returns the same snapshot.
        let later = t0 + chrono::Duration::seconds(30);
        let second = cached_snapshot_or_refresh_at(later, Duration::from_secs(300));
        assert_eq!(first, second);
    }

    #[test]
    fn cached_snapshot_refreshes_after_ttl_expires() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Cache.tsx",
            "cacheStale",
            "acme"
        );
        let _serial = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_cache();
        let t0 = Utc::now();
        let _ = cached_snapshot_or_refresh_at(t0, Duration::from_secs(1));
        let cached_at_before = cache_cell()
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.cached_at)
            .unwrap();
        // Jump well past the 1-second TTL.
        let later = t0 + chrono::Duration::seconds(60);
        let _ = cached_snapshot_or_refresh_at(later, Duration::from_secs(1));
        let cached_at_after = cache_cell()
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.cached_at)
            .unwrap();
        assert_ne!(cached_at_before, cached_at_after);
        assert_eq!(cached_at_after, later);
    }

    #[test]
    fn force_refresh_replaces_cache_even_when_fresh() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Cache.tsx",
            "cacheManualRefresh",
            "acme"
        );
        let _serial = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_cache();
        let t0 = Utc::now();
        let _ = cached_snapshot_or_refresh_at(t0, Duration::from_secs(3600));
        let later = t0 + chrono::Duration::seconds(10);
        let entry = force_refresh_at(later);
        // Cache should now hold the forced timestamp, not t0.
        assert_eq!(entry.cached_at, later);
        let stored_at = cache_cell()
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.cached_at)
            .unwrap();
        assert_eq!(stored_at, later);
    }

    #[test]
    fn invalidate_cache_returns_previous_timestamp_then_clears() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Cache.tsx",
            "cacheInvalidate",
            "acme"
        );
        let _serial = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_cache();
        let t0 = Utc::now();
        let _ = cached_snapshot_or_refresh_at(t0, Duration::from_secs(300));
        let prev = invalidate_cache().unwrap();
        assert_eq!(prev, t0);
        // After invalidate, cache_cell is empty.
        assert!(cache_cell().lock().unwrap().is_none());
        // Second invalidate is a no-op — returns None.
        assert!(invalidate_cache().is_none());
    }

    #[test]
    fn handle_refresh_requires_refresh_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "refreshGate",
            "acme"
        );
        let _serial = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // View-only permission must be rejected.
        let view_only = ctx(&[Permission::AdminComplianceView]);
        let err = handle_refresh(&view_only).unwrap_err();
        assert!(matches!(err, ComplianceViewError::Auth(_)));
        // Refresh permission succeeds and renders the ack page.
        reset_cache();
        let refresher = ctx(&[Permission::AdminComplianceRefresh]);
        let html = handle_refresh(&refresher).unwrap();
        assert!(html.contains("Compliance cache refreshed"));
    }

    #[tokio::test]
    async fn spawn_background_refresh_is_idempotent_and_populates_cache() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Cache.tsx",
            "cacheBackground",
            "acme"
        );
        let _serial = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_cache();
        let handle = spawn_background_refresh(Duration::from_millis(50));
        // Wait long enough for at least one tick after the initial skip.
        tokio::time::sleep(Duration::from_millis(180)).await;
        handle.abort();
        // After the background pass, the cache should be populated and
        // a follow-up call within the TTL must return it without re-walking.
        let stored = cache_cell().lock().unwrap().clone().expect("cache populated");
        let now = stored.cached_at + chrono::Duration::seconds(1);
        let snap = cached_snapshot_or_refresh_at(now, Duration::from_secs(300));
        assert_eq!(snap, stored.snapshot);
    }

    // ── P4 detail-page tests ─────────────────────────────────────────────────

    #[test]
    fn build_crate_detail_returns_populated_struct_for_real_crate() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Drilldown.tsx",
            "drilldownData",
            "acme"
        );
        let workspace = locate_workspace_root();
        let detail = build_crate_detail(&workspace, "cave-cache").unwrap();
        assert_eq!(detail.compliance.name, "cave-cache");
        // cave-cache has a parity manifest in this repo.
        assert!(detail.parity_manifest_raw.is_some());
    }

    #[test]
    fn build_crate_detail_errors_for_unknown_crate() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Drilldown.tsx",
            "drilldownUnknown",
            "acme"
        );
        let workspace = locate_workspace_root();
        let err = build_crate_detail(&workspace, "cave-does-not-exist").unwrap_err();
        assert!(matches!(err, ComplianceViewError::BadRoot(_)));
    }

    #[test]
    fn render_detail_includes_back_link_score_and_manifest() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Drilldown.tsx",
            "drilldownRender",
            "acme"
        );
        let detail = CrateDetail {
            compliance: CrateCompliance {
                name: "cave-x".into(),
                upstream_version: Some("1.2.3".into()),
                upstream_org_repo: Some("org/repo".into()),
                backend_loc: 42,
                backend_test_count: 5,
                ignored_test_count: 1,
                unimplemented_count: 0,
                todo_count: 0,
                portal_admin_present: true,
                cavectl_subcommand_present: false,
                obs_alerts_present: true,
                obs_dashboard_present: false,
                four_track_score: 87,
                infra_only: false,
            },
            ignored_tests: vec![IgnoredTest {
                file: "crates/cave-x/src/lib.rs".into(),
                line: 17,
                name: "wip_thing".into(),
            }],
            parity_manifest_raw: Some("[upstream]\nversion = \"1.2.3\"\n".into()),
            recent_commits: vec![CommitRow {
                sha: "abc123".into(),
                subject: "feat(cave-x): kick off".into(),
            }],
        };
        let html = render_detail(&detail, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html.contains("← back to dashboard"));
        assert!(html.contains("cave-x"));
        assert!(html.contains(">87<"));
        assert!(html.contains("org/repo @ 1.2.3"));
        assert!(html.contains("wip_thing"));
        assert!(html.contains("abc123"));
        assert!(html.contains("crates/cave-x/src/lib.rs"));
    }

    #[test]
    fn render_detail_refuses_without_view_permission() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "drilldownGate",
            "acme"
        );
        let detail = CrateDetail {
            compliance: stub_compliance("cave-x", 50),
            ignored_tests: vec![],
            parity_manifest_raw: None,
            recent_commits: vec![],
        };
        let err = render_detail(&detail, &ctx(&[])).unwrap_err();
        assert!(matches!(err, ComplianceViewError::Auth(_)));
    }

    #[test]
    fn render_detail_marks_infra_only_in_header() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Drilldown.tsx",
            "drilldownInfra",
            "acme"
        );
        let mut compliance = stub_compliance("cave-cli", 25);
        compliance.infra_only = true;
        let detail = CrateDetail {
            compliance,
            ignored_tests: vec![],
            parity_manifest_raw: None,
            recent_commits: vec![],
        };
        let html = render_detail(&detail, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html.contains("infra-only"));
    }

    #[test]
    fn scan_ignored_tests_picks_up_attribute_and_fn_name() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Drilldown.tsx",
            "ignoredScan",
            "acme"
        );
        let tmp = tempfile_dir_with_ignored_fn();
        let crate_dir = tmp.path().join("crates/cave-toy/src");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(
            crate_dir.join("lib.rs"),
            "#[test]\n#[ignore = \"todo\"]\nfn wip_one() {}\n",
        )
        .unwrap();
        let found = scan_ignored_tests(tmp.path(), "cave-toy");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "wip_one");
        assert_eq!(found[0].line, 2);
        assert!(found[0].file.contains("lib.rs"));
    }

    fn tempfile_dir_with_ignored_fn() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("tempdir")
    }

    #[test]
    fn cached_snapshot_is_stale_handles_clock_skew() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Cache.tsx",
            "cacheSkew",
            "acme"
        );
        let now = Utc::now();
        let cached = CachedSnapshot::new(
            ComplianceSnapshot { crates: vec![] },
            now + chrono::Duration::seconds(60), // future timestamp
        );
        assert!(!cached.is_stale(now, Duration::from_secs(1)));
    }
}
