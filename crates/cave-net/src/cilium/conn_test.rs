// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Connectivity test framework — `cilium connectivity test` model.
//!
//! Mirrors `cilium-cli/connectivity/check/check.go` and the result
//! aggregator in `cilium-cli/connectivity/check/test.go`. Each test
//! describes a source/destination pair, a protocol scenario, and the
//! expected verdict; the runner records actuals and the aggregator
//! reports pass/fail counts.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TestKind {
    PodToPod,
    PodToService,
    PodToHost,
    PodToWorld,
    PodToL7Http,
    PodToL7Dns,
    HostToPod,
}

impl TestKind {
    pub fn label(self) -> &'static str {
        match self {
            TestKind::PodToPod => "pod-to-pod",
            TestKind::PodToService => "pod-to-service",
            TestKind::PodToHost => "pod-to-host",
            TestKind::PodToWorld => "pod-to-world",
            TestKind::PodToL7Http => "pod-to-l7-http",
            TestKind::PodToL7Dns => "pod-to-l7-dns",
            TestKind::HostToPod => "host-to-pod",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpectedVerdict {
    Allow,
    Deny,
    /// Some scenarios are inherently flaky; the runner records the
    /// outcome but doesn't count failures against the suite.
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActualOutcome {
    Allowed { duration_ms: u32 },
    Denied,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Test {
    pub name: String,
    pub kind: TestKind,
    pub source: String,
    pub destination: String,
    pub expected: ExpectedVerdict,
    pub actual: Option<ActualOutcome>,
}

impl Test {
    pub fn passed(&self) -> Option<bool> {
        let actual = self.actual?;
        Some(matches!(
            (self.expected, actual),
            (ExpectedVerdict::Allow, ActualOutcome::Allowed { .. })
                | (ExpectedVerdict::Deny, ActualOutcome::Denied)
                | (ExpectedVerdict::Skip, _)
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteReport {
    pub total: u64,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub by_kind: BTreeMap<String, u64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConnTestError {
    #[error("test `{0}` not found")]
    NotFound(String),
    #[error("test `{0}` already exists")]
    Duplicate(String),
    #[error("tenant {tenant} cannot mutate suite owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ConnectivitySuite {
    pub tenant: TenantId,
    pub name: String,
    tests: BTreeMap<String, Test>,
}

impl ConnectivitySuite {
    pub fn new(tenant: TenantId, name: impl Into<String>) -> Self {
        Self {
            tenant,
            name: name.into(),
            tests: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, test: Test) -> Result<(), ConnTestError> {
        if self.tests.contains_key(&test.name) {
            return Err(ConnTestError::Duplicate(test.name));
        }
        self.tests.insert(test.name.clone(), test);
        Ok(())
    }

    pub fn record(&mut self, name: &str, actual: ActualOutcome) -> Result<(), ConnTestError> {
        let t = self
            .tests
            .get_mut(name)
            .ok_or_else(|| ConnTestError::NotFound(name.to_string()))?;
        t.actual = Some(actual);
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&Test> {
        self.tests.get(name)
    }

    pub fn total(&self) -> usize {
        self.tests.len()
    }

    pub fn report(&self) -> SuiteReport {
        let mut report = SuiteReport {
            total: self.tests.len() as u64,
            passed: 0,
            failed: 0,
            skipped: 0,
            by_kind: BTreeMap::new(),
        };
        for t in self.tests.values() {
            *report
                .by_kind
                .entry(t.kind.label().to_string())
                .or_insert(0) += 1;
            if matches!(t.expected, ExpectedVerdict::Skip) {
                report.skipped += 1;
                continue;
            }
            match t.passed() {
                Some(true) => report.passed += 1,
                Some(false) => report.failed += 1,
                None => report.skipped += 1, // unrun
            }
        }
        report
    }

    pub fn unrun_tests(&self) -> Vec<&Test> {
        self.tests
            .values()
            .filter(|t| t.actual.is_none() && !matches!(t.expected, ExpectedVerdict::Skip))
            .collect()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("cilium-cli/connectivity/check/check.go", "Suite");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn t(name: &str, kind: TestKind, expected: ExpectedVerdict) -> Test {
        Test {
            name: name.into(),
            kind,
            source: "ns/client".into(),
            destination: "ns/server".into(),
            expected,
            actual: None,
        }
    }

    fn suite(tenant: TenantId) -> ConnectivitySuite {
        ConnectivitySuite::new(tenant, "default-suite")
    }

    // ── TestKind ───────────────────────────────────────────────────────────

    #[test]
    fn test_kind_labels_match_cli_output() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Kind.Label",
            "tenant-ct-l"
        );
        assert_eq!(TestKind::PodToPod.label(), "pod-to-pod");
        assert_eq!(TestKind::PodToService.label(), "pod-to-service");
        assert_eq!(TestKind::PodToWorld.label(), "pod-to-world");
        assert_eq!(TestKind::PodToL7Http.label(), "pod-to-l7-http");
    }

    // ── Add / record ───────────────────────────────────────────────────────

    #[test]
    fn add_test_records_in_suite() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Add",
            "tenant-ct-a"
        );
        let mut s = suite(tenant);
        s.add(t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        assert_eq!(s.total(), 1);
    }

    #[test]
    fn add_duplicate_test_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Add.Duplicate",
            "tenant-ct-d"
        );
        let mut s = suite(tenant);
        s.add(t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        let err = s
            .add(t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap_err();
        assert!(matches!(err, ConnTestError::Duplicate(_)));
    }

    #[test]
    fn record_unknown_test_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Record.NotFound",
            "tenant-ct-rnf"
        );
        let mut s = suite(tenant);
        let err = s.record("ghost", ActualOutcome::Denied).unwrap_err();
        assert!(matches!(err, ConnTestError::NotFound(_)));
    }

    #[test]
    fn record_actual_outcome() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Record",
            "tenant-ct-r"
        );
        let mut s = suite(tenant);
        s.add(t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.record("test-1", ActualOutcome::Allowed { duration_ms: 5 })
            .unwrap();
        assert!(s.lookup("test-1").unwrap().actual.is_some());
    }

    // ── passed() ───────────────────────────────────────────────────────────

    #[test]
    fn passed_true_when_allow_meets_allowed() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.AllowAllowed",
            "tenant-ct-pa"
        );
        let mut tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow);
        tt.actual = Some(ActualOutcome::Allowed { duration_ms: 5 });
        assert_eq!(tt.passed(), Some(true));
    }

    #[test]
    fn passed_false_when_allow_meets_denied() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.AllowDenied",
            "tenant-ct-pd"
        );
        let mut tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow);
        tt.actual = Some(ActualOutcome::Denied);
        assert_eq!(tt.passed(), Some(false));
    }

