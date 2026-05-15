// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo CD wrap — advanced features.
//!
//! Layered on top of [`super::argocd`]: this module models the bits the
//! native portal needs that are *not* in the basic deployment timeline view —
//! health checks, sync windows, manual-sync specs, and app-of-apps trees.
//!
//! All surface-level logic lives here so the plumbing in `argocd.rs` keeps
//! its narrow shape (one Application + history). Together they replace the
//! Argo CD UI in full.

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    Pass,
    Warn,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    pub kind: String,        // e.g. "Deployment", "StatefulSet", "Service"
    pub name: String,
    pub namespace: String,
    pub result: CheckResult,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncWindowKind {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncWindow {
    pub kind: SyncWindowKind,
    pub schedule_cron: String,
    pub duration_min: u32,
    pub apps: Vec<String>,
    pub manual_sync: bool,
    pub timezone: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdvancedError {
    #[error("invalid cron: {0}")]
    InvalidCron(String),
    #[error("unknown app: {0}")]
    UnknownApp(String),
    #[error("invalid duration {0}")]
    InvalidDuration(u32),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("cycle detected at {0:?}")]
    Cycle(String),
    #[error("invalid timezone: {0}")]
    InvalidTimezone(String),
}

const VALID_CRON_FIELDS: usize = 5;

/// A *very* lax cron validator — five whitespace-separated fields, each
/// containing only digits, `*`, `/`, `,`, or `-`. Sufficient to reject
/// obvious mistakes; cave-pipelines owns the full grammar.
fn is_valid_cron(s: &str) -> bool {
    let fields: Vec<&str> = s.split_whitespace().collect();
    if fields.len() != VALID_CRON_FIELDS {
        return false;
    }
    fields.iter().all(|f| {
        !f.is_empty()
            && f.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '*' | '/' | ',' | '-'))
    })
}

/// IANA-style timezone validator — accepts `UTC` or `Continent/City` form
/// with letters and underscores.
fn is_valid_tz(s: &str) -> bool {
    if s == "UTC" {
        return true;
    }
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    let alpha = |s: &&str| {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
    };
    alpha(&parts[0]) && alpha(&parts[1])
}

impl SyncWindow {
    pub fn new(
        kind: SyncWindowKind,
        cron: impl Into<String>,
        duration_min: u32,
    ) -> Result<Self, AdvancedError> {
        let cron: String = cron.into();
        if !is_valid_cron(&cron) {
            return Err(AdvancedError::InvalidCron(cron));
        }
        if duration_min == 0 || duration_min > 1440 {
            return Err(AdvancedError::InvalidDuration(duration_min));
        }
        Ok(Self {
            kind,
            schedule_cron: cron,
            duration_min,
            apps: Vec::new(),
            manual_sync: false,
            timezone: "UTC".into(),
        })
    }

    pub fn with_apps<I, S>(mut self, apps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.apps = apps.into_iter().map(Into::into).collect();
        self
    }

    pub fn allow_manual(mut self, allow: bool) -> Self {
        self.manual_sync = allow;
        self
    }

    pub fn with_timezone(mut self, tz: impl Into<String>) -> Result<Self, AdvancedError> {
        let tz = tz.into();
        if !is_valid_tz(&tz) {
            return Err(AdvancedError::InvalidTimezone(tz));
        }
        self.timezone = tz;
        Ok(self)
    }

    /// Apps applies to all, or to the listed apps.
    pub fn applies_to(&self, app: &str) -> bool {
        self.apps.is_empty() || self.apps.iter().any(|a| a == app || a == "*")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppOfAppsNode {
    pub name: String,
    pub tenant: String,
    pub children: Vec<String>,
}

#[derive(Debug, Default)]
pub struct AppOfAppsTree {
    nodes: Vec<AppOfAppsNode>,
}

impl AppOfAppsTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, node: AppOfAppsNode) -> Result<(), AdvancedError> {
        // Cycle check: BFS from each child, ensure we don't reach self.
        for child in &node.children {
            if Self::would_form_cycle(&self.nodes, &node.name, child) {
                return Err(AdvancedError::Cycle(node.name));
            }
        }
        if let Some(idx) = self
            .nodes
            .iter()
            .position(|n| n.tenant == node.tenant && n.name == node.name)
        {
            self.nodes[idx] = node;
        } else {
            self.nodes.push(node);
        }
        Ok(())
    }

    fn would_form_cycle(nodes: &[AppOfAppsNode], parent: &str, child: &str) -> bool {
        if parent == child {
            return true;
        }
        let mut stack = vec![child.to_string()];
        let mut visited = std::collections::HashSet::new();
        while let Some(cur) = stack.pop() {
            if cur == parent {
                return true;
            }
            if !visited.insert(cur.clone()) {
                continue;
            }
            if let Some(n) = nodes.iter().find(|n| n.name == cur) {
                stack.extend(n.children.iter().cloned());
            }
        }
        false
    }

