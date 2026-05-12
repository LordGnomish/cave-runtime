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
use std::collections::HashMap;
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
///
/// Two orthogonal axes:
/// * `four_track_score` — **structural** coverage (Backend present + Portal
///   admin page + cavectl subcommand + Observability files). Detects whether
///   the four delivery tracks have artefacts, not whether they are real
///   reimplementations.
/// * `parity_ratio` — **upstream parity** as scored by `cargo run -p
///   cave-kernel --example parity_audit` and surfaced via the audit doc
///   index (`docs/parity/parity-index.json`). Measures whether the cave
///   crate actually mirrors its declared upstream's items, functions,
///   tests, and surface APIs.
///
/// A crate can score 100 on `four_track_score` while having `parity_ratio
/// = 0.0` (scaffold only). The dashboard surfaces both grades so
/// "structural complete" is never mistaken for "upstream parity reached".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Upstream parity ratio in `[0.0, 1.0]` from the parity audit. `None`
    /// when the crate is not in the audit (e.g. infra-only or skeleton
    /// without a manifest). Surfaced as a second-axis grade so structural
    /// completion can never masquerade as real upstream parity.
    #[serde(default)]
    pub parity_ratio: Option<f64>,
    /// `Some(true)` when the crate's `parity.manifest.toml` declares at
    /// least one of `[[files]]`/`[[functions]]`/`[[tests]]`/`[[surfaces]]`;
    /// `Some(false)` when the manifest exists but every section is empty
    /// (calculator returns 0.0 regardless of impl size); `None` when no
    /// manifest is expected (CAVE-internal infra crates).
    #[serde(default)]
    pub manifest_filled: Option<bool>,
    /// Audit tier as defined in `full-audit-2026-05-01.md`:
    /// `"100"` reached, `"A"` close to 100, `"B"` partial-fill, `"C"`
    /// empty manifest with impl present, `"D1"` skeleton, `"D2"` no
    /// manifest yet, `"E"` infra-only.
    #[serde(default)]
    pub audit_tier: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplianceSnapshot {
    pub crates: Vec<CrateCompliance>,
}

impl ComplianceSnapshot {
    /// Aggregate **structural** score over tier-1 (non-infra) crates only.
    /// Infra crates are excluded — they don't ship Portal/cavectl/obs by
    /// contract. This is "do the four delivery tracks exist?", NOT "does
    /// the cave crate actually reach upstream parity?" — see
    /// [`Self::aggregate_parity_score`] for the latter.
    pub fn aggregate_score(&self) -> u8 {
        let scored: Vec<&CrateCompliance> = self.crates.iter().filter(|c| !c.infra_only).collect();
        if scored.is_empty() {
            return 0;
        }
        let total: u32 = scored.iter().map(|c| u32::from(c.four_track_score)).sum();
        ((total / scored.len() as u32).min(100)) as u8
    }

    /// Aggregate **parity** score (0-100) over tier-1 crates that have a
    /// known `parity_ratio`. Crates whose ratio is unknown (audit doesn't
    /// cover them, or `manifest_filled` is unknown) are excluded from the
    /// numerator AND denominator — measuring against an unknown is worse
    /// than measuring against zero.
    ///
    /// Returns 0 when no tier-1 crate has a measurable ratio.
    pub fn aggregate_parity_score(&self) -> u8 {
        let scored: Vec<f64> = self
            .crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.parity_ratio)
            .collect();
        if scored.is_empty() {
            return 0;
        }
        let avg = scored.iter().sum::<f64>() / scored.len() as f64;
        (avg.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// Fraction of tier-1 crates whose `parity.manifest.toml` is non-empty
    /// (i.e. declares any `[[files]]`/`[[functions]]`/`[[tests]]`/`[[surfaces]]`
    /// mappings). Returns `0.0` when no tier-1 crate has a known fill state.
    /// `0.0` when no tier-1 crate has a known fill state.
    pub fn manifest_fill_ratio(&self) -> f64 {
        let scored: Vec<bool> = self
            .crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.manifest_filled)
            .collect();
        if scored.is_empty() {
            return 0.0;
        }
        scored.iter().filter(|b| **b).count() as f64 / scored.len() as f64
    }