    #[test]
    fn passed_true_when_deny_meets_denied() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.DenyDenied",
            "tenant-ct-pdd"
        );
        let mut tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Deny);
        tt.actual = Some(ActualOutcome::Denied);
        assert_eq!(tt.passed(), Some(true));
    }

    #[test]
    fn passed_false_when_deny_meets_allowed() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.DenyAllowed",
            "tenant-ct-pda"
        );
        let mut tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Deny);
        tt.actual = Some(ActualOutcome::Allowed { duration_ms: 5 });
        assert_eq!(tt.passed(), Some(false));
    }

    #[test]
    fn passed_returns_true_for_skip_regardless_of_outcome() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.SkipAlways",
            "tenant-ct-ps"
        );
        let mut tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Skip);
        tt.actual = Some(ActualOutcome::Denied);
        assert_eq!(tt.passed(), Some(true));
    }

    #[test]
    fn passed_returns_none_when_unrun() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.Unrun",
            "tenant-ct-pu"
        );
        let tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow);
        assert!(tt.passed().is_none());
    }

    #[test]
    fn passed_false_on_error_outcome() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Passed.Error",
            "tenant-ct-pe"
        );
        let mut tt = t("test-1", TestKind::PodToPod, ExpectedVerdict::Allow);
        tt.actual = Some(ActualOutcome::Error);
        assert_eq!(tt.passed(), Some(false));
    }

    // ── report() ───────────────────────────────────────────────────────────

    #[test]
    fn report_all_passed() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Report.AllPassed",
            "tenant-ct-rap"
        );
        let mut s = suite(tenant);
        for i in 0..3u8 {
            s.add(t(
                &format!("t-{i}"),
                TestKind::PodToPod,
                ExpectedVerdict::Allow,
            ))
            .unwrap();
            s.record(&format!("t-{i}"), ActualOutcome::Allowed { duration_ms: 5 })
                .unwrap();
        }
        let r = s.report();
        assert_eq!(r.total, 3);
        assert_eq!(r.passed, 3);
        assert_eq!(r.failed, 0);
    }

    #[test]
    fn report_mixed_outcome() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Report.Mixed",
            "tenant-ct-rm"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.add(t("b", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.add(t("c", TestKind::PodToPod, ExpectedVerdict::Deny))
            .unwrap();
        s.record("a", ActualOutcome::Allowed { duration_ms: 5 })
            .unwrap();
        s.record("b", ActualOutcome::Denied).unwrap(); // wrong
        s.record("c", ActualOutcome::Denied).unwrap();
        let r = s.report();
        assert_eq!(r.total, 3);
        assert_eq!(r.passed, 2);
        assert_eq!(r.failed, 1);
    }

    #[test]
    fn report_unrun_count_as_skipped() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Report.Unrun",
            "tenant-ct-ru"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        let r = s.report();
        assert_eq!(r.skipped, 1);
        assert_eq!(r.passed, 0);
        assert_eq!(r.failed, 0);
    }

    #[test]
    fn report_skip_expected_counted_as_skipped() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Report.SkipExpected",
            "tenant-ct-rse"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Skip))
            .unwrap();
        let r = s.report();
        assert_eq!(r.skipped, 1);
    }

    #[test]
    fn report_by_kind_breakdown() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Report.ByKind",
            "tenant-ct-rbk"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.add(t("b", TestKind::PodToService, ExpectedVerdict::Allow))
            .unwrap();
        s.add(t("c", TestKind::PodToService, ExpectedVerdict::Allow))
            .unwrap();
        let r = s.report();
        assert_eq!(*r.by_kind.get("pod-to-pod").unwrap(), 1);
        assert_eq!(*r.by_kind.get("pod-to-service").unwrap(), 2);
    }

    // ── unrun_tests ────────────────────────────────────────────────────────

    #[test]
    fn unrun_tests_returns_unrecorded_non_skip() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Unrun",
            "tenant-ct-u"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.add(t("b", TestKind::PodToPod, ExpectedVerdict::Skip))
            .unwrap();
        s.add(t("c", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.record("a", ActualOutcome::Allowed { duration_ms: 5 })
            .unwrap();
        let unrun = s.unrun_tests();
        assert_eq!(unrun.len(), 1);
        assert_eq!(unrun[0].name, "c");
    }

    #[test]
    fn unrun_tests_excludes_skip_expected() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Unrun.SkipExcluded",
            "tenant-ct-use"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Skip))
            .unwrap();
        let unrun = s.unrun_tests();
        assert!(unrun.is_empty());
    }

    // ── Total ──────────────────────────────────────────────────────────────

    #[test]
    fn total_tracks_added_tests() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Total",
            "tenant-ct-tot"
        );
        let mut s = suite(tenant);
        for i in 0..5u8 {
            s.add(t(
                &format!("t-{i}"),
                TestKind::PodToPod,
                ExpectedVerdict::Allow,
            ))
            .unwrap();
        }
        assert_eq!(s.total(), 5);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn test_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Test.Serde",
            "tenant-ct-tserde"
        );
        let mut tt = t("test-1", TestKind::PodToL7Http, ExpectedVerdict::Allow);
        tt.actual = Some(ActualOutcome::Allowed { duration_ms: 5 });
        let s = serde_json::to_string(&tt).unwrap();
        let back: Test = serde_json::from_str(&s).unwrap();
        assert_eq!(back, tt);
    }

    #[test]
    fn report_serde_round_trip() {
        let (_c, tenant) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/test.go",
            "Report.Serde",
            "tenant-ct-rserde"
        );
        let mut s = suite(tenant);
        s.add(t("a", TestKind::PodToPod, ExpectedVerdict::Allow))
            .unwrap();
        s.record("a", ActualOutcome::Allowed { duration_ms: 5 })
            .unwrap();
        let r = s.report();
        let json = serde_json::to_string(&r).unwrap();
        let back: SuiteReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn outcome_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Outcome.Serde",
            "tenant-ct-oserde"
        );
        for o in [
            ActualOutcome::Allowed { duration_ms: 10 },
            ActualOutcome::Denied,
            ActualOutcome::Error,
        ] {
            let s = serde_json::to_string(&o).unwrap();
            let back: ActualOutcome = serde_json::from_str(&s).unwrap();
            assert_eq!(back, o);
        }
    }

    #[test]
    fn expected_verdict_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "cilium-cli/connectivity/check/check.go",
            "Verdict.Serde",
            "tenant-ct-vserde"
        );
        for v in [
            ExpectedVerdict::Allow,
            ExpectedVerdict::Deny,
            ExpectedVerdict::Skip,
        ] {
            let s = serde_json::to_string(&v).unwrap();
            let back: ExpectedVerdict = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }
}
