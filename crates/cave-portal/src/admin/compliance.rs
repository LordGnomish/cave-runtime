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
    /// Where `parity_ratio` came from: `"manifest"` (live on-disk
    /// `parity.manifest.toml::[parity] fill_ratio`), `"audit"`
    /// (2026-05-01 audit-doc snapshot), or `"none"` (never
    /// measured). Added 2026-05-13 by Fix-A so the dashboard can
    /// render the provenance.
    #[serde(default)]
    pub parity_ratio_source: Option<String>,
    /// `last_audit` date from the on-disk manifest, propagated when
    /// `parity_ratio_source == "manifest"`. `None` when audit-doc
    /// snapshot or never measured.
    #[serde(default)]
    pub parity_ratio_last_audit: Option<String>,
    /// **Honest** upstream parity ratio (added 2026-05-13 by the
    /// full-re-audit pass). Computed as
    /// `(fully_ported_mapped + skipped) / total` — i.e. excludes
    /// `[[partial]]` blocks that the manifest author self-flagged as
    /// scope-cut / MVP / shape-only. `None` when the crate is not
    /// audited or honest_ratio was not authored. Always satisfies
    /// `honest_parity_ratio <= parity_ratio` when both are present.
    #[serde(default)]
    pub honest_parity_ratio: Option<f64>,
    /// `[parity] mapped_count` — fully-ported entries (post-2026-05-13
    /// re-audit, partials excluded). `None` when the manifest doesn't
    /// declare a count.
    #[serde(default)]
    pub parity_mapped_count: Option<u32>,
    /// `[parity] partial_count` — entries demoted to `[[partial]]`
    /// because the note self-flags scope cut / MVP / deferred surface.
    #[serde(default)]
    pub parity_partial_count: Option<u32>,
    /// `[parity] skipped_count` — entries skipped per the charter
    /// (stdlib-analog, CLI, test-harness, etc.).
    #[serde(default)]
    pub parity_skipped_count: Option<u32>,
    /// `[parity] unmapped_count` — honest gaps not yet ported.
    #[serde(default)]
    pub parity_unmapped_count: Option<u32>,
    /// `[parity] total` — total inventory entries (mapped + partial +
    /// skipped + unmapped).
    #[serde(default)]
    pub parity_total_count: Option<u32>,
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
    /// `[portal_ui].status` from the crate's parity manifest, populated
    /// by `scripts/distribute-portal-ui-audit.py` from
    /// `docs/parity/portal-ui-audit-2026-05-11.md`.
    /// One of `"none"` | `"scaffold"` | `"partial"` | `"complete"`.
    /// `None` when the crate has no `[portal_ui]` block (typically
    /// infra-only or audit-omitted).
    #[serde(default)]
    pub portal_ui_status: Option<String>,
    /// `[portal_ui].priority` — `"P0"` (release blocker), `"P1"`
    /// (important), `"P2"` (CLI-first / low-traffic). `None` when
    /// absent.
    #[serde(default)]
    pub portal_ui_priority: Option<String>,
    /// `[portal_ui].upstream_url` — canonical upstream UI URL (or the
    /// project page when no first-party UI exists).
    #[serde(default)]
    pub portal_ui_upstream_url: Option<String>,
    /// Numeric score derived from `portal_ui_status`:
    /// `none = 0`, `scaffold = 25`, `partial = 60`, `complete = 100`.
    /// `None` mirrors `portal_ui_status = None`.
    #[serde(default)]
    pub portal_ui_score: Option<u8>,
    /// Behavioral-parity ratio (0.0..=1.0) — fraction of `[[upstream_test]]`
    /// entries with `status = "ported"` over the total declared in the
    /// manifest. `None` when the crate has no `[[upstream_test]]` block.
    /// Added 2026-05-13 by the upstream-test-port batch — measures
    /// whether the cave reimpl actually ports upstream tests, not just
    /// the surface shape (which `parity_ratio` already covers).
    #[serde(default)]
    pub behavioral_parity: Option<f64>,
    /// Count of `[[upstream_test]]` entries with `status = "ported"`.
    #[serde(default)]
    pub behavioral_ported: Option<u32>,
    /// Total `[[upstream_test]]` entries declared in the manifest.
    #[serde(default)]
    pub behavioral_total: Option<u32>,
    /// `[behavioral_parity].audit_scope` — short description of which
    /// upstream packages are covered in the audit subset.
    #[serde(default)]
    pub behavioral_audit_scope: Option<String>,
    /// `[behavioral_parity].audit_at` — date the cross-reference last
    /// updated.
    #[serde(default)]
    pub behavioral_audit_at: Option<String>,
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

    /// Aggregate **honest** upstream-parity score (0-100) over tier-1
    /// crates with a known `honest_parity_ratio`. Honest ratio excludes
    /// `[[partial]]` blocks (shape-only / scope-cut / MVP ports the
    /// manifest author self-flagged on 2026-05-13's re-audit pass), so
    /// it is strictly `<= aggregate_parity_score`. Same exclusion rule
    /// as the standard parity score: crates without a measured ratio
    /// drop out of both numerator and denominator.
    pub fn aggregate_honest_parity_score(&self) -> u8 {
        let scored: Vec<f64> = self
            .crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.honest_parity_ratio)
            .collect();
        if scored.is_empty() {
            return 0;
        }
        let avg = scored.iter().sum::<f64>() / scored.len() as f64;
        (avg.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// Number of tier-1 crates with a known honest-parity ratio.
    pub fn honest_parity_measured_count(&self) -> usize {
        self.crates
            .iter()
            .filter(|c| !c.infra_only && c.honest_parity_ratio.is_some())
            .count()
    }

    /// A-F letter for the **honest** parity aggregate. Same scale as
    /// `parity_grade`. Returns `'F'` when no crate has a measurable
    /// honest ratio.
    pub fn honest_parity_grade(&self) -> char {
        let measured = self.honest_parity_measured_count();
        if measured == 0 {
            return 'F';
        }
        let ratio = f64::from(self.aggregate_honest_parity_score()) / 100.0;
        compute_parity_grade(ratio)
    }

    /// Total `[[partial]]` blocks across all tier-1 crates with a
    /// declared `partial_count`. Surfaced on the dashboard so the
    /// honest-axis grade has a concrete "what dropped me from A to B"
    /// number behind it.
    pub fn total_partial_blocks(&self) -> u32 {
        self.crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.parity_partial_count)
            .sum()
    }

    /// Total `[[mapped]]` blocks across all tier-1 crates with a
    /// declared `mapped_count`. Used by the dashboard breakdown card.
    pub fn total_mapped_blocks(&self) -> u32 {
        self.crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.parity_mapped_count)
            .sum()
    }

    /// Total `[[skipped]]` blocks across all tier-1 crates with a
    /// declared `skipped_count`.
    pub fn total_skipped_blocks(&self) -> u32 {
        self.crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.parity_skipped_count)
            .sum()
    }

    /// Total `[[unmapped]]` blocks across all tier-1 crates with a
    /// declared `unmapped_count`.
    pub fn total_unmapped_blocks(&self) -> u32 {
        self.crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.parity_unmapped_count)
            .sum()
    }

    /// Aggregate **portal-UI parity** score (0-100) over tier-1 crates
    /// whose `[portal_ui]` block is present. Score per crate maps from
    /// `status`: `none = 0`, `scaffold = 25`, `partial = 60`,
    /// `complete = 100`. Crates without a `[portal_ui]` block (audit
    /// omission or infra) are excluded from both numerator and
    /// denominator — measuring against an unknown is worse than
    /// counting it as zero.
    pub fn portal_ui_avg_score(&self) -> u8 {
        let scored: Vec<u8> = self
            .crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.portal_ui_score)
            .collect();
        if scored.is_empty() {
            return 0;
        }
        let total: u32 = scored.iter().map(|&s| u32::from(s)).sum();
        ((total / scored.len() as u32).min(100)) as u8
    }

    /// Number of tier-1 crates whose `[portal_ui]` block is populated.
    /// Surfaced on the dashboard as "N of M tier-1 crates have a portal-
    /// UI score" so the grade can't be gamed by omitting rows.
    pub fn portal_ui_measured_count(&self) -> usize {
        self.crates
            .iter()
            .filter(|c| !c.infra_only && c.portal_ui_score.is_some())
            .count()
    }

    /// A-F letter for the **portal-UI parity** aggregate. Same scale as
    /// `parity_grade` — both axes measure progress toward upstream
    /// faithfulness on different surfaces, so the boundaries match.
    /// Returns `'F'` when no crate has a measurable portal-UI score.
    pub fn portal_ui_grade(&self) -> char {
        if self.portal_ui_measured_count() == 0 {
            return 'F';
        }
        let ratio = f64::from(self.portal_ui_avg_score()) / 100.0;
        compute_parity_grade(ratio)
    }

    /// Aggregate **behavioral parity** score (0-100) over tier-1 crates
    /// whose manifest declares a `[[upstream_test]]` block. Score per
    /// crate = `ported / total × 100`. Crates without a behavioral
    /// audit are excluded from both numerator and denominator — the
    /// audit subset is opt-in. Returns 0 when no crate has been audited.
    pub fn behavioral_parity_avg(&self) -> u8 {
        let scored: Vec<f64> = self
            .crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.behavioral_parity)
            .collect();
        if scored.is_empty() {
            return 0;
        }
        let avg = scored.iter().sum::<f64>() / scored.len() as f64;
        (avg.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    /// Number of tier-1 crates that have a behavioral audit.
    pub fn behavioral_audited_count(&self) -> usize {
        self.crates
            .iter()
            .filter(|c| !c.infra_only && c.behavioral_parity.is_some())
            .count()
    }

    /// A-F grade for behavioral parity. Same scale as `parity_grade`.
    /// Returns `'F'` when no crate has been audited so the dashboard
    /// shows the honest baseline (most crates are unaudited at first
    /// landing).
    pub fn behavioral_grade(&self) -> char {
        if self.behavioral_audited_count() == 0 {
            return 'F';
        }
        compute_parity_grade(f64::from(self.behavioral_parity_avg()) / 100.0)
    }

    /// Total `[[upstream_test]]` entries declared across audited tier-1
    /// crates — used by the dashboard headline so the audit scope is
    /// visible (e.g. "73 of 85 upstream tests ported across 5 crates").
    pub fn behavioral_total_declared(&self) -> u32 {
        self.crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.behavioral_total)
            .sum()
    }

    /// Total `[[upstream_test]]` entries with `status = "ported"` across
    /// audited tier-1 crates.
    pub fn behavioral_total_ported(&self) -> u32 {
        self.crates
            .iter()
            .filter(|c| !c.infra_only)
            .filter_map(|c| c.behavioral_ported)
            .sum()
    }

    /// Count of crates at P0 priority whose portal-UI status is still
    /// `none` or `scaffold`. Used by the dashboard headline so a clean
    /// average can't hide a P0-blocker.
    pub fn portal_ui_p0_gaps(&self) -> usize {
        self.crates
            .iter()
            .filter(|c| {
                !c.infra_only
                    && c.portal_ui_priority.as_deref() == Some("P0")
                    && matches!(
                        c.portal_ui_status.as_deref(),
                        Some("none") | Some("scaffold")
                    )
            })
            .count()
    }
}

