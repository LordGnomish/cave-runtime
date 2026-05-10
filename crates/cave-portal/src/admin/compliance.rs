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
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

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
}

/// Aggregated compliance state for the whole workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceSnapshot {
    pub crates: Vec<CrateCompliance>,
}

impl ComplianceSnapshot {
    pub fn aggregate_score(&self) -> u8 {
        if self.crates.is_empty() {
            return 0;
        }
        let total: u32 = self.crates.iter().map(|c| u32::from(c.four_track_score)).sum();
        ((total / self.crates.len() as u32).min(100)) as u8
    }

    pub fn total_stub_count(&self) -> u32 {
        self.crates
            .iter()
            .map(|c| c.unimplemented_count + c.todo_count + c.ignored_test_count)
            .sum()
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

/// Parse the `[upstream]` table from a parity manifest. Returns
/// `(version, org/repo)` if both fields are present. Tolerant of
/// missing or malformed manifests.
pub fn parse_parity_manifest(content: &str) -> (Option<String>, Option<String>) {
    let mut in_upstream = false;
    let mut org: Option<String> = None;
    let mut repo: Option<String> = None;
    let mut version: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_upstream = trimmed == "[upstream]";
            continue;
        }
        if !in_upstream {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("version") {
            version = extract_string_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("org") {
            org = extract_string_value(rest);
        } else if let Some(rest) = trimmed.strip_prefix("repo") {
            repo = extract_string_value(rest);
        }
    }
    let org_repo = match (org, repo) {
        (Some(o), Some(r)) => Some(format!("{o}/{r}")),
        _ => None,
    };
    (version, org_repo)
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
    let manifest = crate_root.join("parity.manifest.toml");
    let (upstream_version, upstream_org_repo) = if manifest.exists() {
        parse_parity_manifest(&fs::read_to_string(&manifest).unwrap_or_default())
    } else {
        (None, None)
    };
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

/// One-shot snapshot for the live admin handler.
pub fn live_snapshot() -> ComplianceSnapshot {
    let root = workspace_root();
    let names = discover_crate_names(&root);
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    build_snapshot(&root, &refs).unwrap_or(ComplianceSnapshot { crates: vec![] })
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
    let avg = snapshot.aggregate_score();
    let stubs = snapshot.total_stub_count();
    let grade = snapshot.grade();
    let avg_color = cell_color(avg);

    let rows: Vec<Vec<String>> = snapshot
        .crates
        .iter()
        .map(|c| {
            let score_html = format!(
                r#"<span class="px-2 py-1 rounded {color}">{score}</span>"#,
                color = cell_color(c.four_track_score),
                score = c.four_track_score,
            );
            vec![
                c.name.clone(),
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
<section class="grid grid-cols-4 gap-4 mb-6">
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">CRATES</div><div class="text-3xl font-bold">{total}</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">AVG 4-TRACK</div><div class="text-3xl font-bold {avg_color} px-2 rounded">{avg}</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL STUBS</div><div class="text-3xl font-bold">{stubs}</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">GRADE</div><div class="text-3xl font-bold">{grade}</div></div>
</section>
<section><h2 class="text-lg font-semibold mb-2">Per-crate matrix</h2>{tbl}</section>"#,
        quote = escape(GOLDEN_RULE),
        total = total,
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
}