    /// Count of tier-1 crates with a known parity ratio (i.e. measurable
    /// against the audit). Used by the dashboard to surface "N of M
    /// tier-1 crates have a parity score" so the parity grade can't be
    /// gamed by hiding crates from the audit.
    pub fn parity_measured_count(&self) -> usize {
        self.crates
            .iter()
            .filter(|c| !c.infra_only && c.parity_ratio.is_some())
            .count()
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

    /// A-F letter for the **structural** aggregate (Portal/cavectl/obs
    /// presence). See [`Self::parity_grade`] for the upstream-parity grade.
    pub fn grade(&self) -> char {
        compliance_grade_letter(self.aggregate_score())
    }

    /// A-F letter for the **upstream parity** aggregate. Returns `'F'`
    /// when no crate has a measurable ratio so the grade is never
    /// "missing → A".
    pub fn parity_grade(&self) -> char {
        let avg_ratio = if self.parity_measured_count() == 0 {
            0.0
        } else {
            f64::from(self.aggregate_parity_score()) / 100.0
        };
        compute_parity_grade(avg_ratio)
    }
}

/// Map a 0-100 structural score to an A-F letter grade.
pub fn compliance_grade_letter(score: u8) -> char {
    match score {
        90..=u8::MAX => 'A',
        80..=89 => 'B',
        70..=79 => 'C',
        60..=69 => 'D',
        _ => 'F',
    }
}

/// Map a `[0.0, 1.0]` parity ratio to an A-F letter grade.
///
/// Boundaries (intentionally stricter than the structural grade — real
/// upstream parity is rarer than 4-track scaffolding):
/// * `>= 0.70` → A
/// * `>= 0.50` → B
/// * `>= 0.30` → C
/// * `>= 0.15` → D
/// * `< 0.15` → F
pub fn compute_parity_grade(ratio: f64) -> char {
    if ratio.is_nan() {
        return 'F';
    }
    let r = ratio.clamp(0.0, 1.0);
    if r >= 0.70 {
        'A'
    } else if r >= 0.50 {
        'B'
    } else if r >= 0.30 {
        'C'
    } else if r >= 0.15 {
        'D'
    } else {
        'F'
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

/// One row from `docs/parity/parity-index.json` — the audit-doc-derived
/// view of a crate's upstream-parity state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityIndexEntry {
    pub tier: String,
    #[serde(default)]
    pub parity_ratio: Option<f64>,
    #[serde(default)]
    pub manifest_filled: Option<bool>,
    #[serde(default)]
    pub cave_src_loc: Option<u64>,
    #[serde(default)]
    pub upstream: Option<String>,
    #[serde(default)]
    pub upstream_version: Option<String>,
    #[serde(default)]
    pub stubs: Option<u32>,
    #[serde(default)]
    pub note: Option<String>,
}

/// JSON wrapper matching the on-disk schema of `parity-index.json`.
/// `generated_from`/`generated_at` are deserialised so a `cargo about`-
/// style provenance log can surface them later; they are not currently
/// rendered.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct ParityIndexFile {
    #[serde(default)]
    generated_from: Option<String>,
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    crates: HashMap<String, ParityIndexEntry>,
}

/// Embedded snapshot of the parity audit index. The audit doc + index
/// JSON live in `docs/parity/`; the dashboard reads from the filesystem
/// at runtime (so a re-run of `scripts/build-parity-index.py` is picked
/// up without a rebuild) but ALSO embeds this fallback so the dashboard
/// still renders in environments where `docs/` isn't deployed.
const PARITY_INDEX_EMBEDDED: &str = include_str!("../../../../docs/parity/parity-index.json");

/// Load the parity index from a path on disk. Returns the inner crate
/// map. Silently returns an empty map on any read/parse failure — the
/// dashboard still renders, it just shows `parity_ratio: None` for
/// every crate.
pub fn load_parity_index_from(path: &Path) -> HashMap<String, ParityIndexEntry> {
    let Ok(raw) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    parse_parity_index_json(&raw)
}

/// Parse the parity-index JSON blob. Public for tests + the embedded-
/// fallback path; production callers should prefer
/// [`load_parity_index_from`] (filesystem) or [`load_parity_index`]
/// (filesystem-with-embedded-fallback).
pub fn parse_parity_index_json(raw: &str) -> HashMap<String, ParityIndexEntry> {
    match serde_json::from_str::<ParityIndexFile>(raw) {
        Ok(f) => f.crates,
        Err(_) => HashMap::new(),
    }
}

/// Load the parity index, preferring the on-disk copy under the given
/// workspace root and falling back to the binary-embedded snapshot. The
/// disk path is `<workspace>/docs/parity/parity-index.json`.
pub fn load_parity_index(workspace_root: &Path) -> HashMap<String, ParityIndexEntry> {
    let on_disk = load_parity_index_from(
        &workspace_root.join("docs/parity/parity-index.json"),
    );
    if !on_disk.is_empty() {
        return on_disk;
    }
    parse_parity_index_json(PARITY_INDEX_EMBEDDED)
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        // Parity-axis fields are attached in a separate pass by
        // [`attach_parity_index`] so the filesystem walk stays pure and
        // testable without the audit JSON.
        parity_ratio: None,
        manifest_filled: None,
        audit_tier: None,
    })
}

/// Mutate `snapshot` in place, copying audit-derived parity data from
/// `index` onto each matching crate. Crates absent from the index keep
/// their default `None` fields (the dashboard surfaces "—" + "audit
/// unknown" for those). Infra-only crates are touched too — the audit
/// tier `"E"` is informative even if no parity ratio applies.
pub fn attach_parity_index(
    snapshot: &mut ComplianceSnapshot,
    index: &HashMap<String, ParityIndexEntry>,
) {
    for c in &mut snapshot.crates {
        if let Some(entry) = index.get(&c.name) {
            c.parity_ratio = entry.parity_ratio;
            c.manifest_filled = entry.manifest_filled;
            c.audit_tier = Some(entry.tier.clone());
            // Backfill upstream metadata from the audit when the crate's
            // own manifest didn't surface it (typical for tier C/D where
            // the manifest exists but is sparse).
            if c.upstream_org_repo.is_none() {
                c.upstream_org_repo = entry.upstream.clone();
            }
            if c.upstream_version.is_none() {
                c.upstream_version = entry.upstream_version.clone();
            }
        }
    }
}

/// Build a compliance snapshot for the given crate name list. Loads
/// the parity audit index from `<workspace>/docs/parity/parity-index.json`
/// (or the binary-embedded fallback) and stamps `parity_ratio` +
/// `manifest_filled` + `audit_tier` onto each crate.
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
    let mut snapshot = ComplianceSnapshot { crates };
    let index = load_parity_index(workspace_root);
    attach_parity_index(&mut snapshot, &index);
    Ok(snapshot)
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// Column the dashboard rows are sorted by. The selector is exposed
/// in the URL (`?sort=score`); unknown values fall back to `Score`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Worst-compliance first — the default. Drives the maintainer's eye.
    Score,
    /// Highest stub indicator count first (unimpl + todo + ignored).
    StubCount,
    /// Crate name, lexicographic ascending — useful for "find a crate".
    Name,
    /// Worst upstream-parity ratio first; crates without a known ratio
    /// sort last so the dashboard surfaces measured-and-bad over
    /// measured-and-good before falling back to unmeasured.
    Parity,
}