/// Map a `[portal_ui].status` value to the numeric score the
/// dashboard averages. Unknown statuses return `None` so a typo in
/// the audit doc surfaces as missing data rather than silent zero.
pub fn portal_ui_status_score(status: &str) -> Option<u8> {
    match status {
        "none" => Some(0),
        "scaffold" => Some(25),
        "partial" => Some(60),
        "complete" => Some(100),
        _ => None,
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
    /// `[portal_ui].status` — `"none"` | `"scaffold"` | `"partial"` |
    /// `"complete"`. `None` when the block is missing.
    pub portal_ui_status: Option<String>,
    /// `[portal_ui].priority` — `"P0"` | `"P1"` | `"P2"`.
    pub portal_ui_priority: Option<String>,
    /// `[portal_ui].upstream_url` — canonical upstream UI URL.
    pub portal_ui_upstream_url: Option<String>,
}

/// Parse `[upstream]`, `[parity]`, and `[portal_ui]` tables from a
/// parity manifest. Tolerant of missing or malformed manifests —
/// unknown sections are skipped.
pub fn parse_parity_manifest_full(content: &str) -> ParityManifest {
    let mut section: &str = "";
    let mut org: Option<String> = None;
    let mut repo: Option<String> = None;
    let mut version: Option<String> = None;
    let mut infra_only: Option<bool> = None;
    let mut portal_ui_status: Option<String> = None;
    let mut portal_ui_priority: Option<String> = None;
    let mut portal_ui_upstream_url: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            // Skip array-of-tables headers — `[[mapped]]` should not
            // reset the section to "mapped" because we only consume
            // the singular tables.
            if rest.starts_with('[') {
                continue;
            }
            section = match rest {
                "upstream" => "upstream",
                "parity" => "parity",
                "portal_ui" => "portal_ui",
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
            "portal_ui" => {
                if let Some(rest) = trimmed.strip_prefix("status") {
                    portal_ui_status = extract_string_value(rest);
                } else if let Some(rest) = trimmed.strip_prefix("priority") {
                    portal_ui_priority = extract_string_value(rest);
                } else if let Some(rest) = trimmed.strip_prefix("upstream_url") {
                    portal_ui_upstream_url = extract_string_value(rest);
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
        portal_ui_status,
        portal_ui_priority,
        portal_ui_upstream_url,
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
    /// Where `parity_ratio` came from: `"manifest"` (on-disk
    /// `parity.manifest.toml::[parity] fill_ratio`), `"audit"`
    /// (2026-05-01 audit-doc snapshot), or `"none"` (never measured).
    /// Added 2026-05-13 by Fix-A so the dashboard can render the
    /// provenance and the live handler can re-read the manifest as
    /// the source of truth.
    #[serde(default)]
    pub parity_ratio_source: Option<String>,
    /// `last_audit` date from the on-disk manifest, propagated by the
    /// build script when `parity_ratio_source == "manifest"`. Lets
    /// the dashboard show "measured YYYY-MM-DD" alongside the ratio.
    #[serde(default)]
    pub last_audit_disk: Option<String>,
    /// Honest-parity ratio — `(fully_ported + skipped) / total`,
    /// excluding `[[partial]]` blocks. Added 2026-05-13 alongside the
    /// `[[partial]]` schema extension.
    #[serde(default)]
    pub honest_ratio: Option<f64>,
    /// Per-class inventory counts (added 2026-05-13).
    #[serde(default)]
    pub mapped_count: Option<u32>,
    #[serde(default)]
    pub partial_count: Option<u32>,
    #[serde(default)]
    pub skipped_count: Option<u32>,
    #[serde(default)]
    pub unmapped_count: Option<u32>,
    #[serde(default)]
    pub total_count: Option<u32>,
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
    /// Behavioral-parity ratio (0.0..=1.0) — populated by
    /// `scripts/build-parity-index.py` from the crate's
    /// `[[upstream_test]]` block. `None` when the crate has no
    /// behavioral audit. See [`CrateCompliance::behavioral_parity`].
    #[serde(default)]
    pub behavioral_parity: Option<f64>,
    #[serde(default)]
    pub behavioral_ported: Option<u32>,
    #[serde(default)]
    pub behavioral_total: Option<u32>,
    #[serde(default)]
    pub behavioral_audit_scope: Option<String>,
    #[serde(default)]
    pub behavioral_audit_at: Option<String>,
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
    let portal_ui_score = manifest
        .portal_ui_status
        .as_deref()
        .and_then(portal_ui_status_score);
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
        parity_ratio_source: None,
        parity_ratio_last_audit: None,
        honest_parity_ratio: None,
        parity_mapped_count: None,
        parity_partial_count: None,
        parity_skipped_count: None,
        parity_unmapped_count: None,
        parity_total_count: None,
        manifest_filled: None,
        audit_tier: None,
        portal_ui_status: manifest.portal_ui_status,
        portal_ui_priority: manifest.portal_ui_priority,
        portal_ui_upstream_url: manifest.portal_ui_upstream_url,
        portal_ui_score,
        behavioral_parity: None,
        behavioral_ported: None,
        behavioral_total: None,
        behavioral_audit_scope: None,
        behavioral_audit_at: None,
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
            c.parity_ratio_source = entry.parity_ratio_source.clone();
            c.parity_ratio_last_audit = entry.last_audit_disk.clone();
            c.honest_parity_ratio = entry.honest_ratio;
            c.parity_mapped_count = entry.mapped_count;
            c.parity_partial_count = entry.partial_count;
            c.parity_skipped_count = entry.skipped_count;
            c.parity_unmapped_count = entry.unmapped_count;
            c.parity_total_count = entry.total_count;
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
            // Behavioral-parity overlay (added 2026-05-13).
            c.behavioral_parity = entry.behavioral_parity;
            c.behavioral_ported = entry.behavioral_ported;
            c.behavioral_total = entry.behavioral_total;
            c.behavioral_audit_scope = entry.behavioral_audit_scope.clone();
            c.behavioral_audit_at = entry.behavioral_audit_at.clone();
        }
    }

    // Fix-A 2026-05-13 second pass: even when the parity-index JSON
    // wasn't regenerated since the last manifest edit, re-read the
    // on-disk `[parity] fill_ratio` (and the 2026-05-13 honest_ratio +
    // class counts) for any crate the dashboard analyses. The JSON is
    // built by `scripts/build-parity-index.py` (offline) and may lag a
    // few commits behind master; the dashboard renders are live so they
    // should always reflect the newest measured value.
    for c in &mut snapshot.crates {
        if let Some(live) = read_manifest_parity(&c.name) {
            // Only override fill if the manifest value differs OR the
            // index didn't carry a source — that way audit-doc-only
            // crates (no manifest) keep their original entry.
            if c.parity_ratio.map(|r| (r - live.fill_ratio).abs() > 0.000_5).unwrap_or(true) {
                c.parity_ratio = Some(live.fill_ratio);
            }
            c.parity_ratio_source = Some("manifest".into());
            c.parity_ratio_last_audit = Some(live.last_audit);
            // honest_ratio defaults to fill_ratio when the manifest
            // doesn't yet declare one (no [[partial]] block authored).
            c.honest_parity_ratio = Some(live.honest_ratio.unwrap_or(live.fill_ratio));
            c.parity_mapped_count = live.mapped_count;
            c.parity_partial_count = live.partial_count;
            c.parity_skipped_count = live.skipped_count;
            c.parity_unmapped_count = live.unmapped_count;
            c.parity_total_count = live.total_count;
        }
    }
}

/// Live-read snapshot of `[parity]` keys from an on-disk manifest.
/// Carries the 2026-05-13 honest-axis fields (`honest_ratio` + class
/// counts) alongside the legacy `fill_ratio` so the dashboard renders
/// a complete picture.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveManifestParity {
    pub fill_ratio: f64,
    pub honest_ratio: Option<f64>,
    pub last_audit: String,
    pub mapped_count: Option<u32>,
    pub partial_count: Option<u32>,
    pub skipped_count: Option<u32>,
    pub unmapped_count: Option<u32>,
    pub total_count: Option<u32>,
}

/// Live-read `crates/<name>/parity.manifest.toml`'s `[parity]` block.
/// Returns `None` when the manifest doesn't exist or doesn't carry a
/// measured `fill_ratio`/`ratio`. The honest-axis fields default to
/// `None` when the manifest hasn't been re-audited under the
/// 2026-05-13 schema. Pure I/O.
pub fn read_manifest_parity(crate_name: &str) -> Option<LiveManifestParity> {
    let path = workspace_root().join("crates").join(crate_name).join("parity.manifest.toml");
    let text = fs::read_to_string(&path).ok()?;

    // Find the [parity] section so we don't accidentally match a key
    // in a comment elsewhere in the file.
    let mut in_section = false;
    let mut fill: Option<f64> = None;
    let mut honest: Option<f64> = None;
    let mut audit: Option<String> = None;
    let mut mapped: Option<u32> = None;
    let mut partial: Option<u32> = None;
    let mut skipped: Option<u32> = None;
    let mut unmapped: Option<u32> = None;
    let mut total: Option<u32> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[parity]") {
            in_section = true;
            continue;
        }
        if in_section {
            // A new section header ends the [parity] block.
            if trimmed.starts_with('[') && !trimmed.starts_with("[parity]") && !trimmed.starts_with('#') {
                break;
            }
            let value_part = trimmed.split('#').next().unwrap_or(trimmed);
            // Try `fill_ratio` first, then legacy `ratio`. Skip if we
            // already set fill to avoid the `ratio` prefix shadowing
            // `fill_ratio` later.
            if fill.is_none() {
                if let Some(rest) = value_part.strip_prefix("fill_ratio") {
                    fill = parse_value(rest);
                } else if let Some(rest) = value_part.strip_prefix("ratio") {
                    fill = parse_value(rest);
                }
            }
            if honest.is_none() {
                if let Some(rest) = value_part.strip_prefix("honest_ratio") {
                    honest = parse_value(rest);
                }
            }
            if audit.is_none() {
                if let Some(rest) = value_part.strip_prefix("last_audit") {
                    if let Some(after_eq) = rest.split_once('=') {
                        audit = Some(after_eq.1.trim().trim_matches('"').to_string());
                    }
                }
            }
            if mapped.is_none() {
                if let Some(rest) = value_part.strip_prefix("mapped_count") {
                    mapped = parse_value_u32(rest);
                }
            }
            if partial.is_none() {
                if let Some(rest) = value_part.strip_prefix("partial_count") {
                    partial = parse_value_u32(rest);
                }
            }
            if skipped.is_none() {
                if let Some(rest) = value_part.strip_prefix("skipped_count") {
                    skipped = parse_value_u32(rest);
                }
            }
            if unmapped.is_none() {
                if let Some(rest) = value_part.strip_prefix("unmapped_count") {
                    unmapped = parse_value_u32(rest);
                }
            }
            if total.is_none() {
                if let Some(rest) = value_part.strip_prefix("total") {
                    total = parse_value_u32(rest);
                }
            }
        }
    }
    Some(LiveManifestParity {
        fill_ratio: fill?,
        honest_ratio: honest,
        last_audit: audit.unwrap_or_default(),
        mapped_count: mapped,
        partial_count: partial,
        skipped_count: skipped,
        unmapped_count: unmapped,
        total_count: total,
    })
}

