//! `/admin/_audit` — Charter v2 per-crate matrix.
//!
//! Five-axis grade roll-up (see [`meta_audit`]) tells you HOW the
//! workspace is doing in aggregate. The Charter v2 matrix tells you
//! WHICH CRATE is keeping the workspace honest and which is dragging
//! it down, rule-by-rule. The eight rules (in canonical column order
//! on the card) are:
//!
//!   1. **TDD** — has tests, no `#[ignore = "impl pending"]` markers
//!   2. **SPDX** — every `.rs` file in `src/**` carries an
//!      `SPDX-License-Identifier:` header
//!   3. **source-stamp** — manifest `[upstream]` block has
//!      `org`, `repo`, `version` non-empty
//!   4. **no-stub** — `unimplemented!()` + `todo!()` counts both 0
//!   5. **no-backcompat** — no `#[deprecated]` attribute anywhere in
//!      `src/**`
//!   6. **always-latest** — `parity.manifest.toml::last_audit` is at
//!      most 90 days old
//!   7. **4-track** — Portal + cavectl + alerts + dashboard all present
//!   8. **honest** — manifest declares `honest_ratio` + at least one
//!      filled section
//!
//! Each rule resolves to PASS / FAIL / N/A per crate; the matrix view
//! is a sortable + filterable grid of those eight pills plus
//! fill_ratio / test count / last audit / manifest pin / last commit.

use crate::admin::compliance::{CommitRow, ComplianceSnapshot, CrateCompliance};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ── Charter v2 rule model ────────────────────────────────────────────

/// One of the eight Charter v2 rules. The variant order is canonical
/// — it matches the column order on the dashboard card and the array
/// indices into [`CrateCharter::verdicts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CharterRule {
    Tdd,
    Spdx,
    SourceStamp,
    NoStub,
    NoBackcompat,
    AlwaysLatest,
    FourTrack,
    Honest,
}

impl CharterRule {
    pub const ALL: [CharterRule; 8] = [
        CharterRule::Tdd,
        CharterRule::Spdx,
        CharterRule::SourceStamp,
        CharterRule::NoStub,
        CharterRule::NoBackcompat,
        CharterRule::AlwaysLatest,
        CharterRule::FourTrack,
        CharterRule::Honest,
    ];

    pub fn slug(&self) -> &'static str {
        // RED stub — every variant slugged the same so canonical-order
        // tests fail.
        "tdd"
    }

    pub fn label(&self) -> &'static str {
        "TDD"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    Na,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrateScan {
    pub src_rs_files: u32,
    pub spdx_marked_files: u32,
    pub deprecated_attr_count: u32,
}

impl CrateScan {
    pub fn empty() -> Self {
        Self {
            src_rs_files: 0,
            spdx_marked_files: 0,
            deprecated_attr_count: 0,
        }
    }
}

// ── Per-crate row ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrateCharter {
    pub name: String,
    pub infra_only: bool,
    pub fill_ratio: Option<f64>,
    pub fill_ratio_source: Option<String>,
    pub honest_ratio: Option<f64>,
    pub test_count: u32,
    pub last_audit: Option<String>,
    pub manifest_pin: Option<String>,
    pub last_commit: Option<CommitRow>,
    pub verdicts: [Verdict; 8],
}

impl CrateCharter {
    pub fn pass_count(&self) -> u32 {
        self.verdicts.iter().filter(|v| **v == Verdict::Pass).count() as u32
    }
    pub fn fail_count(&self) -> u32 {
        self.verdicts.iter().filter(|v| **v == Verdict::Fail).count() as u32
    }
    pub fn verdict_for(&self, _rule: CharterRule) -> Verdict {
        // RED stub.
        Verdict::Na
    }
}

// ── Filter / sort ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    All,
    AnyFailing,
    Tier1,
    InfraOnly,
}

impl FilterMode {
    pub fn parse(_s: &str) -> Self {
        // RED stub.
        FilterMode::All
    }
    pub fn slug(&self) -> &'static str {
        // RED stub.
        "all"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    Name,
    FillRatio,
    TestCount,
    LastAudit,
    PassCount,
}