impl SortKey {
    pub fn parse(s: &str) -> Self {
        match s {
            "stubs" | "stub_count" => SortKey::StubCount,
            "name" => SortKey::Name,
            "parity" => SortKey::Parity,
            _ => SortKey::Score,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SortKey::Score => "score",
            SortKey::StubCount => "stubs",
            SortKey::Name => "name",
            SortKey::Parity => "parity",
        }
    }
}

impl Default for SortKey {
    fn default() -> Self {
        SortKey::Score
    }
}

/// Optional filter applied before sorting. Multiple values aren't
/// combined — the selector is single-pick to keep the URL readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    /// Show every crate, including infra-only ones.
    All,
    /// Tier-1 crates whose 4-track score is below 50 (Grade F territory).
    ScoreUnder50,
    /// Tier-1 crates that have at least one missing 4-track signal
    /// (i.e. score < 100). Highlights the long tail.
    TrackGap,
    /// Hide infra-only crates entirely.
    ExcludeInfra,
    /// Tier-1 crates whose `parity.manifest.toml` is empty (no `[[files]]`/
    /// `[[functions]]`/`[[tests]]`/`[[surfaces]]` declared). These score 0
    /// in the parity audit regardless of impl size — biggest dashboard
    /// lever per hour spent (per the audit).
    ManifestEmpty,
    /// Tier-1 crates with a known parity ratio above 0 — i.e. the ones
    /// actually being measured. Useful to focus on real progress instead
    /// of the empty-manifest long tail.
    ParityMeasured,
}

impl FilterMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "score_lt_50" | "score<50" => FilterMode::ScoreUnder50,
            "track_gap" => FilterMode::TrackGap,
            "exclude_infra" => FilterMode::ExcludeInfra,
            "manifest_empty" => FilterMode::ManifestEmpty,
            "parity_measured" => FilterMode::ParityMeasured,
            _ => FilterMode::All,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FilterMode::All => "all",
            FilterMode::ScoreUnder50 => "score_lt_50",
            FilterMode::TrackGap => "track_gap",
            FilterMode::ExcludeInfra => "exclude_infra",
            FilterMode::ManifestEmpty => "manifest_empty",
            FilterMode::ParityMeasured => "parity_measured",
        }
    }
}

impl Default for FilterMode {
    fn default() -> Self {
        FilterMode::All
    }
}

/// Query-string knobs for the dashboard view — survives across links
/// so a maintainer can deep-link into a filtered view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewQuery {
    pub sort: SortKey,
    pub filter: FilterMode,
}

impl ViewQuery {
    /// Apply filter + sort to the snapshot's crate list. Returns a new
    /// vec — the underlying snapshot is left untouched.
    pub fn apply(&self, snapshot: &ComplianceSnapshot) -> Vec<CrateCompliance> {
        let mut rows: Vec<CrateCompliance> = snapshot
            .crates
            .iter()
            .filter(|c| match self.filter {
                FilterMode::All => true,
                FilterMode::ScoreUnder50 => !c.infra_only && c.four_track_score < 50,
                FilterMode::TrackGap => !c.infra_only && c.four_track_score < 100,
                FilterMode::ExcludeInfra => !c.infra_only,
                FilterMode::ManifestEmpty => !c.infra_only && c.manifest_filled == Some(false),
                FilterMode::ParityMeasured => {
                    !c.infra_only && c.parity_ratio.map(|r| r > 0.0).unwrap_or(false)
                }
            })
            .cloned()
            .collect();
        match self.sort {
            SortKey::Score => rows.sort_by(|a, b| match a.four_track_score.cmp(&b.four_track_score) {
                Ordering::Equal => a.name.cmp(&b.name),
                ord => ord,
            }),
            SortKey::StubCount => {
                rows.sort_by(|a, b| {
                    let sa = a.unimplemented_count + a.todo_count + a.ignored_test_count;
                    let sb = b.unimplemented_count + b.todo_count + b.ignored_test_count;
                    match sb.cmp(&sa) {
                        Ordering::Equal => a.name.cmp(&b.name),
                        ord => ord,
                    }
                });
            }
            SortKey::Name => rows.sort_by(|a, b| a.name.cmp(&b.name)),
            SortKey::Parity => {
                rows.sort_by(|a, b| {
                    // Unmeasured last; otherwise ascending (worst first).
                    let key = |c: &CrateCompliance| match c.parity_ratio {
                        Some(r) => (0u8, (r * 10_000.0) as i64),
                        None => (1u8, i64::MAX),
                    };
                    match key(a).cmp(&key(b)) {
                        Ordering::Equal => a.name.cmp(&b.name),
                        ord => ord,
                    }
                });
            }
        }
        rows
    }

    /// Build the `?sort=…&filter=…` querystring fragment (without `?`).
    pub fn to_query_string(&self) -> String {
        format!("sort={}&filter={}", self.sort.as_str(), self.filter.as_str())
    }
}

pub fn render(
    snapshot: &ComplianceSnapshot,
    ctx: &RequestCtx,
) -> Result<String, ComplianceViewError> {
    render_with_view(snapshot, ctx, ViewQuery::default())
}