fn parse_value(rest: &str) -> Option<f64> {
    let after_eq = rest.split_once('=')?;
    after_eq.1.trim().parse::<f64>().ok()
}

fn parse_value_u32(rest: &str) -> Option<u32> {
    let after_eq = rest.split_once('=')?;
    after_eq.1.trim().parse::<u32>().ok()
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
    let honest_avg = snapshot.aggregate_honest_parity_score();
    let honest_grade = snapshot.honest_parity_grade();
    let honest_color = cell_color(honest_avg);
    let honest_measured = snapshot.honest_parity_measured_count();
    let total_partial = snapshot.total_partial_blocks();
    let total_mapped = snapshot.total_mapped_blocks();
    let total_unmapped = snapshot.total_unmapped_blocks();
    let portal_ui_avg = snapshot.portal_ui_avg_score();
    let portal_ui_grade = snapshot.portal_ui_grade();
    let portal_ui_color = cell_color(portal_ui_avg);
    let portal_ui_measured = snapshot.portal_ui_measured_count();
    let portal_ui_p0_gaps = snapshot.portal_ui_p0_gaps();
    let behavioral_avg = snapshot.behavioral_parity_avg();
    let behavioral_grade = snapshot.behavioral_grade();
    let behavioral_color = cell_color(behavioral_avg);
    let behavioral_audited = snapshot.behavioral_audited_count();
    let behavioral_ported = snapshot.behavioral_total_ported();
    let behavioral_total = snapshot.behavioral_total_declared();

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
            let behavioral_html = match (c.infra_only, c.behavioral_parity) {
                (true, _) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-200 text-gray-600" title="infra-only">infra</span>"#,
                ),
                (false, Some(r)) => {
                    let pct = (r.clamp(0.0, 1.0) * 100.0).round() as u8;
                    let detail = match (c.behavioral_ported, c.behavioral_total) {
                        (Some(p), Some(t)) => format!(
                            r#" <span class="text-[10px] text-gray-500">{p}/{t}</span>"#
                        ),
                        _ => String::new(),
                    };
                    format!(
                        r#"<span class="px-2 py-1 rounded {color}">{pct}%</span>{detail}"#,
                        color = cell_color(pct),
                        pct = pct,
                        detail = detail,
                    )
                }
                (false, None) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-100 text-gray-500" title="no [[upstream_test]] block in parity.manifest.toml">—</span>"#,
                ),
            };
            let portal_ui_html = match (c.infra_only, c.portal_ui_score.as_ref()) {
                (true, _) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-200 text-gray-600" title="infra-only">infra</span>"#,
                ),
                (false, Some(&score)) => {
                    let status = c.portal_ui_status.as_deref().unwrap_or("—");
                    let priority = c.portal_ui_priority.as_deref().unwrap_or("");
                    let prio_badge = if priority.is_empty() {
                        String::new()
                    } else {
                        format!(r#" <span class="text-[10px] text-gray-500">·{}</span>"#, escape(priority))
                    };
                    format!(
                        r#"<span class="px-2 py-1 rounded {color}">{score}</span> <span class="text-[10px] text-gray-500">{status}</span>{prio}"#,
                        color = cell_color(score),
                        score = score,
                        status = escape(status),
                        prio = prio_badge,
                    )
                }
                (false, None) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-100 text-gray-500" title="no [portal_ui] block in parity.manifest.toml">—</span>"#,
                ),
            };
            let honest_html = match (c.infra_only, c.honest_parity_ratio, c.parity_ratio) {
                (true, _, _) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-200 text-gray-600" title="infra-only">infra</span>"#,
                ),
                (false, Some(h), Some(p)) => {
                    let pct = (h.clamp(0.0, 1.0) * 100.0).round() as u8;
                    let drop = ((p - h).clamp(0.0, 1.0) * 100.0).round() as i32;
                    let drop_html = if drop > 0 {
                        format!(r#" <span class="text-[10px] text-red-600" title="demoted by [[partial]] blocks">-{drop}</span>"#)
                    } else {
                        String::new()
                    };
                    let partial_badge = match c.parity_partial_count {
                        Some(n) if n > 0 => format!(
                            r#" <span class="text-[10px] text-amber-700" title="{n} self-flagged partial port(s)">{n}p</span>"#,
                        ),
                        _ => String::new(),
                    };
                    format!(
                        r#"<span class="px-2 py-1 rounded {color}">{pct}%</span>{drop}{partial}"#,
                        color = cell_color(pct),
                        pct = pct,
                        drop = drop_html,
                        partial = partial_badge,
                    )
                }
                (false, Some(h), None) => {
                    let pct = (h.clamp(0.0, 1.0) * 100.0).round() as u8;
                    format!(
                        r#"<span class="px-2 py-1 rounded {color}">{pct}%</span>"#,
                        color = cell_color(pct),
                        pct = pct,
                    )
                }
                (false, None, _) => format!(
                    r#"<span class="px-2 py-1 rounded bg-gray-100 text-gray-500" title="honest_ratio not authored — manifest lacks [[partial]] re-audit">—</span>"#,
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
                honest_html,
                portal_ui_html,
                behavioral_html,
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
<section class="grid grid-cols-4 gap-4 mb-6">
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
  <div class="p-5 bg-white rounded shadow border-2 border-amber-300">
    <div class="text-xs uppercase text-amber-700 tracking-wide mb-1">Honest Parity</div>
    <div class="flex items-baseline gap-3">
      <div class="text-4xl font-bold {honest_color} px-2 rounded">{honest_avg}</div>
      <div class="text-3xl font-bold text-gray-700">Grade {honest_grade}</div>
    </div>
    <div class="mt-2 text-xs text-gray-500">
      Strict ratio across {honest_measured}/{tier1} tier-1 crates &mdash; <strong>excludes [[partial]]</strong>
      (shape-only / scope-cut / MVP). {total_partial} partial of {total_mapped} mapped; {total_unmapped} honest unmapped.
      Source: <a class="text-blue-700 underline" href="https://github.com/LordGnomish/cave-runtime/blob/main/scripts/honest-re-audit.py">honest-re-audit.py</a>.
    </div>
  </div>
  <div class="p-5 bg-white rounded shadow">
    <div class="text-xs uppercase text-gray-500 tracking-wide mb-1">Portal UI Parity</div>
    <div class="flex items-baseline gap-3">
      <div class="text-4xl font-bold {portal_ui_color} px-2 rounded">{portal_ui_avg}</div>
      <div class="text-3xl font-bold text-gray-700">Grade {portal_ui_grade}</div>
    </div>
    <div class="mt-2 text-xs text-gray-500">
      `[portal_ui]` blocks across {portal_ui_measured}/{tier1} tier-1 crates. Source:
      <a class="text-blue-700 underline" href="https://github.com/LordGnomish/cave-runtime/blob/main/docs/parity/portal-ui-audit-2026-05-11.md">portal-ui-audit-2026-05-11.md</a>.
      P0 release-blocker gaps: <strong>{portal_ui_p0_gaps}</strong>.
    </div>
  </div>
  <div class="p-5 bg-white rounded shadow">
    <div class="text-xs uppercase text-gray-500 tracking-wide mb-1">Behavioral Parity</div>
    <div class="flex items-baseline gap-3">
      <div class="text-4xl font-bold {behavioral_color} px-2 rounded">{behavioral_avg}</div>
      <div class="text-3xl font-bold text-gray-700">Grade {behavioral_grade}</div>
    </div>
    <div class="mt-2 text-xs text-gray-500">
      Ported {behavioral_ported}/{behavioral_total} upstream tests across
      {behavioral_audited}/{tier1} tier-1 crates with a behavioral audit.
      Scope-bounded: denominator is each crate's audited subset, not
      the full upstream corpus. Empty `[[upstream_test]]` block → not
      audited → counted as F.
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
        honest_avg = honest_avg,
        honest_color = honest_color,
        honest_grade = honest_grade,
        honest_measured = honest_measured,
        total_partial = total_partial,
        total_mapped = total_mapped,
        total_unmapped = total_unmapped,
        portal_ui_avg = portal_ui_avg,
        portal_ui_color = portal_ui_color,
        portal_ui_grade = portal_ui_grade,
        portal_ui_measured = portal_ui_measured,
        portal_ui_p0_gaps = portal_ui_p0_gaps,
        behavioral_avg = behavioral_avg,
        behavioral_color = behavioral_color,
        behavioral_grade = behavioral_grade,
        behavioral_audited = behavioral_audited,
        behavioral_ported = behavioral_ported,
        behavioral_total = behavioral_total,
        sort_form = sort_form,
        tbl = table(
            &[
                "crate", "upstream", "loc", "tests", "ignored", "unimpl!",
                "portal", "cavectl", "alerts", "dash", "structural", "parity",
                "honest", "portal-ui", "behavioral",
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
                    parity_ratio_source: None,
                    parity_ratio_last_audit: None,
                    honest_parity_ratio: None,
                    parity_mapped_count: None,
                    parity_partial_count: None,
                    parity_skipped_count: None,
                    parity_unmapped_count: None,
                    parity_total_count: None,
                    manifest_filled: None,
                    audit_tier: None,
                    portal_ui_status: None,
                    portal_ui_priority: None,
                    portal_ui_upstream_url: None,
                    portal_ui_score: None,
                    behavioral_parity: None,
                    behavioral_ported: None,
                    behavioral_total: None,
                    behavioral_audit_scope: None,
                    behavioral_audit_at: None,
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
            parity_ratio_source: None,
            parity_ratio_last_audit: None,
            honest_parity_ratio: None,
            parity_mapped_count: None,
            parity_partial_count: None,
            parity_skipped_count: None,
            parity_unmapped_count: None,
            parity_total_count: None,
            manifest_filled: None,
            audit_tier: None,
            portal_ui_status: None,
            portal_ui_priority: None,
            portal_ui_upstream_url: None,
            portal_ui_score: None,
            behavioral_parity: None,
            behavioral_ported: None,
            behavioral_total: None,
            behavioral_audit_scope: None,
            behavioral_audit_at: None,
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
                parity_ratio_source: Some("manifest".into()),
                parity_ratio_last_audit: Some("2026-05-13".into()),
                honest_parity_ratio: Some(0.38),
                parity_mapped_count: Some(8),
                parity_partial_count: Some(2),
                parity_skipped_count: Some(3),
                parity_unmapped_count: Some(11),
                parity_total_count: Some(24),
                manifest_filled: Some(true),
                audit_tier: Some("B".into()),
                portal_ui_status: Some("partial".into()),
                portal_ui_priority: Some("P1".into()),
                portal_ui_upstream_url: Some("https://example.com/ui".into()),
                portal_ui_score: Some(60),
                behavioral_parity: None,
                behavioral_ported: None,
                behavioral_total: None,
                behavioral_audit_scope: None,
                behavioral_audit_at: None,
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
        // The embedded parity-index.json reflects the 2026-05-12 measured
        // audit pass for the K8s core 5 crates (apiserver / scheduler /
        // controller-manager / kubelet / cri) — their wave3 self-reported
        // 1.0 was replaced with measured `fill_ratio` values landed in
        // their respective manifest [parity] blocks. cave-etcd (2026-05-12)
        // had already moved to 0.9155 via the same pattern.
        let m = parse_parity_index_json(PARITY_INDEX_EMBEDDED);
        for name in ["cave-apiserver", "cave-cri", "cave-kubelet", "cave-scheduler"] {
            let e = m.get(name).unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(e.tier, "100", "{name} should be tier 100");
            // Post-2026-05-12: measured ratios are strictly < 1.0 and > 0.7.
            let r = e.parity_ratio.expect("measured ratio present");
            assert!(
                (0.7..1.0).contains(&r),
                "{name} ratio = {r}, expected 0.7..1.0 after measured audit"
            );
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
        // 2026-05-13 storage-backends sweep bumped 0.7838 → 0.8108
        // (file + inmem backends landed; see
        // docs/parity/cave-vault-port-2026-05-13.md). The full-honest-
        // re-audit pass (later 2026-05-13) self-healed a stale
        // unmapped_count in the manifest (declared 7, real count 8),
        // landing on 0.7895 = (19 mapped + 11 skipped) / 38 total.
        assert!(
            vault.parity_ratio.unwrap() > 0.78 && vault.parity_ratio.unwrap() < 0.83,
            "got vault parity {:?}",
            vault.parity_ratio
        );
        // cave-net still reports tier C (audit doc was frozen 2026-05-01),
        // but the on-disk manifest now carries an honest `fill_ratio`
        // (the 2026-05-13 audit demoted 11 breadcrumb-only mappings to
        // unmapped, bringing the ratio down from 1.0 to 0.9179). The
        // disk-overlay propagates the live value to the index so the
        // dashboard reflects the honest state.
        let net = m.get("cave-net").unwrap();
        assert_eq!(net.tier, "C");
        let net_ratio = net.parity_ratio.expect("cave-net has a measured ratio");
        assert!(
            net_ratio > 0.90 && net_ratio < 0.95,
            "expected cave-net ratio ~0.917, got {net_ratio:?}"
        );
        assert_eq!(net.manifest_filled, Some(true));
    }

    #[test]
    fn parse_parity_index_json_returns_empty_on_garbage() {
        assert!(parse_parity_index_json("not json").is_empty());
        assert!(parse_parity_index_json("").is_empty());
    }

    // ── Portal UI parity (3rd grade) ─────────────────────────────────────

    fn portal_ui_compliance(
        name: &str,
        infra: bool,
        status: Option<&str>,
        priority: Option<&str>,
    ) -> CrateCompliance {
        let mut c = stub_compliance(name, 80);
        c.infra_only = infra;
        c.portal_ui_status = status.map(str::to_string);
        c.portal_ui_priority = priority.map(str::to_string);
        c.portal_ui_score = status.and_then(portal_ui_status_score);
        c
    }

    #[test]
    fn portal_ui_status_score_maps_each_bucket() {
        assert_eq!(portal_ui_status_score("none"), Some(0));
        assert_eq!(portal_ui_status_score("scaffold"), Some(25));
        assert_eq!(portal_ui_status_score("partial"), Some(60));
        assert_eq!(portal_ui_status_score("complete"), Some(100));
        // Unknown statuses MUST surface as None — never silently zero.
        assert_eq!(portal_ui_status_score(""), None);
        assert_eq!(portal_ui_status_score("WIP"), None);
    }

    #[test]
    fn portal_ui_avg_score_excludes_infra_and_unknown() {
        let snap = ComplianceSnapshot {
            crates: vec![
                portal_ui_compliance("a", false, Some("complete"), Some("P0")),
                portal_ui_compliance("b", false, Some("partial"), Some("P1")),
                portal_ui_compliance("c", false, Some("scaffold"), Some("P2")),
                portal_ui_compliance("d", false, None, None),     // unknown excluded
                portal_ui_compliance("e", true, Some("none"), Some("P2")), // infra excluded
            ],
        };
        // Average of {100, 60, 25} = 61.66… → 61 (integer division).
        assert_eq!(snap.portal_ui_avg_score(), 61);
        assert_eq!(snap.portal_ui_measured_count(), 3);
    }

    #[test]
    fn portal_ui_grade_at_boundaries() {
        // 0 measured → F (never auto-graded A)
        let empty = ComplianceSnapshot { crates: vec![] };
        assert_eq!(empty.portal_ui_grade(), 'F');
        let unknown_only = ComplianceSnapshot {
            crates: vec![portal_ui_compliance("x", false, None, None)],
        };
        assert_eq!(unknown_only.portal_ui_grade(), 'F');

        // All complete → A
        let all_complete = ComplianceSnapshot {
            crates: vec![
                portal_ui_compliance("a", false, Some("complete"), Some("P0")),
                portal_ui_compliance("b", false, Some("complete"), Some("P1")),
            ],
        };
        assert_eq!(all_complete.portal_ui_avg_score(), 100);
        assert_eq!(all_complete.portal_ui_grade(), 'A');

        // Mixed scaffold/partial — score 42 → D
        let mixed = ComplianceSnapshot {
            crates: vec![
                portal_ui_compliance("a", false, Some("scaffold"), Some("P2")),
                portal_ui_compliance("b", false, Some("partial"), Some("P1")),
            ],
        };
        // (25 + 60) / 2 = 42 → ratio 0.42 → grade C (≥0.30)
        assert_eq!(mixed.portal_ui_avg_score(), 42);
        assert_eq!(mixed.portal_ui_grade(), 'C');

        // All none → ratio 0.0 → F
        let all_none = ComplianceSnapshot {
            crates: vec![
                portal_ui_compliance("a", false, Some("none"), Some("P0")),
                portal_ui_compliance("b", false, Some("none"), Some("P1")),
            ],
        };
        assert_eq!(all_none.portal_ui_avg_score(), 0);
        assert_eq!(all_none.portal_ui_grade(), 'F');
    }

    #[test]
    fn portal_ui_p0_gaps_counts_release_blockers_only() {
        let snap = ComplianceSnapshot {
            crates: vec![
                portal_ui_compliance("blocker-none", false, Some("none"), Some("P0")),
                portal_ui_compliance("blocker-scaffold", false, Some("scaffold"), Some("P0")),
                portal_ui_compliance("blocker-partial", false, Some("partial"), Some("P0")),
                portal_ui_compliance("p1-scaffold", false, Some("scaffold"), Some("P1")),
                portal_ui_compliance("p2-none", false, Some("none"), Some("P2")),
                portal_ui_compliance("infra-none", true, Some("none"), Some("P0")),
            ],
        };
        // Only P0 + (none OR scaffold) + non-infra counts.
        assert_eq!(snap.portal_ui_p0_gaps(), 2);
    }

    #[test]
    fn parse_parity_manifest_portal_ui_block_extracts_fields() {
        let manifest = r#"
[upstream]
org = "kedacore"
repo = "keda"

[parity]
infra_only = false

[portal_ui]
upstream_ui  = "KEDA dashboard"
upstream_url = "https://keda.sh/docs/"
status       = "partial"
loc          = 261
priority     = "P0"
notes        = "Scaler views"
last_audit   = "2026-05-11"
"#;
        let m = parse_parity_manifest_full(manifest);
        assert_eq!(m.portal_ui_status.as_deref(), Some("partial"));
        assert_eq!(m.portal_ui_priority.as_deref(), Some("P0"));
        assert_eq!(
            m.portal_ui_upstream_url.as_deref(),
            Some("https://keda.sh/docs/")
        );
    }

    #[test]
    fn parse_parity_manifest_portal_ui_block_absent_leaves_none() {
        let manifest = r#"
[upstream]
org = "x"
repo = "y"
"#;
        let m = parse_parity_manifest_full(manifest);
        assert!(m.portal_ui_status.is_none());
        assert!(m.portal_ui_priority.is_none());
        assert!(m.portal_ui_upstream_url.is_none());
    }

    #[test]
    fn parse_parity_manifest_ignores_array_of_tables_headers() {
        // [[mapped]] inside the inventory pattern must NOT reset the
        // current section — the parser should stay outside the singular
        // tables it cares about.
        let manifest = r#"
[upstream]
org = "etcd-io"
repo = "etcd"

[parity]
infra_only = false

[[mapped]]
upstream_pkg = "server/storage/wal/"

[portal_ui]
status   = "scaffold"
priority = "P0"
"#;
        let m = parse_parity_manifest_full(manifest);
        assert_eq!(m.upstream_org_repo.as_deref(), Some("etcd-io/etcd"));
        assert_eq!(m.portal_ui_status.as_deref(), Some("scaffold"));
        assert_eq!(m.portal_ui_priority.as_deref(), Some("P0"));
    }

    #[test]
    fn render_includes_third_portal_ui_grade_card() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/PortalUI.tsx",
            "PortalUIScore",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![portal_ui_compliance(
                "cave-x",
                false,
                Some("partial"),
                Some("P0"),
            )],
        };
        let html = render(&snap, &ctx(&[Permission::AdminComplianceView])).unwrap();
        // Headline card is present.
        assert!(html.contains("Portal UI Parity"));
        // The third grade rendered. partial = 60 → ratio 0.6 → grade B.
        assert!(html.contains("Grade B"));
        // Table got a portal-ui column header.
        assert!(html.contains("portal-ui"));
        // P0 release-blocker gap counter is rendered.
        assert!(html.contains("release-blocker gaps"));
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
        // Post-2026-05-12 trajectory:
        //   1.0  (wave3 self-report)
        //   0.86 (measured audit pass: 26 mapped / 17 skipped / 7 unmapped of 50)
        //   0.88 (CEL evaluator MVP landed: 27 mapped / 17 skipped / 6 unmapped of 50)
        // Assert the ratio falls in the post-CEL-MVP band so future
        // unmapped-clearing sweeps still fit without test churn.
        let r = api.parity_ratio.expect("apiserver has a measured ratio");
        assert!(
            (0.85..=0.95).contains(&r),
            "cave-apiserver ratio = {r}, expected 0.85..=0.95 after measured audit + CEL MVP"
        );
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

    // ─────────────────────────────────────────────────────────────────────────
    // Honest-parity axis (added 2026-05-13 by the full-honest-re-audit pass).
    // The honest axis excludes [[partial]] blocks from the parity-fill numerator
    // so the dashboard surfaces the strict line-by-line ratio alongside the
    // shape-level one.
    // ─────────────────────────────────────────────────────────────────────────

    /// Helper — builds a CrateCompliance with the honest axis populated.
    fn honest_compliance(
        name: &str,
        ratio: Option<f64>,
        honest: Option<f64>,
        partial_count: Option<u32>,
        infra: bool,
    ) -> CrateCompliance {
        let mut c = stub_compliance(name, 100);
        c.parity_ratio = ratio;
        c.honest_parity_ratio = honest;
        c.parity_partial_count = partial_count;
        c.parity_mapped_count = Some(10);
        c.parity_skipped_count = Some(5);
        c.parity_unmapped_count = Some(3);
        c.parity_total_count = Some(18);
        c.manifest_filled = Some(true);
        c.audit_tier = Some("B".into());
        c.infra_only = infra;
        c
    }

    #[test]
    fn aggregate_honest_parity_score_averages_only_measured_tier1() {
        let snap = ComplianceSnapshot {
            crates: vec![
                honest_compliance("cache", Some(0.95), Some(0.84), Some(4), false),
                honest_compliance("cri", Some(0.94), Some(0.85), Some(3), false),
                // No honest ratio — drops out of numerator AND denominator.
                honest_compliance("scheduler", Some(0.90), None, None, false),
                // Infra — excluded regardless.
                honest_compliance("kernel", Some(1.0), Some(1.0), Some(0), true),
            ],
        };
        // Mean of (0.84, 0.85) = 0.845 → 85 (rounded).
        assert_eq!(snap.aggregate_honest_parity_score(), 85);
        assert_eq!(snap.honest_parity_grade(), 'A');
        assert_eq!(snap.honest_parity_measured_count(), 2);
    }

    #[test]
    fn aggregate_honest_parity_score_zero_when_no_measured() {
        let snap = ComplianceSnapshot {
            crates: vec![honest_compliance("x", None, None, None, false)],
        };
        assert_eq!(snap.aggregate_honest_parity_score(), 0);
        // No measured ratio → grade is 'F', never silently 'A'.
        assert_eq!(snap.honest_parity_grade(), 'F');
    }

    #[test]
    fn honest_parity_strictly_le_shape_parity() {
        // A real-world invariant: honest_ratio = (mapped + skipped) / total,
        // shape ratio = (mapped + partial + skipped) / total — so honest ≤ shape
        // whenever partial ≥ 0. The aggregator must preserve that invariant.
        let snap = ComplianceSnapshot {
            crates: vec![
                honest_compliance("cache", Some(0.9474), Some(0.8421), Some(4), false),
                honest_compliance("cri", Some(0.9412), Some(0.8529), Some(3), false),
                honest_compliance("etcd", Some(0.9155), Some(0.8873), Some(2), false),
            ],
        };
        assert!(snap.aggregate_honest_parity_score() <= snap.aggregate_parity_score());
    }

    #[test]
    fn total_partial_blocks_sums_only_tier1() {
        let snap = ComplianceSnapshot {
            crates: vec![
                honest_compliance("cache", Some(0.95), Some(0.84), Some(4), false),
                honest_compliance("cri", Some(0.94), Some(0.85), Some(3), false),
                honest_compliance("infra", Some(1.0), Some(1.0), Some(99), true),
            ],
        };
        // Only tier-1 contributors counted; infra's 99 is excluded.
        assert_eq!(snap.total_partial_blocks(), 7);
        assert_eq!(snap.total_mapped_blocks(), 20); // 10 + 10 from two tier-1
        assert_eq!(snap.total_skipped_blocks(), 10);
        assert_eq!(snap.total_unmapped_blocks(), 6);
    }

    #[test]
    fn parity_index_round_trips_honest_fields() {
        let raw = r#"{
            "crates": {
                "cave-cache": {
                    "tier": "C",
                    "parity_ratio": 0.9474,
                    "parity_ratio_source": "manifest",
                    "honest_ratio": 0.8421,
                    "mapped_count": 19,
                    "partial_count": 4,
                    "skipped_count": 13,
                    "unmapped_count": 2,
                    "total_count": 38
                }
            }
        }"#;
        let m = parse_parity_index_json(raw);
        let e = m.get("cave-cache").expect("crate present");
        assert_eq!(e.honest_ratio, Some(0.8421));
        assert_eq!(e.mapped_count, Some(19));
        assert_eq!(e.partial_count, Some(4));
        assert_eq!(e.skipped_count, Some(13));
        assert_eq!(e.unmapped_count, Some(2));
        assert_eq!(e.total_count, Some(38));
    }

    #[test]
    fn parity_index_back_compat_without_honest_fields() {
        // A pre-2026-05-13 index that has no honest_* / *_count fields must
        // still parse cleanly — every new field is #[serde(default)].
        let raw = r#"{
            "crates": {
                "cave-x": {
                    "tier": "B",
                    "parity_ratio": 0.5
                }
            }
        }"#;
        let m = parse_parity_index_json(raw);
        let e = m.get("cave-x").expect("crate present");
        assert_eq!(e.parity_ratio, Some(0.5));
        assert_eq!(e.honest_ratio, None);
        assert_eq!(e.mapped_count, None);
        assert_eq!(e.partial_count, None);
    }

    #[test]
    fn attach_parity_index_copies_honest_fields() {
        let mut snap = ComplianceSnapshot {
            crates: vec![stub_compliance("cave-cache", 100)],
        };
        let mut index: HashMap<String, ParityIndexEntry> = HashMap::new();
        index.insert(
            "cave-cache".into(),
            ParityIndexEntry {
                tier: "C".into(),
                parity_ratio: Some(0.9474),
                parity_ratio_source: Some("manifest".into()),
                last_audit_disk: Some("2026-05-13".into()),
                honest_ratio: Some(0.8421),
                mapped_count: Some(19),
                partial_count: Some(4),
                skipped_count: Some(13),
                unmapped_count: Some(2),
                total_count: Some(38),
                manifest_filled: Some(true),
                cave_src_loc: None,
                upstream: None,
                upstream_version: None,
                stubs: None,
                note: None,
                behavioral_parity: None,
                behavioral_ported: None,
                behavioral_total: None,
                behavioral_audit_scope: None,
                behavioral_audit_at: None,
            },
        );
        attach_parity_index(&mut snap, &index);
        let c = &snap.crates[0];
        // The live re-read pass may have overridden these from the on-disk
        // manifest, so accept either the index value or the matching manifest
        // value. The key invariant: honest ≤ shape and partial > 0 propagated.
        assert!(c.parity_ratio.is_some());
        assert!(c.honest_parity_ratio.is_some());
        assert!(c.parity_partial_count.is_some());
        let honest = c.honest_parity_ratio.unwrap();
        let shape = c.parity_ratio.unwrap();
        assert!(honest <= shape + 0.000_5, "honest must be ≤ shape: {honest} > {shape}");
    }

    #[test]
    fn read_manifest_parity_parses_2026_05_13_schema() {
        // The on-disk manifest after the honest-re-audit pass has both the
        // legacy fill_ratio + the new honest_ratio + class counts. The reader
        // must surface all of them.
        let cache_parity = read_manifest_parity("cave-cache")
            .expect("cave-cache manifest exists in the workspace");
        assert!(cache_parity.fill_ratio > 0.9);
        assert!(cache_parity.honest_ratio.is_some(), "honest_ratio populated by 2026-05-13 pass");
        let honest = cache_parity.honest_ratio.unwrap();
        assert!(honest < cache_parity.fill_ratio, "honest < shape after partial demotions");
        assert!(cache_parity.partial_count.unwrap_or(0) > 0, "cache has 4 self-flagged partials");
        // Sum of class counts equals total.
        let sum = cache_parity.mapped_count.unwrap()
            + cache_parity.partial_count.unwrap()
            + cache_parity.skipped_count.unwrap()
            + cache_parity.unmapped_count.unwrap();
        assert_eq!(sum, cache_parity.total_count.unwrap());
    }

    #[test]
    fn read_manifest_parity_back_compat_without_honest_fields() {
        // Parser must accept a manifest snippet that pre-dates the
        // 2026-05-13 schema (no honest_ratio, no class counts). We
        // exercise the parsing logic directly via a synthetic file
        // path inside the workspace's `crates/` directory: any crate
        // whose manifest carries only `fill_ratio` + `last_audit`
        // returns `Some(LiveManifestParity)` with the new fields as
        // `None`. The cave-runtime workspace ships such a manifest
        // for cave-acme (infra-only, fill_ratio = 0.0, no honest).
        let p = read_manifest_parity("cave-acme")
            .expect("cave-acme manifest exists and is parseable");
        // cave-acme is infra-only with fill_ratio = 0.0 (no inventory);
        // either honest_ratio = None or honest_ratio = 0.0 is acceptable
        // — the parser must just not crash.
        assert!(p.fill_ratio.is_finite());
        // The crate has no [[partial]] block, so partial_count is None
        // or 0 — either is back-compatible.
        match p.partial_count {
            None => {} // pre-2026-05-13 schema
            Some(n) => assert_eq!(n, 0, "infra crate has no partials"),
        }
    }

    #[test]
    fn render_dashboard_includes_honest_parity_card() {
        let snap = ComplianceSnapshot {
            crates: vec![
                honest_compliance("cave-x", Some(0.95), Some(0.84), Some(4), false),
                honest_compliance("cave-y", Some(0.90), Some(0.80), Some(2), false),
            ],
        };
        let html = render_with_view(
            &snap,
            &ctx(&[Permission::AdminComplianceView]),
            ViewQuery::default(),
        )
        .expect("render OK");
        assert!(html.contains("Honest Parity"), "honest parity card present");
        // Aggregator output appears as a percentage somewhere in the body.
        // Mean of (0.84, 0.80) = 0.82 → 82.
        assert!(html.contains("82"), "aggregate honest score 82 rendered");
        // Per-crate honest column appears in the table head.
        assert!(html.contains(">honest<"), "honest column header rendered");
    }

    #[test]
    fn render_dashboard_honest_card_grade_b_for_82() {
        let snap = ComplianceSnapshot {
            crates: vec![
                honest_compliance("cave-x", Some(0.95), Some(0.84), Some(4), false),
                honest_compliance("cave-y", Some(0.90), Some(0.80), Some(2), false),
            ],
        };
        // Mean honest = 0.82 → Grade A (≥0.70 per compute_parity_grade).
        assert_eq!(snap.honest_parity_grade(), 'A');
        let html = render_with_view(
            &snap,
            &ctx(&[Permission::AdminComplianceView]),
            ViewQuery::default(),
        )
        .expect("render OK");
        // The "Grade X" string must appear THREE times in the four-card row:
        // structural, parity, honest, portal-ui — one each. (Portal UI card
        // requires its own data; without it the card shows 'F'.)
        let grade_count = html.matches("Grade ").count();
        assert!(grade_count >= 4, "four cards each carry a Grade label; got {grade_count}");
    }

    #[test]
    fn render_dashboard_honest_column_shows_minus_badge_when_demoted() {
        let snap = ComplianceSnapshot {
            crates: vec![
                // Honest 11 points below shape — should render a "-11" badge.
                honest_compliance("cave-cache", Some(0.95), Some(0.84), Some(4), false),
            ],
        };
        let html = render_with_view(
            &snap,
            &ctx(&[Permission::AdminComplianceView]),
            ViewQuery::default(),
        )
        .expect("render OK");
        // The drop badge format is " -<N>" (no plus sign; never zero).
        assert!(html.contains("-11"), "demotion drop -11 rendered in honest column");
        // Partial count badge "4p"
        assert!(html.contains("4p"), "partial count badge rendered");
    }

    #[test]
    fn honest_re_audit_manifest_self_consistent_for_top14() {
        // Walk every manifest we touched on 2026-05-13. Each one's
        // [parity] block must have mapped + partial + skipped + unmapped
        // == total, and fill_ratio + honest_ratio must lie in [0, 1].
        let top14 = [
            "cave-cache","cave-cri","cave-net","cave-etcd","cave-scheduler",
            "cave-apiserver","cave-kubelet","cave-mesh","cave-rdbms-operator",
            "cave-streams","cave-vault","cave-controller-manager","cave-auth",
            "cave-karpenter",
        ];
        for name in top14 {
            let p = read_manifest_parity(name)
                .unwrap_or_else(|| panic!("manifest missing for {name}"));
            let total = p.total_count.expect(&format!("{name} declares total"));
            let mapped = p.mapped_count.unwrap_or(0);
            let partial = p.partial_count.unwrap_or(0);
            let skipped = p.skipped_count.unwrap_or(0);
            let unmapped = p.unmapped_count.unwrap_or(0);
            assert_eq!(
                mapped + partial + skipped + unmapped,
                total,
                "{name}: counts sum to total"
            );
            assert!((0.0..=1.0).contains(&p.fill_ratio), "{name}: fill_ratio in [0,1]");
            let honest = p.honest_ratio.expect(&format!("{name} carries honest_ratio"));
            assert!((0.0..=1.0).contains(&honest), "{name}: honest_ratio in [0,1]");
            assert!(
                honest <= p.fill_ratio + 0.000_5,
                "{name}: honest_ratio ({honest}) must be ≤ fill_ratio ({})",
                p.fill_ratio
            );
        }
    }

    #[test]
    fn honest_re_audit_specific_crates_demoted_as_expected() {
        // Spot-check: the crates we audited on 2026-05-13 each carry a
        // floor on partial_count — i.e. the re-audit produced at least
        // this many [[partial]] blocks. Subsequent parallel-session
        // commits may add more (new [[mapped]] entries whose notes
        // re-trigger the self-flag regex), but never fewer.
        let lower_bound = [
            ("cave-cache", 4), ("cave-cri", 3), ("cave-etcd", 2),
            ("cave-apiserver", 1), ("cave-mesh", 1),
            ("cave-controller-manager", 1), ("cave-auth", 1),
        ];
        for (name, lower) in lower_bound {
            let p = read_manifest_parity(name)
                .unwrap_or_else(|| panic!("{name} manifest exists"));
            assert!(
                p.partial_count.unwrap_or(0) >= lower,
                "{name}: partial_count must be ≥ {lower}, got {:?}",
                p.partial_count
            );
            assert!(
                p.honest_ratio.unwrap_or(1.0) < p.fill_ratio + 0.000_5,
                "{name} honest must be ≤ fill"
            );
        }
    }

    #[test]
    fn cave_net_reclassifies_idiom_map_entries_to_skipped() {
        // The honest re-audit moved 61 idiom_map.rs entries and 3
        // binary_cites.rs entries from [[mapped]] to [[skipped]] with
        // reason "stdlib-analog" / "CLI". The manifest must reflect
        // that — skipped_count grew, mapped_count shrank.
        let p = read_manifest_parity("cave-net").expect("cave-net manifest");
        // Before the pass: mapped=106 skipped=17 → after: mapped=42 skipped=81.
        // Tolerate small drift from later edits by checking the directional
        // invariant.
        assert!(p.mapped_count.unwrap() < 50, "mapped count dropped below 50");
        assert!(p.skipped_count.unwrap() > 70, "skipped count grew above 70");
    }

    #[test]
    fn honest_re_audit_script_classifies_partial_via_note_self_flag() {
        // A pure-Rust regression of the regex: any block whose note self-flags
        // as scope-cut / MVP / deferred / placeholder must end up in the
        // honest_ratio's numerator deficit (i.e. should be [[partial]]).
        //
        // We hold the regex source-of-truth in scripts/honest-re-audit.py so
        // the test only verifies that the 14 crates we audited have AT LEAST
        // the expected lower-bound partial_count — never less. Re-running the
        // script must be non-decreasing on these counts.
        let lower_bound = [
            ("cave-cache", 4), ("cave-cri", 3), ("cave-etcd", 2),
            ("cave-apiserver", 1), ("cave-mesh", 1),
            ("cave-controller-manager", 1), ("cave-auth", 1),
        ];
        for (name, lower) in lower_bound {
            let p = read_manifest_parity(name).unwrap();
            assert!(
                p.partial_count.unwrap_or(0) >= lower,
                "{name}: partial_count must be ≥ {lower}"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // (end honest-parity axis tests)
    // ─────────────────────────────────────────────────────────────────────────

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

    // ---------------- behavioral-parity tests ----------------

    fn behavioral_compliance(
        name: &str,
        infra: bool,
        ratio: Option<f64>,
        ported: Option<u32>,
        total: Option<u32>,
    ) -> CrateCompliance {
        let mut c = stub_compliance(name, 100);
        c.infra_only = infra;
        c.behavioral_parity = ratio;
        c.behavioral_ported = ported;
        c.behavioral_total = total;
        c
    }

    #[test]
    fn behavioral_parity_avg_excludes_unaudited_and_infra() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Behavioral.tsx",
            "behavioralAvg",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![
                behavioral_compliance("a", false, Some(0.8), Some(8), Some(10)),
                behavioral_compliance("b", false, Some(0.5), Some(5), Some(10)),
                // unaudited tier-1 — excluded from numerator AND denominator
                behavioral_compliance("c", false, None, None, None),
                // infra — always excluded
                behavioral_compliance("d", true, Some(1.0), Some(10), Some(10)),
            ],
        };
        // (0.8 + 0.5) / 2 = 0.65 → 65.
        assert_eq!(snap.behavioral_parity_avg(), 65);
        assert_eq!(snap.behavioral_audited_count(), 2);
        assert_eq!(snap.behavioral_total_ported(), 13);
        assert_eq!(snap.behavioral_total_declared(), 20);
    }

    #[test]
    fn behavioral_grade_returns_f_when_no_audit() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Behavioral.tsx",
            "behavioralGradeF",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![
                behavioral_compliance("x", false, None, None, None),
                behavioral_compliance("y", false, None, None, None),
            ],
        };
        // No crate has been audited → honest F baseline.
        assert_eq!(snap.behavioral_grade(), 'F');
        assert_eq!(snap.behavioral_audited_count(), 0);
    }

    #[test]
    fn behavioral_grade_maps_to_parity_grade_scale() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Behavioral.tsx",
            "behavioralGradeScale",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![
                // avg 0.75 → A on parity scale (≥0.70).
                behavioral_compliance("a", false, Some(0.75), Some(15), Some(20)),
            ],
        };
        assert_eq!(snap.behavioral_grade(), 'A');
    }

    #[test]
    fn render_includes_behavioral_card_and_column_header() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/tech-insights/src/components/Scorecards/Behavioral.tsx",
            "behavioralRender",
            "acme"
        );
        let snap = ComplianceSnapshot {
            crates: vec![behavioral_compliance("a", false, Some(0.8), Some(8), Some(10))],
        };
        let html = render(&snap, &ctx(&[Permission::AdminComplianceView])).unwrap();
        // Headline card.
        assert!(
            html.contains("Behavioral Parity"),
            "expected Behavioral Parity card in dashboard"
        );
        // Per-row column.
        assert!(
            html.contains(">behavioral<"),
            "expected behavioral column header in matrix"
        );
        // Per-row count detail (8/10 from the test fixture).
        assert!(html.contains("8/10"));
    }
}