impl SortKey {
    pub fn parse(_s: &str) -> Self {
        // RED stub.
        SortKey::Name
    }
    pub fn slug(&self) -> &'static str {
        // RED stub.
        "name"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharterMatrix {
    pub rows: Vec<CrateCharter>,
    pub total_crates: u32,
    pub crates_with_any_fail: u32,
    pub rule_pass_counts: [u32; 8],
    pub rule_fail_counts: [u32; 8],
    pub rule_na_counts: [u32; 8],
    pub filter: FilterMode,
    pub sort: SortKey,
}

// ── Public API (RED stubs) ───────────────────────────────────────────

pub fn evaluate(_c: &CrateCompliance, _scan: &CrateScan, _today: NaiveDate) -> [Verdict; 8] {
    // RED stub — always Na, so every rule-evaluation test fails.
    [Verdict::Na; 8]
}

pub fn build_matrix(
    _snap: &ComplianceSnapshot,
    _today: NaiveDate,
    _scans: &std::collections::BTreeMap<String, CrateScan>,
    _last_commits: &std::collections::BTreeMap<String, CommitRow>,
    filter: FilterMode,
    sort: SortKey,
) -> CharterMatrix {
    // RED stub — empty matrix; build/filter/sort tests will fail.
    CharterMatrix {
        rows: Vec::new(),
        total_crates: 0,
        crates_with_any_fail: 0,
        rule_pass_counts: [0; 8],
        rule_fail_counts: [0; 8],
        rule_na_counts: [0; 8],
        filter,
        sort,
    }
}

pub fn scan_crate_io(_workspace_root: &std::path::Path, _crate_name: &str) -> CrateScan {
    // RED stub — always empty.
    CrateScan::empty()
}

pub fn render_section(_m: &CharterMatrix, _tenant_id: &str) -> String {
    // RED stub — render placeholder so HTML-asserting tests fail.
    String::from("<section><!-- RED stub --></section>")
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::compliance::{ComplianceSnapshot, CrateCompliance};
    use std::collections::BTreeMap;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 18).unwrap()
    }

    fn base_crate(name: &str) -> CrateCompliance {
        CrateCompliance {
            name: name.into(),
            upstream_version: Some("v1.0.0".into()),
            upstream_org_repo: Some("acme/upstream".into()),
            backend_loc: 1000,
            backend_test_count: 20,
            ignored_test_count: 0,
            unimplemented_count: 0,
            todo_count: 0,
            portal_admin_present: true,
            cavectl_subcommand_present: true,
            obs_alerts_present: true,
            obs_dashboard_present: true,
            four_track_score: 100,
            infra_only: false,
            parity_ratio: Some(0.8),
            parity_ratio_source: Some("manifest".into()),
            parity_ratio_last_audit: Some("2026-05-15".into()),
            honest_parity_ratio: Some(0.75),
            parity_mapped_count: Some(10),
            parity_partial_count: Some(2),
            parity_skipped_count: Some(1),
            parity_unmapped_count: Some(0),
            parity_total_count: Some(13),
            manifest_filled: Some(true),
            audit_tier: Some("A".into()),
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

    fn full_scan(n_files: u32) -> CrateScan {
        CrateScan {
            src_rs_files: n_files,
            spdx_marked_files: n_files,
            deprecated_attr_count: 0,
        }
    }

    // ── rule evaluation: TDD ─────────────────────────────────────────

    #[test]
    fn tdd_passes_when_tests_present_and_no_ignored() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Pass, "TDD column");
    }

    #[test]
    fn tdd_fails_when_no_tests_in_tier_one_crate() {
        let mut c = base_crate("cave-x");
        c.backend_test_count = 0;
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Fail);
    }

    #[test]
    fn tdd_fails_when_ignored_tests_present() {
        let mut c = base_crate("cave-x");
        c.ignored_test_count = 1;
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Fail);
    }

    #[test]
    fn tdd_na_for_infra_only_without_tests() {
        let mut c = base_crate("cave-utils");
        c.infra_only = true;
        c.backend_test_count = 0;
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Na);
    }

    // ── rule evaluation: SPDX ─────────────────────────────────────────

    #[test]
    fn spdx_passes_when_every_file_carries_header() {
        let c = base_crate("cave-x");
        let scan = CrateScan {
            src_rs_files: 10,
            spdx_marked_files: 10,
            deprecated_attr_count: 0,
        };
        let v = evaluate(&c, &scan, today());
        assert_eq!(v[1], Verdict::Pass);
    }

    #[test]
    fn spdx_fails_with_any_file_missing_header() {
        let c = base_crate("cave-x");
        let scan = CrateScan {
            src_rs_files: 10,
            spdx_marked_files: 9,
            deprecated_attr_count: 0,
        };
        let v = evaluate(&c, &scan, today());
        assert_eq!(v[1], Verdict::Fail);
    }

    #[test]
    fn spdx_na_when_no_source_files() {
        let c = base_crate("cave-empty");
        let v = evaluate(&c, &CrateScan::empty(), today());
        assert_eq!(v[1], Verdict::Na);
    }

    // ── rule evaluation: source-stamp ─────────────────────────────────

    #[test]
    fn source_stamp_passes_with_full_upstream() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[2], Verdict::Pass);
    }

    #[test]
    fn source_stamp_fails_when_version_missing() {
        let mut c = base_crate("cave-x");
        c.upstream_version = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[2], Verdict::Fail);
    }

    #[test]
    fn source_stamp_na_for_infra_only() {
        let mut c = base_crate("cave-utils");
        c.infra_only = true;
        c.upstream_org_repo = None;
        c.upstream_version = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[2], Verdict::Na);
    }

    // ── rule evaluation: no-stub ─────────────────────────────────────

    #[test]
    fn no_stub_passes_when_zero_unimpl_and_todo() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[3], Verdict::Pass);
    }

    #[test]
    fn no_stub_fails_with_unimpl_present() {
        let mut c = base_crate("cave-x");
        c.unimplemented_count = 1;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[3], Verdict::Fail);
    }

    #[test]
    fn no_stub_fails_with_todo_present() {
        let mut c = base_crate("cave-x");
        c.todo_count = 1;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[3], Verdict::Fail);
    }

    // ── rule evaluation: no-backcompat ───────────────────────────────

    #[test]
    fn no_backcompat_passes_with_zero_deprecated_attrs() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[4], Verdict::Pass);
    }

    #[test]
    fn no_backcompat_fails_with_any_deprecated_attr() {
        let c = base_crate("cave-x");
        let scan = CrateScan {
            src_rs_files: 1,
            spdx_marked_files: 1,
            deprecated_attr_count: 1,
        };
        let v = evaluate(&c, &scan, today());
        assert_eq!(v[4], Verdict::Fail);
    }

    // ── rule evaluation: always-latest ───────────────────────────────

    #[test]
    fn always_latest_passes_when_audit_under_90_days() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = Some("2026-05-15".into());
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Pass);
    }

    #[test]
    fn always_latest_fails_when_audit_older_than_90_days() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = Some("2025-01-01".into());
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Fail);
    }

    #[test]
    fn always_latest_na_when_no_audit_date() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Na);
    }

    #[test]
    fn always_latest_fails_on_unparseable_date() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = Some("yesterday".into());
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Fail);
    }

    // ── rule evaluation: 4-track ─────────────────────────────────────

    #[test]
    fn four_track_passes_when_all_present() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[6], Verdict::Pass);
    }

    #[test]
    fn four_track_fails_when_any_track_missing() {
        let mut c = base_crate("cave-x");
        c.obs_alerts_present = false;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[6], Verdict::Fail);
    }

    #[test]
    fn four_track_na_for_infra_only() {
        let mut c = base_crate("cave-utils");
        c.infra_only = true;
        c.portal_admin_present = false;
        c.cavectl_subcommand_present = false;
        c.obs_alerts_present = false;
        c.obs_dashboard_present = false;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[6], Verdict::Na);
    }

    // ── rule evaluation: honest ──────────────────────────────────────

    #[test]
    fn honest_passes_with_honest_ratio_and_filled_manifest() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[7], Verdict::Pass);
    }

    #[test]
    fn honest_fails_when_manifest_empty() {
        let mut c = base_crate("cave-x");
        c.manifest_filled = Some(false);
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[7], Verdict::Fail);
    }

    #[test]
    fn honest_fails_when_honest_ratio_missing() {
        let mut c = base_crate("cave-x");
        c.honest_parity_ratio = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[7], Verdict::Fail);
    }

    // ── canonical ordering ────────────────────────────────────────────

    #[test]
    fn charter_rule_all_has_eight_entries_in_canonical_order() {
        let slugs: Vec<&str> = CharterRule::ALL.iter().map(|r| r.slug()).collect();
        assert_eq!(
            slugs,
            vec![
                "tdd",
                "spdx",
                "source_stamp",
                "no_stub",
                "no_backcompat",
                "always_latest",
                "four_track",
                "honest",
            ]
        );
    }

    #[test]
    fn crate_charter_pass_count_matches_verdict_array() {
        let mut c = base_crate("cave-x");
        c.unimplemented_count = 1;
        c.obs_alerts_present = false;
        let v = evaluate(&c, &full_scan(1), today());
        let row = CrateCharter {
            name: c.name.clone(),
            infra_only: c.infra_only,
            fill_ratio: c.parity_ratio,
            fill_ratio_source: c.parity_ratio_source.clone(),
            honest_ratio: c.honest_parity_ratio,
            test_count: c.backend_test_count,
            last_audit: c.parity_ratio_last_audit.clone(),
            manifest_pin: Some("acme/upstream @ v1.0.0".into()),
            last_commit: None,
            verdicts: v,
        };
        assert_eq!(row.fail_count(), 2);
        assert_eq!(row.pass_count(), 6);
        assert_eq!(row.verdict_for(CharterRule::NoStub), Verdict::Fail);
        assert_eq!(row.verdict_for(CharterRule::FourTrack), Verdict::Fail);
        assert_eq!(row.verdict_for(CharterRule::Tdd), Verdict::Pass);
    }

    // ── matrix build ─────────────────────────────────────────────────

    fn snap(crates: Vec<CrateCompliance>) -> ComplianceSnapshot {
        ComplianceSnapshot { crates }
    }

    fn empty_scans() -> BTreeMap<String, CrateScan> {
        BTreeMap::new()
    }
    fn empty_commits() -> BTreeMap<String, CommitRow> {
        BTreeMap::new()
    }

    #[test]
    fn build_matrix_produces_one_row_per_crate() {
        let s = snap(vec![base_crate("cave-a"), base_crate("cave-b"), base_crate("cave-c")]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        assert_eq!(m.total_crates, 3);
        assert_eq!(m.rows.len(), 3);
    }

    #[test]
    fn build_matrix_attaches_per_crate_scan_results_for_spdx() {
        let s = snap(vec![base_crate("cave-x")]);
        let mut scans = BTreeMap::new();
        scans.insert(
            "cave-x".into(),
            CrateScan {
                src_rs_files: 10,
                spdx_marked_files: 7,
                deprecated_attr_count: 0,
            },
        );
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.verdict_for(CharterRule::Spdx), Verdict::Fail);
    }

    #[test]
    fn build_matrix_carries_last_commit_when_available() {
        let s = snap(vec![base_crate("cave-x")]);
        let mut commits = BTreeMap::new();
        commits.insert(
            "cave-x".into(),
            CommitRow {
                sha: "deadbeef".into(),
                subject: "feat(cave-x): wire it up".into(),
            },
        );
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &commits,
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.last_commit.as_ref().unwrap().sha, "deadbeef");
    }

    #[test]
    fn build_matrix_carries_manifest_pin_from_upstream_fields() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.manifest_pin.as_deref(), Some("acme/upstream @ v1.0.0"));
    }

    #[test]
    fn build_matrix_carries_fill_ratio_and_test_count() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.fill_ratio, Some(0.8));
        assert_eq!(row.test_count, 20);
        assert_eq!(row.last_audit.as_deref(), Some("2026-05-15"));
    }

    #[test]
    fn build_matrix_records_rule_pass_counts() {
        let a = base_crate("cave-a");
        let mut b = base_crate("cave-b");
        b.unimplemented_count = 1;
        let mut c = base_crate("cave-c");
        c.obs_alerts_present = false;
        let s = snap(vec![a, b, c]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        assert_eq!(m.rule_pass_counts[3], 2, "no_stub passes for a + c");
        assert_eq!(m.rule_fail_counts[3], 1, "no_stub fails for b");
        assert_eq!(m.rule_pass_counts[6], 2, "four_track passes for a + b");
        assert_eq!(m.rule_fail_counts[6], 1, "four_track fails for c");
    }

    // ── filter / sort ────────────────────────────────────────────────

    #[test]
    fn filter_any_failing_drops_clean_crates() {
        let a = base_crate("cave-a");
        let mut b = base_crate("cave-b");
        b.unimplemented_count = 1;
        let s = snap(vec![a, b]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::AnyFailing,
            SortKey::Name,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-b"]);
        assert_eq!(m.total_crates, 2);
    }

    #[test]
    fn filter_infra_only_keeps_infra_drops_tier1() {
        let a = base_crate("cave-tier1");
        let mut b = base_crate("cave-utils");
        b.infra_only = true;
        let s = snap(vec![a, b]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::InfraOnly,
            SortKey::Name,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-utils"]);
    }

    #[test]
    fn filter_tier1_drops_infra() {
        let a = base_crate("cave-tier1");
        let mut b = base_crate("cave-utils");
        b.infra_only = true;
        let s = snap(vec![a, b]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::Tier1,
            SortKey::Name,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-tier1"]);
    }

    #[test]
    fn sort_fill_ratio_orders_higher_first_none_last() {
        let mut a = base_crate("cave-low");
        a.parity_ratio = Some(0.2);
        let mut b = base_crate("cave-high");
        b.parity_ratio = Some(0.9);
        let mut c = base_crate("cave-none");
        c.parity_ratio = None;
        let s = snap(vec![a, b, c]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::FillRatio,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-high", "cave-low", "cave-none"]);
    }

    #[test]
    fn sort_pass_count_orders_clean_first() {
        let a = base_crate("cave-clean");
        let mut b = base_crate("cave-broken");
        b.unimplemented_count = 1;
        b.obs_alerts_present = false;
        b.honest_parity_ratio = None;
        let s = snap(vec![b, a]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::PassCount,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-clean", "cave-broken"]);
    }

    #[test]
    fn sort_last_audit_orders_newest_first() {
        let mut a = base_crate("cave-old");
        a.parity_ratio_last_audit = Some("2025-01-01".into());
        let mut b = base_crate("cave-new");
        b.parity_ratio_last_audit = Some("2026-05-10".into());
        let s = snap(vec![a, b]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::LastAudit,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-new", "cave-old"]);
    }

    #[test]
    fn filter_mode_parse_handles_aliases() {
        assert_eq!(FilterMode::parse("failing"), FilterMode::AnyFailing);
        assert_eq!(FilterMode::parse("any_failing"), FilterMode::AnyFailing);
        assert_eq!(FilterMode::parse("tier1"), FilterMode::Tier1);
        assert_eq!(FilterMode::parse("infra"), FilterMode::InfraOnly);
        assert_eq!(FilterMode::parse("infra_only"), FilterMode::InfraOnly);
        assert_eq!(FilterMode::parse("nonsense"), FilterMode::All);
    }

    #[test]
    fn sort_key_parse_handles_aliases() {
        assert_eq!(SortKey::parse("ratio"), SortKey::FillRatio);
        assert_eq!(SortKey::parse("tests"), SortKey::TestCount);
        assert_eq!(SortKey::parse("audit"), SortKey::LastAudit);
        assert_eq!(SortKey::parse("passes"), SortKey::PassCount);
        assert_eq!(SortKey::parse("nonsense"), SortKey::Name);
    }

    // ── render ───────────────────────────────────────────────────────

    #[test]
    fn render_section_includes_heading_and_table() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("Charter v2 matrix"));
        assert!(html.contains("<table"));
        assert!(html.contains("cave-x"));
    }

    #[test]
    fn render_section_includes_eight_rule_pills_in_header() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        for rule in CharterRule::ALL.iter() {
            assert!(
                html.contains(rule.label()),
                "header pill for {} missing",
                rule.label()
            );
        }
    }

    #[test]
    fn render_section_emits_per_rule_pass_fail_pills_on_row() {
        let mut bad = base_crate("cave-bad");
        bad.unimplemented_count = 1;
        let s = snap(vec![bad]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("FAIL"));
        assert!(html.contains("PASS"));
    }

    #[test]
    fn render_section_links_to_filter_and_sort_variants_preserving_tenant() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("tenant_id=acme"));
        assert!(html.contains("filter=any_failing"));
        assert!(html.contains("sort=fill_ratio"));
    }

    #[test]
    fn render_section_shows_empty_state_when_filter_excludes_everything() {
        let s = snap(vec![base_crate("cave-clean")]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::AnyFailing,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("No crates match"));
    }

    // ── scan_crate_io smoke ──────────────────────────────────────────

    #[test]
    fn scan_crate_io_returns_empty_for_missing_crate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = scan_crate_io(tmp.path(), "does-not-exist");
        assert_eq!(s, CrateScan::empty());
    }

    #[test]
    fn scan_crate_io_counts_spdx_and_deprecated() {
        let tmp = tempfile::TempDir::new().unwrap();
        let crate_dir = tmp.path().join("crates/cave-y/src");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(
            crate_dir.join("a.rs"),
            "// SPDX-License-Identifier: AGPL-3.0-or-later\n// Copyright (C) 2026\npub fn ok() {}\n",
        )
        .unwrap();
        std::fs::write(
            crate_dir.join("b.rs"),
            "// no header here\n#[deprecated]\npub fn old() {}\n",
        )
        .unwrap();
        let s = scan_crate_io(tmp.path(), "cave-y");
        assert_eq!(s.src_rs_files, 2);
        assert_eq!(s.spdx_marked_files, 1);
        assert_eq!(s.deprecated_attr_count, 1);
    }

    // ── JSON ─────────────────────────────────────────────────────────

    #[test]
    fn charter_matrix_serialises_to_json() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let j = serde_json::to_value(&m).unwrap();
        assert!(j["rows"].is_array());
        assert_eq!(j["rows"].as_array().unwrap().len(), 1);
        assert!(j["rule_pass_counts"].is_array());
        assert_eq!(j["rule_pass_counts"].as_array().unwrap().len(), 8);
        assert!(j["total_crates"].is_number());
    }
}