pub fn render_with_view(
    snapshot: &ComplianceSnapshot,
    ctx: &RequestCtx,
    view: ViewQuery,
) -> Result<String, ComplianceViewError> {
    ctx.authorise(Permission::AdminComplianceView)?;
    let filtered = view.apply(snapshot);
    let total = snapshot.crates.len();
    let tier1 = snapshot.tier1_count();
    let infra = snapshot.infra_count();
    let avg = snapshot.aggregate_score();
    let stubs = snapshot.total_stub_count();
    let grade = snapshot.grade();
    let avg_color = cell_color(avg);
    let parity_avg = snapshot.aggregate_parity_score();
    let parity_grade = snapshot.parity_grade();
    let parity_color = cell_color(parity_avg);
    let parity_measured = snapshot.parity_measured_count();
    let manifest_fill_pct = (snapshot.manifest_fill_ratio() * 100.0).round() as u8;

    let rows: Vec<Vec<String>> = filtered
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
            let parity_html = match (c.infra_only, c.parity_ratio) {
                (true, _) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-200 text-gray-600" title="infra-only">infra</span>"#,
                ),
                (false, Some(r)) => {
                    let pct = (r.clamp(0.0, 1.0) * 100.0).round() as u8;
                    let tier_badge = c
                        .audit_tier
                        .as_deref()
                        .map(|t| format!(r#" <span class="text-[10px] text-gray-500">·{}</span>"#, escape(t)))
                        .unwrap_or_default();
                    let warn = if c.manifest_filled == Some(false) {
                        r#" <span title="parity.manifest.toml is empty — calculator returns 0.0 regardless of impl size" class="text-orange-600">⚠</span>"#
                    } else {
                        ""
                    };
                    format!(
                        r#"<span class="px-2 py-1 rounded {color}">{pct}%</span>{warn}{badge}"#,
                        color = cell_color(pct),
                        pct = pct,
                        warn = warn,
                        badge = tier_badge,
                    )
                }
                (false, None) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-100 text-gray-500" title="not covered by parity audit">—</span>"#,
                ),
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
                parity_html,
            ]
        })
        .collect();

    let sort_form = format!(
        r#"<form method="get" action="/admin/compliance" class="mb-4 flex gap-3 items-end">
  <input type="hidden" name="tenant_id" value="{tenant}" />
  <label class="text-sm">sort by
    <select name="sort" class="border rounded px-2 py-1">
      <option value="score"{sel_score}>structural (worst first)</option>
      <option value="parity"{sel_parity}>parity (worst first)</option>
      <option value="stubs"{sel_stubs}>stub count (most first)</option>
      <option value="name"{sel_name}>name (A→Z)</option>
    </select>
  </label>
  <label class="text-sm">filter
    <select name="filter" class="border rounded px-2 py-1">
      <option value="all"{f_all}>all crates</option>
      <option value="score_lt_50"{f_lt50}>tier-1 with structural &lt; 50</option>
      <option value="track_gap"{f_gap}>tier-1 with any track gap</option>
      <option value="manifest_empty"{f_mfe}>tier-1 with empty manifest</option>
      <option value="parity_measured"{f_pms}>tier-1 with parity &gt; 0</option>
      <option value="exclude_infra"{f_xinf}>exclude infra-only</option>
    </select>
  </label>
  <button type="submit" class="bg-blue-600 text-white text-sm px-3 py-1 rounded">apply</button>
  <span class="text-xs text-gray-500 ml-2">showing {shown} / {total}</span>
</form>"#,
        tenant = escape(ctx.tenant.as_str()),
        sel_score = if view.sort == SortKey::Score { " selected" } else { "" },
        sel_parity = if view.sort == SortKey::Parity { " selected" } else { "" },
        sel_stubs = if view.sort == SortKey::StubCount { " selected" } else { "" },
        sel_name = if view.sort == SortKey::Name { " selected" } else { "" },
        f_all = if view.filter == FilterMode::All { " selected" } else { "" },
        f_lt50 = if view.filter == FilterMode::ScoreUnder50 { " selected" } else { "" },
        f_gap = if view.filter == FilterMode::TrackGap { " selected" } else { "" },
        f_mfe = if view.filter == FilterMode::ManifestEmpty { " selected" } else { "" },
        f_pms = if view.filter == FilterMode::ParityMeasured { " selected" } else { "" },
        f_xinf = if view.filter == FilterMode::ExcludeInfra { " selected" } else { "" },
        shown = filtered.len(),
        total = total,
    );

    let body = format!(
        r#"<section class="mb-6 p-4 bg-gray-100 rounded">
  <p class="italic text-gray-700">{quote}</p>
</section>
<section class="grid grid-cols-2 gap-4 mb-6">
  <div class="p-5 bg-white rounded shadow">
    <div class="text-xs uppercase text-gray-500 tracking-wide mb-1">Structural Coverage</div>
    <div class="flex items-baseline gap-3">
      <div class="text-4xl font-bold {avg_color} px-2 rounded">{avg}</div>
      <div class="text-3xl font-bold text-gray-700">Grade {grade}</div>
    </div>
    <div class="mt-2 text-xs text-gray-500">Backend + Portal page + cavectl + Observability artefacts present.<br/>Does NOT measure whether the four tracks reach upstream parity.</div>
  </div>
  <div class="p-5 bg-white rounded shadow">
    <div class="text-xs uppercase text-gray-500 tracking-wide mb-1">Upstream Parity</div>
    <div class="flex items-baseline gap-3">
      <div class="text-4xl font-bold {parity_color} px-2 rounded">{parity_avg}</div>
      <div class="text-3xl font-bold text-gray-700">Grade {parity_grade}</div>
    </div>
    <div class="mt-2 text-xs text-gray-500">
      Audit ratio across {parity_measured}/{tier1} tier-1 crates. Source:
      <a class="text-blue-700 underline" href="https://github.com/LordGnomish/cave-runtime/blob/main/docs/parity/full-audit-2026-05-01.md">full-audit-2026-05-01.md</a>.
      Manifest fill: {manifest_fill_pct}% of tier-1 crates declare items.
    </div>
  </div>
</section>
<section class="grid grid-cols-3 gap-4 mb-6">
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">TIER-1 CRATES</div><div class="text-3xl font-bold">{tier1}</div><div class="text-xs text-gray-400">+ {infra} infra · {total} total</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">TOTAL STUBS</div><div class="text-3xl font-bold">{stubs}</div><div class="text-xs text-gray-400">unimpl! + todo + ignored</div></div>
  <div class="p-4 bg-white rounded shadow"><div class="text-xs text-gray-500">PARITY MEASURED</div><div class="text-3xl font-bold">{parity_measured}</div><div class="text-xs text-gray-400">of {tier1} tier-1 in audit</div></div>
</section>
{sort_form}
<section><h2 class="text-lg font-semibold mb-2">Per-crate matrix</h2>{tbl}</section>"#,
        quote = escape(GOLDEN_RULE),
        total = total,
        tier1 = tier1,
        infra = infra,
        avg = avg,
        avg_color = avg_color,
        stubs = stubs,
        grade = grade,
        parity_avg = parity_avg,
        parity_color = parity_color,
        parity_grade = parity_grade,
        parity_measured = parity_measured,
        manifest_fill_pct = manifest_fill_pct,
        sort_form = sort_form,
        tbl = table(
            &[
                "crate", "upstream", "loc", "tests", "ignored", "unimpl!",
                "portal", "cavectl", "alerts", "dash", "structural", "parity",
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

/// Build the upstream-parity card + an optional "manifest empty" warning
/// banner for the drill-down view. Returns `(card_html, warning_html)`;
/// the warning is empty for infra-only / no-audit / filled-manifest crates.
fn render_parity_block(c: &CrateCompliance) -> (String, String) {
    if c.infra_only {
        let card = r#"<div class="p-4 bg-gray-100 rounded shadow">
  <div class="text-xs text-gray-500 uppercase tracking-wide">Upstream Parity</div>
  <div class="text-3xl font-bold text-gray-600 mt-1">infra</div>
  <div class="text-xs text-gray-500 mt-2">Infrastructure-only crate — no upstream counterpart.</div>
</div>"#.to_string();
        return (card, String::new());
    }
    let card = match c.parity_ratio {
        Some(r) => {
            let pct = (r.clamp(0.0, 1.0) * 100.0).round() as u8;
            let grade = compute_parity_grade(r);
            let tier_label = c
                .audit_tier
                .as_deref()
                .map(|t| format!("audit tier <strong>{}</strong>", escape(t)))
                .unwrap_or_else(|| "audit tier —".into());
            format!(
                r#"<div class="p-4 bg-white rounded shadow">
  <div class="text-xs text-gray-500 uppercase tracking-wide">Upstream Parity</div>
  <div class="flex items-baseline gap-3 mt-1">
    <div class="text-3xl font-bold {color} px-2 inline-block rounded">{pct}%</div>
    <div class="text-2xl font-bold text-gray-700">Grade {grade}</div>
  </div>
  <div class="text-xs text-gray-500 mt-2">{tier_label} — source: <a class="text-blue-700 underline" href="https://github.com/LordGnomish/cave-runtime/blob/main/docs/parity/full-audit-2026-05-01.md">full-audit-2026-05-01.md</a></div>
</div>"#,
                color = cell_color(pct),
                pct = pct,
                grade = grade,
                tier_label = tier_label,
            )
        }
        None => r#"<div class="p-4 bg-gray-50 rounded shadow border border-dashed">
  <div class="text-xs text-gray-500 uppercase tracking-wide">Upstream Parity</div>
  <div class="text-3xl font-bold text-gray-500 mt-1">—</div>
  <div class="text-xs text-gray-500 mt-2">Not covered by the parity audit (no parity.manifest.toml or audit-pending).</div>
</div>"#.to_string(),
    };
    let warning = if c.manifest_filled == Some(false) {
        r#"<section class="mb-6 p-3 bg-orange-50 border border-orange-200 rounded text-sm text-orange-800">
  <strong>⚠ parity.manifest empty.</strong> The manifest declares an upstream
  but no <code>[[files]]</code>/<code>[[functions]]</code>/<code>[[tests]]</code>/<code>[[surfaces]]</code>
  mappings, so the parity calculator returns <code>0.0</code> regardless of how
  much real code this crate ships. Fill in upstream→cave mappings (see
  <a class="underline" href="https://github.com/LordGnomish/cave-runtime/blob/main/docs/parity/full-audit-2026-05-01.md">audit doc</a>)
  to unlock a real parity score.
</section>"#.to_string()
    } else {
        String::new()
    };
    (card, warning)
}

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

    let (parity_card, parity_warn) = render_parity_block(c);

    let body = format!(
        r#"<p class="mb-4"><a class="text-blue-700 underline" href="/admin/compliance">← back to dashboard</a></p>
<header class="mb-6">
  <h1 class="text-2xl font-bold">{name}{badge}</h1>
  <p class="text-gray-600">upstream: {upstream}</p>
</header>
{warn}
<section class="grid grid-cols-2 gap-4 mb-6">
  <div class="p-4 bg-white rounded shadow">
    <div class="text-xs text-gray-500 uppercase tracking-wide">Structural Coverage</div>
    <div class="text-3xl font-bold {score_color} px-2 inline-block rounded mt-1">{score}</div>
    <div class="text-xs text-gray-500 mt-2">Backend + Portal + cavectl + Observability presence.</div>
  </div>
  {parity_card}
</section>
<section class="grid grid-cols-2 gap-4 mb-6">
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
        warn = parity_warn,
        score = c.four_track_score,
        score_color = cell_color(c.four_track_score),
        parity_card = parity_card,
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
                    parity_ratio: None,
                    manifest_filled: None,
                    audit_tier: None,
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
        // Dual-card header: structural grade + upstream parity grade.
        assert!(html.contains("Structural Coverage"));
        assert!(html.contains("Upstream Parity"));
        assert!(html.contains("Grade A"));
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
            parity_ratio: None,
            manifest_filled: None,
            audit_tier: None,
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
                parity_ratio: Some(0.42),
                manifest_filled: Some(true),
                audit_tier: Some("B".into()),
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

    // ── P5 sort/filter tests ─────────────────────────────────────────────────

    fn snap_three() -> ComplianceSnapshot {
        let mut a = stub_compliance("cave-a", 30);
        a.unimplemented_count = 5;
        let mut b = stub_compliance("cave-b", 70);
        b.unimplemented_count = 1;
        let mut c = stub_compliance("cave-c", 100);
        c.unimplemented_count = 0;
        let mut infra = stub_compliance("cave-cli", 25);
        infra.infra_only = true;
        ComplianceSnapshot { crates: vec![a, b, c, infra] }
    }

    #[test]
    fn view_query_sort_by_score_orders_worst_first() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "sortByScore",
            "acme"
        );
        let view = ViewQuery {
            sort: SortKey::Score,
            filter: FilterMode::All,
        };
        let rows = view.apply(&snap_three());
        assert_eq!(rows.first().unwrap().name, "cave-cli"); // 25 (lowest)
        assert_eq!(rows.last().unwrap().name, "cave-c");    // 100 (highest)
    }

    #[test]
    fn view_query_sort_by_stub_count_orders_most_first() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "sortByStubs",
            "acme"
        );
        let view = ViewQuery {
            sort: SortKey::StubCount,
            filter: FilterMode::All,
        };
        let rows = view.apply(&snap_three());
        // cave-a has 5 unimpl, the most.
        assert_eq!(rows.first().unwrap().name, "cave-a");
    }

    #[test]
    fn view_query_filter_score_under_50_drops_others_and_infra() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "filterScoreLt50",
            "acme"
        );
        let view = ViewQuery {
            sort: SortKey::Score,
            filter: FilterMode::ScoreUnder50,
        };
        let rows = view.apply(&snap_three());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "cave-a");
    }

    #[test]
    fn view_query_filter_track_gap_excludes_perfect_and_infra() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "filterTrackGap",
            "acme"
        );
        let view = ViewQuery {
            sort: SortKey::Name,
            filter: FilterMode::TrackGap,
        };
        let rows = view.apply(&snap_three());
        let names: Vec<&str> = rows.iter().map(|c| c.name.as_str()).collect();
        // cave-a (30) and cave-b (70) have gaps; cave-c (100) does not;
        // cave-cli (infra) is excluded regardless.
        assert_eq!(names, vec!["cave-a", "cave-b"]);
    }

    #[test]
    fn view_query_filter_exclude_infra_drops_infra_only() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "filterExcludeInfra",
            "acme"
        );
        let view = ViewQuery {
            sort: SortKey::Name,
            filter: FilterMode::ExcludeInfra,
        };
        let rows = view.apply(&snap_three());
        assert!(rows.iter().all(|c| !c.infra_only));
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn view_query_to_query_string_round_trips_through_parsers() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "urlRoundTrip",
            "acme"
        );
        let view = ViewQuery {
            sort: SortKey::StubCount,
            filter: FilterMode::TrackGap,
        };
        let qs = view.to_query_string();
        assert_eq!(qs, "sort=stubs&filter=track_gap");
        // Pretend the browser sent it back — parsers should recover the same view.
        let parsed_sort = SortKey::parse("stubs");
        let parsed_filter = FilterMode::parse("track_gap");
        assert_eq!(parsed_sort, SortKey::StubCount);
        assert_eq!(parsed_filter, FilterMode::TrackGap);
    }

    #[test]
    fn render_with_view_preserves_selection_in_form_markup() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/SortFilter.tsx",
            "renderPreservesSelection",
            "acme"
        );
        let snap = snap_three();
        let view = ViewQuery {
            sort: SortKey::StubCount,
            filter: FilterMode::TrackGap,
        };
        let html = render_with_view(&snap, &ctx(&[Permission::AdminComplianceView]), view).unwrap();
        // Both selected= markers should be present in the form HTML.
        assert!(html.contains(r#"value="stubs" selected"#));
        assert!(html.contains(r#"value="track_gap" selected"#));
        // Default render (no view) still works through the legacy entry point.
        let html_default = render(&snap, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html_default.contains(r#"value="score" selected"#));
        assert!(html_default.contains(r#"value="all" selected"#));
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

    // ---------------- parity-axis tests ----------------

    fn parity_compliance(
        name: &str,
        score: u8,
        ratio: Option<f64>,
        filled: Option<bool>,
        tier: Option<&str>,
        infra: bool,
    ) -> CrateCompliance {
        let mut c = stub_compliance(name, score);
        c.parity_ratio = ratio;
        c.manifest_filled = filled;
        c.audit_tier = tier.map(str::to_string);
        c.infra_only = infra;
        c
    }

    #[test]
    fn compute_parity_grade_maps_buckets() {
        assert_eq!(compute_parity_grade(1.0), 'A');
        assert_eq!(compute_parity_grade(0.70), 'A');
        assert_eq!(compute_parity_grade(0.69), 'B');
        assert_eq!(compute_parity_grade(0.50), 'B');
        assert_eq!(compute_parity_grade(0.49), 'C');
        assert_eq!(compute_parity_grade(0.30), 'C');
        assert_eq!(compute_parity_grade(0.29), 'D');
        assert_eq!(compute_parity_grade(0.15), 'D');
        assert_eq!(compute_parity_grade(0.14), 'F');
        assert_eq!(compute_parity_grade(0.0), 'F');
        // NaN and out-of-range inputs stay safe.
        assert_eq!(compute_parity_grade(f64::NAN), 'F');
        assert_eq!(compute_parity_grade(2.0), 'A'); // clamp
        assert_eq!(compute_parity_grade(-0.5), 'F'); // clamp
    }

    #[test]
    fn aggregate_parity_score_averages_only_measured_tier1() {
        let snap = ComplianceSnapshot {
            crates: vec![
                // Measured tier-1 contributors:
                parity_compliance("a", 100, Some(1.0), Some(true), Some("100"), false),
                parity_compliance("b", 100, Some(0.50), Some(true), Some("B"), false),
                // Unmeasured tier-1 — must be ignored, NOT counted as 0:
                parity_compliance("c", 75, None, None, None, false),
                // Infra crate — excluded regardless:
                parity_compliance("d", 100, Some(0.0), Some(false), Some("E"), true),
            ],
        };
        // Mean of (1.0, 0.50) = 0.75 → 75.
        assert_eq!(snap.aggregate_parity_score(), 75);
        assert_eq!(snap.parity_grade(), 'A');
        assert_eq!(snap.parity_measured_count(), 2);
    }

    #[test]
    fn aggregate_parity_score_is_zero_when_nothing_measured() {
        let snap = ComplianceSnapshot {
            crates: vec![
                parity_compliance("a", 100, None, None, None, false),
                parity_compliance("b", 25, None, None, None, false),
            ],
        };
        assert_eq!(snap.aggregate_parity_score(), 0);
        assert_eq!(snap.parity_grade(), 'F');
        assert_eq!(snap.parity_measured_count(), 0);
    }

    #[test]
    fn manifest_fill_ratio_is_fraction_of_known_filled() {
        let snap = ComplianceSnapshot {
            crates: vec![
                parity_compliance("a", 100, Some(0.4), Some(true), Some("A"), false),
                parity_compliance("b", 100, Some(0.0), Some(false), Some("C"), false),
                parity_compliance("c", 100, Some(0.0), Some(false), Some("C"), false),
                parity_compliance("d", 100, None, None, None, true), // infra excluded
            ],
        };
        // 1 filled / 3 tier-1-with-known-state = 0.333…
        let ratio = snap.manifest_fill_ratio();
        assert!((ratio - 1.0 / 3.0).abs() < 1e-9, "got {ratio}");
    }

    #[test]
    fn parse_parity_index_json_round_trips_audit_doc_snapshot() {
        // The embedded parity-index.json must round-trip and surface the
        // canonical Tier ✅ crates with ratio 1.0. After the 2026-05-12
        // disk-overlay pass, `manifest_filled` reflects the on-disk
        // manifest state (license + [parity] block present), so the
        // Tier C cave-net example below now reports `true` rather than
        // its original audit-doc `false`.
        //
        // cave-etcd is special: the 2026-05-12 inventory expansion
        // replaced its wave3 self-reported `parity_ratio = 1.0` with a
        // measured `fill_ratio = 0.9155` (30 mapped + 35 skipped of 71
        // total packages). The disk-overlay treats the newer
        // `last_audit` as authoritative, including for honest downgrades.
        let m = parse_parity_index_json(PARITY_INDEX_EMBEDDED);
        for name in ["cave-apiserver", "cave-cri", "cave-kubelet", "cave-scheduler"] {
            let e = m.get(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(e.tier, "100", "{name} should be tier 100");
            assert_eq!(e.parity_ratio, Some(1.0));
            assert_eq!(e.manifest_filled, Some(true));
        }
        let etcd = m.get("cave-etcd").unwrap();
        assert_eq!(etcd.tier, "100");
        assert_eq!(etcd.manifest_filled, Some(true));
        let etcd_ratio = etcd.parity_ratio.expect("cave-etcd has a measured ratio");
        assert!(
            (etcd_ratio - 0.9155).abs() < 1e-3,
            "cave-etcd ratio = {etcd_ratio}, expected ~0.9155"
        );
        let vault = m.get("cave-vault").unwrap();
        assert_eq!(vault.tier, "A");
        assert!(vault.parity_ratio.unwrap() > 0.6 && vault.parity_ratio.unwrap() < 0.7);
        // cave-net still reports tier C (audit doc was frozen 2026-05-01),
        // but its on-disk manifest now carries `fill_ratio = 1.0` and a
        // `[parity]` block; the disk-overlay propagates those to the
        // index so the dashboard reflects the live state.
        let net = m.get("cave-net").unwrap();
        assert_eq!(net.tier, "C");
        assert_eq!(net.parity_ratio, Some(1.0));
        assert_eq!(net.manifest_filled, Some(true));
    }

    #[test]
    fn parse_parity_index_json_returns_empty_on_garbage() {
        assert!(parse_parity_index_json("not json").is_empty());
        assert!(parse_parity_index_json("").is_empty());
    }

    #[test]
    fn attach_parity_index_stamps_fields_on_match_only() {
        let mut snap = ComplianceSnapshot {
            crates: vec![
                stub_compliance("cave-apiserver", 100),
                stub_compliance("cave-unknown-by-audit", 87),
            ],
        };
        let index = parse_parity_index_json(PARITY_INDEX_EMBEDDED);
        attach_parity_index(&mut snap, &index);
        let api = snap.crates.iter().find(|c| c.name == "cave-apiserver").unwrap();
        assert_eq!(api.parity_ratio, Some(1.0));
        assert_eq!(api.audit_tier.as_deref(), Some("100"));
        let unknown = snap
            .crates
            .iter()
            .find(|c| c.name == "cave-unknown-by-audit")
            .unwrap();
        assert!(unknown.parity_ratio.is_none());
        assert!(unknown.audit_tier.is_none());
    }

    #[test]
    fn view_query_sort_by_parity_orders_worst_first_then_unmeasured() {
        let snap = ComplianceSnapshot {
            crates: vec![
                parity_compliance("c-good", 100, Some(0.9), Some(true), Some("A"), false),
                parity_compliance("b-bad", 100, Some(0.1), Some(true), Some("B"), false),
                parity_compliance("d-unknown", 100, None, None, None, false),
                parity_compliance("a-empty", 100, Some(0.0), Some(false), Some("C"), false),
            ],
        };
        let view = ViewQuery {
            sort: SortKey::Parity,
            filter: FilterMode::All,
        };
        let names: Vec<String> = view.apply(&snap).into_iter().map(|c| c.name).collect();
        // a-empty (0.0) before b-bad (0.1) before c-good (0.9) before d-unknown (None).
        assert_eq!(names, vec!["a-empty", "b-bad", "c-good", "d-unknown"]);
    }

    #[test]
    fn view_query_filter_manifest_empty_keeps_only_empty_tier1() {
        let snap = ComplianceSnapshot {
            crates: vec![
                parity_compliance("empty1", 100, Some(0.0), Some(false), Some("C"), false),
                parity_compliance("filled", 100, Some(0.5), Some(true), Some("A"), false),
                parity_compliance("unknown", 100, None, None, None, false),
                parity_compliance("infra", 100, Some(0.0), Some(false), Some("E"), true),
            ],
        };
        let view = ViewQuery {
            sort: SortKey::Name,
            filter: FilterMode::ManifestEmpty,
        };
        let names: Vec<String> = view.apply(&snap).into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["empty1"]);
    }

    #[test]
    fn view_query_filter_parity_measured_keeps_only_positive_ratio_tier1() {
        let snap = ComplianceSnapshot {
            crates: vec![
                parity_compliance("zero", 100, Some(0.0), Some(false), Some("C"), false),
                parity_compliance("partial", 100, Some(0.25), Some(true), Some("B"), false),
                parity_compliance("perfect", 100, Some(1.0), Some(true), Some("100"), false),
                parity_compliance("unknown", 100, None, None, None, false),
            ],
        };
        let view = ViewQuery {
            sort: SortKey::Name,
            filter: FilterMode::ParityMeasured,
        };
        let names: Vec<String> = view.apply(&snap).into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["partial", "perfect"]);
    }

    #[test]
    fn render_dashboard_shows_dual_grade_cards_and_parity_column() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/DualGrade.tsx",
            "dualGrade",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![
                parity_compliance("cave-apiserver", 100, Some(1.0), Some(true), Some("100"), false),
                parity_compliance("cave-portal", 100, Some(0.25), Some(true), Some("B"), false),
            ],
        };
        let html = render(&snap, &ctx(&[Permission::AdminComplianceView])).unwrap();
        // Both cards present.
        assert!(html.contains("Structural Coverage"));
        assert!(html.contains("Upstream Parity"));
        // Structural grade A (avg 100) + parity grade B (avg 0.625) both render.
        assert!(html.contains("Grade A"), "expected structural Grade A");
        assert!(html.contains("Grade B"), "expected parity Grade B");
        // Per-row parity % shows up.
        assert!(html.contains("100%"));
        assert!(html.contains("25%"));
        // Manifest fill section appears.
        assert!(html.contains("Manifest fill:"));
    }

    #[test]
    fn render_dashboard_shows_dash_for_unmeasured_crate() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Unmeasured.tsx",
            "unmeasured",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![parity_compliance("cave-unmeasured", 87, None, None, None, false)],
        };
        let html = render(&snap, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html.contains("not covered by parity audit"));
    }

    #[test]
    fn render_detail_shows_parity_card_and_warning_for_empty_manifest() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/EmptyManifestBanner.tsx",
            "emptyBanner",
            "acme"
        );
        let detail = CrateDetail {
            compliance: parity_compliance(
                "cave-net",
                100,
                Some(0.0),
                Some(false),
                Some("C"),
                false,
            ),
            ignored_tests: vec![],
            parity_manifest_raw: Some("[upstream]\norg = \"cilium\"\n".into()),
            recent_commits: vec![],
        };
        let html = render_detail(&detail, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html.contains("Upstream Parity"));
        assert!(html.contains("parity.manifest empty"));
        assert!(html.contains("audit tier"));
    }

    /// Diagnostic-only: runs against the real workspace and prints the
    /// dual-grade snapshot. Ignored by default so CI doesn't depend on
    /// the audit JSON being checked in; run with
    /// `cargo test -p cave-portal -- --ignored --nocapture
    /// live_snapshot_dual_grade_prints`.
    #[test]
    #[ignore = "diagnostic — prints against the real workspace"]
    fn live_snapshot_dual_grade_prints() {
        let snap = live_snapshot();
        println!("crates total        : {}", snap.crates.len());
        println!("tier-1 count        : {}", snap.tier1_count());
        println!("infra count         : {}", snap.infra_count());
        println!(
            "structural          : {} (Grade {})",
            snap.aggregate_score(),
            snap.grade()
        );
        println!(
            "parity              : {} (Grade {})",
            snap.aggregate_parity_score(),
            snap.parity_grade()
        );
        println!(
            "parity measured     : {} / {}",
            snap.parity_measured_count(),
            snap.tier1_count()
        );
        println!(
            "manifest fill ratio : {:.1}%",
            snap.manifest_fill_ratio() * 100.0
        );
        println!("total stubs         : {}", snap.total_stub_count());
        let perfect: Vec<&str> = snap
            .crates
            .iter()
            .filter(|c| c.parity_ratio == Some(1.0))
            .map(|c| c.name.as_str())
            .collect();
        println!("at 100% parity      : {} → {:?}", perfect.len(), perfect);
    }

    #[test]
    fn render_detail_shows_infra_parity_label_for_infra_crate() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/InfraParity.tsx",
            "infraParity",
            "acme"
        );
        let detail = CrateDetail {
            compliance: parity_compliance("cave-runtime", 100, None, None, Some("E"), true),
            ignored_tests: vec![],
            parity_manifest_raw: None,
            recent_commits: vec![],
        };
        let html = render_detail(&detail, &ctx(&[Permission::AdminComplianceView])).unwrap();
        assert!(html.contains("Upstream Parity"));
        assert!(html.contains("Infrastructure-only crate"));
        // No misleading empty-manifest warning for infra crates.
        assert!(!html.contains("parity.manifest empty"));
    }
}