    pub fn descendants(&self, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack: Vec<String> = self
            .nodes
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        let mut visited = std::collections::HashSet::new();
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            out.push(cur.clone());
            if let Some(n) = self.nodes.iter().find(|n| n.name == cur) {
                stack.extend(n.children.iter().cloned());
            }
        }
        out.sort();
        out
    }

    pub fn roots(&self, tenant: &str) -> Vec<&AppOfAppsNode> {
        let referenced: std::collections::HashSet<&str> = self
            .nodes
            .iter()
            .filter(|n| n.tenant == tenant)
            .flat_map(|n| n.children.iter().map(String::as_str))
            .collect();
        let mut out: Vec<&AppOfAppsNode> = self
            .nodes
            .iter()
            .filter(|n| n.tenant == tenant && !referenced.contains(n.name.as_str()))
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn count(&self) -> usize {
        self.nodes.len()
    }
}

/// Aggregated health snapshot — folded across [`HealthCheck`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub passing: u32,
    pub warning: u32,
    pub failing: u32,
    pub unknown: u32,
}

impl HealthSnapshot {
    pub fn aggregate(checks: &[HealthCheck]) -> Self {
        let mut h = Self {
            passing: 0,
            warning: 0,
            failing: 0,
            unknown: 0,
        };
        for c in checks {
            match c.result {
                CheckResult::Pass => h.passing += 1,
                CheckResult::Warn => h.warning += 1,
                CheckResult::Fail => h.failing += 1,
                CheckResult::Unknown => h.unknown += 1,
            }
        }
        h
    }

    pub fn overall(&self) -> CheckResult {
        if self.failing > 0 {
            CheckResult::Fail
        } else if self.warning > 0 {
            CheckResult::Warn
        } else if self.passing > 0 {
            CheckResult::Pass
        } else {
            CheckResult::Unknown
        }
    }

    pub fn health_score(&self) -> f64 {
        let total = self.passing + self.warning + self.failing + self.unknown;
        if total == 0 {
            return 0.0;
        }
        let score = self.passing as f64 + 0.5 * self.warning as f64;
        score / total as f64
    }
}

/// Manual-sync request (plumbing replacement for the Argo "Sync" button).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualSyncSpec {
    pub tenant: String,
    pub app: String,
    pub revision: Option<String>,
    pub prune: bool,
    pub dry_run: bool,
    pub strategy: SyncStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStrategy {
    Apply,
    Hook,
    Force,
}

impl ManualSyncSpec {
    pub fn allowed_for(&self, persona: ViewPersona) -> bool {
        // Force is admin-only; tenant + operator can apply/hook.
        match self.strategy {
            SyncStrategy::Force => persona == ViewPersona::Admin,
            _ => matches!(
                persona,
                ViewPersona::Tenant | ViewPersona::Operator | ViewPersona::Admin
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(kind: &str, name: &str, r: CheckResult) -> HealthCheck {
        HealthCheck {
            kind: kind.into(),
            name: name.into(),
            namespace: "default".into(),
            result: r,
            message: String::new(),
        }
    }

    #[test]
    fn cron_valid_simple() {
        assert!(is_valid_cron("* * * * *"));
        assert!(is_valid_cron("0 0 * * 0"));
        assert!(is_valid_cron("*/5 9-17 * * 1-5"));
    }

    #[test]
    fn cron_invalid_field_count() {
        assert!(!is_valid_cron("* * * *"));
        assert!(!is_valid_cron("* * * * * *"));
        assert!(!is_valid_cron(""));
    }

    #[test]
    fn cron_invalid_chars() {
        assert!(!is_valid_cron("a b c d e"));
        assert!(!is_valid_cron("* * * * @"));
    }

    #[test]
    fn tz_valid_utc_and_pairs() {
        assert!(is_valid_tz("UTC"));
        assert!(is_valid_tz("Europe/Istanbul"));
        assert!(is_valid_tz("America/New_York"));
    }

    #[test]
    fn tz_invalid() {
        assert!(!is_valid_tz("Europe"));
        assert!(!is_valid_tz("a/b/c"));
        assert!(!is_valid_tz("Eu rope/Istanbul"));
    }

    #[test]
    fn sync_window_new_invalid_cron() {
        let err = SyncWindow::new(SyncWindowKind::Allow, "bad", 30).unwrap_err();
        assert!(matches!(err, AdvancedError::InvalidCron(_)));
    }

    #[test]
    fn sync_window_new_zero_duration_rejected() {
        let err = SyncWindow::new(SyncWindowKind::Allow, "* * * * *", 0).unwrap_err();
        assert!(matches!(err, AdvancedError::InvalidDuration(0)));
    }

    #[test]
    fn sync_window_new_huge_duration_rejected() {
        let err = SyncWindow::new(SyncWindowKind::Allow, "* * * * *", 5000).unwrap_err();
        assert!(matches!(err, AdvancedError::InvalidDuration(5000)));
    }

    #[test]
    fn sync_window_with_apps_filters() {
        let w = SyncWindow::new(SyncWindowKind::Deny, "* * * * *", 60)
            .unwrap()
            .with_apps(["web", "api"]);
        assert!(w.applies_to("web"));
        assert!(w.applies_to("api"));
        assert!(!w.applies_to("db"));
    }

    #[test]
    fn sync_window_empty_apps_applies_all() {
        let w = SyncWindow::new(SyncWindowKind::Deny, "* * * * *", 60).unwrap();
        assert!(w.applies_to("anything"));
    }

    #[test]
    fn sync_window_wildcard_apps_applies_all() {
        let w = SyncWindow::new(SyncWindowKind::Deny, "* * * * *", 60)
            .unwrap()
            .with_apps(["*"]);
        assert!(w.applies_to("anything"));
    }

    #[test]
    fn sync_window_allow_manual() {
        let w = SyncWindow::new(SyncWindowKind::Deny, "* * * * *", 60)
            .unwrap()
            .allow_manual(true);
        assert!(w.manual_sync);
    }

    #[test]
    fn sync_window_with_timezone() {
        let w = SyncWindow::new(SyncWindowKind::Allow, "* * * * *", 60)
            .unwrap()
            .with_timezone("Europe/Istanbul")
            .unwrap();
        assert_eq!(w.timezone, "Europe/Istanbul");
    }

    #[test]
    fn sync_window_invalid_timezone() {
        let err = SyncWindow::new(SyncWindowKind::Allow, "* * * * *", 60)
            .unwrap()
            .with_timezone("not_a_zone")
            .unwrap_err();
        assert!(matches!(err, AdvancedError::InvalidTimezone(_)));
    }

    #[test]
    fn app_of_apps_upsert_inserts() {
        let mut t = AppOfAppsTree::new();
        t.upsert(AppOfAppsNode {
            name: "root".into(),
            tenant: "acme".into(),
            children: vec!["a".into()],
        })
        .unwrap();
        assert_eq!(t.count(), 1);
    }

    #[test]
    fn app_of_apps_self_reference_is_cycle() {
        let mut t = AppOfAppsTree::new();
        let err = t
            .upsert(AppOfAppsNode {
                name: "x".into(),
                tenant: "acme".into(),
                children: vec!["x".into()],
            })
            .unwrap_err();
        assert!(matches!(err, AdvancedError::Cycle(_)));
    }

    #[test]
    fn app_of_apps_indirect_cycle_rejected() {
        let mut t = AppOfAppsTree::new();
        t.upsert(AppOfAppsNode {
            name: "a".into(),
            tenant: "acme".into(),
            children: vec!["b".into()],
        })
        .unwrap();
        t.upsert(AppOfAppsNode {
            name: "b".into(),
            tenant: "acme".into(),
            children: vec!["c".into()],
        })
        .unwrap();
        // a -> b -> c, so making c -> a would form cycle
        let err = t
            .upsert(AppOfAppsNode {
                name: "c".into(),
                tenant: "acme".into(),
                children: vec!["a".into()],
            })
            .unwrap_err();
        assert!(matches!(err, AdvancedError::Cycle(_)));
    }

    #[test]
    fn app_of_apps_descendants_collects_transitively() {
        let mut t = AppOfAppsTree::new();
        t.upsert(AppOfAppsNode { name: "root".into(), tenant: "acme".into(), children: vec!["a".into(), "b".into()] }).unwrap();
        t.upsert(AppOfAppsNode { name: "a".into(), tenant: "acme".into(), children: vec!["c".into()] }).unwrap();
        t.upsert(AppOfAppsNode { name: "b".into(), tenant: "acme".into(), children: vec![] }).unwrap();
        t.upsert(AppOfAppsNode { name: "c".into(), tenant: "acme".into(), children: vec![] }).unwrap();
        let d = t.descendants("root");
        assert_eq!(d, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn app_of_apps_descendants_unknown_returns_empty() {
        let t = AppOfAppsTree::new();
        assert!(t.descendants("ghost").is_empty());
    }

    #[test]
    fn app_of_apps_roots_only_unreferenced() {
        let mut t = AppOfAppsTree::new();
        t.upsert(AppOfAppsNode { name: "root".into(), tenant: "acme".into(), children: vec!["a".into()] }).unwrap();
        t.upsert(AppOfAppsNode { name: "a".into(), tenant: "acme".into(), children: vec![] }).unwrap();
        let roots = t.roots("acme");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "root");
    }

    #[test]
    fn app_of_apps_replaces_with_same_name_tenant() {
        let mut t = AppOfAppsTree::new();
        t.upsert(AppOfAppsNode { name: "x".into(), tenant: "acme".into(), children: vec![] }).unwrap();
        t.upsert(AppOfAppsNode { name: "x".into(), tenant: "acme".into(), children: vec!["y".into()] }).unwrap();
        assert_eq!(t.count(), 1);
    }

    #[test]
    fn health_snapshot_aggregates() {
        let s = HealthSnapshot::aggregate(&[
            check("D", "a", CheckResult::Pass),
            check("D", "b", CheckResult::Pass),
            check("D", "c", CheckResult::Warn),
            check("D", "d", CheckResult::Fail),
            check("D", "e", CheckResult::Unknown),
        ]);
        assert_eq!(s.passing, 2);
        assert_eq!(s.warning, 1);
        assert_eq!(s.failing, 1);
        assert_eq!(s.unknown, 1);
    }

    #[test]
    fn health_snapshot_overall_fail_takes_priority() {
        let s = HealthSnapshot::aggregate(&[
            check("D", "a", CheckResult::Pass),
            check("D", "b", CheckResult::Fail),
        ]);
        assert_eq!(s.overall(), CheckResult::Fail);
    }

    #[test]
    fn health_snapshot_overall_warn_above_pass() {
        let s = HealthSnapshot::aggregate(&[
            check("D", "a", CheckResult::Pass),
            check("D", "b", CheckResult::Warn),
        ]);
        assert_eq!(s.overall(), CheckResult::Warn);
    }

    #[test]
    fn health_snapshot_overall_pass_when_only_pass() {
        let s = HealthSnapshot::aggregate(&[check("D", "a", CheckResult::Pass)]);
        assert_eq!(s.overall(), CheckResult::Pass);
    }

    #[test]
    fn health_snapshot_overall_unknown_when_empty() {
        let s = HealthSnapshot::aggregate(&[]);
        assert_eq!(s.overall(), CheckResult::Unknown);
    }

    #[test]
    fn health_score_full_pass_is_one() {
        let s = HealthSnapshot::aggregate(&[
            check("D", "a", CheckResult::Pass),
            check("D", "b", CheckResult::Pass),
        ]);
        assert!((s.health_score() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn health_score_warn_counts_half() {
        let s = HealthSnapshot::aggregate(&[
            check("D", "a", CheckResult::Pass),
            check("D", "b", CheckResult::Warn),
        ]);
        assert!((s.health_score() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn health_score_fail_zero() {
        let s = HealthSnapshot::aggregate(&[check("D", "a", CheckResult::Fail)]);
        assert_eq!(s.health_score(), 0.0);
    }

    #[test]
    fn health_score_zero_when_empty() {
        let s = HealthSnapshot::aggregate(&[]);
        assert_eq!(s.health_score(), 0.0);
    }

    #[test]
    fn manual_sync_force_admin_only() {
        let m = ManualSyncSpec {
            tenant: "acme".into(),
            app: "web".into(),
            revision: None,
            prune: false,
            dry_run: false,
            strategy: SyncStrategy::Force,
        };
        assert!(!m.allowed_for(ViewPersona::Tenant));
        assert!(!m.allowed_for(ViewPersona::Operator));
        assert!(m.allowed_for(ViewPersona::Admin));
    }

    #[test]
    fn manual_sync_apply_open_to_all() {
        let m = ManualSyncSpec {
            tenant: "acme".into(),
            app: "web".into(),
            revision: None,
            prune: false,
            dry_run: false,
            strategy: SyncStrategy::Apply,
        };
        assert!(m.allowed_for(ViewPersona::Tenant));
        assert!(m.allowed_for(ViewPersona::Operator));
        assert!(m.allowed_for(ViewPersona::Admin));
    }

    #[test]
    fn manual_sync_round_trips_json() {
        let m = ManualSyncSpec {
            tenant: "acme".into(),
            app: "web".into(),
            revision: Some("v1".into()),
            prune: true,
            dry_run: false,
            strategy: SyncStrategy::Hook,
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: ManualSyncSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn check_result_serializes_snake_case() {
        let s = serde_json::to_string(&CheckResult::Warn).unwrap();
        assert_eq!(s, "\"warn\"");
    }

    #[test]
    fn sync_window_kind_serializes() {
        let s = serde_json::to_string(&SyncWindowKind::Deny).unwrap();
        assert_eq!(s, "\"deny\"");
    }
}
