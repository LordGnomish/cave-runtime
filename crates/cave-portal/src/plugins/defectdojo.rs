//! DefectDojo wrap — finding list, SLA tracker, risk acceptance.
//!
//! Replaces the DefectDojo web UI. Tenant-scoped findings come from
//! cave-vulns / cave-scan / cave-sbom; this plugin shapes them for the
//! native portal page with severity-based SLAs, risk-acceptance lifecycle,
//! and triage workflow.

use super::ViewPersona;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Default SLA in days per severity. Maps to the platform-wide policy.
    pub fn sla_days(&self) -> u32 {
        match self {
            Severity::Critical => 7,
            Severity::High => 30,
            Severity::Medium => 90,
            Severity::Low => 180,
            Severity::Info => 365,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    /// Just imported / discovered.
    Open,
    /// Triaged: confirmed real, awaiting fix.
    Confirmed,
    /// Triaged: false positive, won't fix.
    FalsePositive,
    /// Risk accepted: ack'd risk, monitored, not fixed.
    RiskAccepted,
    /// Closed: fix landed and verified.
    Mitigated,
    /// Closed: out of scope (e.g., decommissioned).
    OutOfScope,
}

impl FindingStatus {
    pub fn is_open(&self) -> bool {
        matches!(self, FindingStatus::Open | FindingStatus::Confirmed)
    }
    pub fn is_closed(&self) -> bool {
        matches!(
            self,
            FindingStatus::FalsePositive
                | FindingStatus::Mitigated
                | FindingStatus::OutOfScope
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub tenant: String,
    pub product: String,
    pub title: String,
    pub severity: Severity,
    pub status: FindingStatus,
    pub cve: Option<String>,
    pub cwe: Option<String>,
    pub component: Option<String>,
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub created_day: String,    // YYYY-MM-DD
    pub last_seen_day: String,  // YYYY-MM-DD
    pub closed_day: Option<String>,
    pub assigned_to: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskAcceptance {
    pub finding_id: String,
    pub accepted_by: String,
    pub reason: String,
    pub accepted_day: String,
    pub expires_day: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DefectError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid day: {0}")]
    InvalidDay(String),
    #[error("acceptance already exists for {0}")]
    AlreadyAccepted(String),
    #[error("expiry must be after acceptance")]
    BadExpiry,
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("severity {0:?} requires admin to risk-accept")]
    AdminOnlyAcceptance(Severity),
    #[error("finding is closed: {0:?}")]
    AlreadyClosed(FindingStatus),
}

fn is_iso_day(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    for &i in &[0_usize, 1, 2, 3, 5, 6, 8, 9] {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
    }
    true
}

fn day_diff_days(from: &str, to: &str) -> i64 {
    // Pure-arithmetic difference in *calendar days* assuming both are ISO.
    // Not timezone-aware; sufficient for SLA bucketing within a single zone.
    let parse = |s: &str| -> Option<(i64, i64, i64)> {
        let bytes = s.as_bytes();
        if !is_iso_day(s) {
            return None;
        }
        let y: i64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
        let m: i64 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
        let d: i64 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
        Some((y, m, d))
    };
    fn julian(y: i64, m: i64, d: i64) -> i64 {
        // simple Julian day number, good enough for diffs
        let a = (14 - m) / 12;
        let y = y + 4800 - a;
        let m = m + 12 * a - 3;
        d + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
    }
    let (a, b) = match (parse(from), parse(to)) {
        (Some(a), Some(b)) => (a, b),
        _ => return 0,
    };
    julian(b.0, b.1, b.2) - julian(a.0, a.1, a.2)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingQuery {
    pub tenant: String,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub severity: Option<Severity>,
    #[serde(default)]
    pub status: Option<FindingStatus>,
    #[serde(default)]
    pub cve: Option<String>,
    #[serde(default)]
    pub assigned_to: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Default)]
pub struct DefectDojoPlugin {
    findings: Vec<Finding>,
    acceptances: Vec<RiskAcceptance>,
}

impl DefectDojoPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, f: Finding) -> Result<(), DefectError> {
        if !is_iso_day(&f.created_day) {
            return Err(DefectError::InvalidDay(f.created_day));
        }
        if !is_iso_day(&f.last_seen_day) {
            return Err(DefectError::InvalidDay(f.last_seen_day));
        }
        if let Some(d) = &f.closed_day {
            if !is_iso_day(d) {
                return Err(DefectError::InvalidDay(d.clone()));
            }
        }
        if let Some(idx) = self
            .findings
            .iter()
            .position(|x| x.tenant == f.tenant && x.id == f.id)
        {
            self.findings[idx] = f;
        } else {
            self.findings.push(f);
        }
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.findings.len()
    }

    pub fn find(&self, tenant: &str, id: &str) -> Option<&Finding> {
        self.findings
            .iter()
            .find(|f| f.tenant == tenant && f.id == id)
    }

    pub fn query(&self, q: &FindingQuery) -> Vec<&Finding> {
        let limit = q.limit.unwrap_or(usize::MAX);
        let mut out: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| f.tenant == q.tenant)
            .filter(|f| q.product.as_deref().map_or(true, |p| f.product == p))
            .filter(|f| q.severity.map_or(true, |s| f.severity == s))
            .filter(|f| q.status.map_or(true, |s| f.status == s))
            .filter(|f| q.cve.as_deref().map_or(true, |c| f.cve.as_deref() == Some(c)))
            .filter(|f| {
                q.assigned_to.as_deref().map_or(true, |u| {
                    f.assigned_to.as_deref() == Some(u)
                })
            })
            .take(limit)
            .collect();
        out.sort_by(|a, b| {
            // descending by severity, then ascending by created_day
            b.severity
                .cmp(&a.severity)
                .then(a.created_day.cmp(&b.created_day))
                .then(a.id.cmp(&b.id))
        });
        out
    }

    pub fn transition(
        &mut self,
        tenant: &str,
        id: &str,
        new_status: FindingStatus,
        actor: &str,
        today: &str,
    ) -> Result<&Finding, DefectError> {
        if !is_iso_day(today) {
            return Err(DefectError::InvalidDay(today.into()));
        }
        let f = self
            .findings
            .iter_mut()
            .find(|f| f.tenant == tenant && f.id == id)
            .ok_or_else(|| DefectError::NotFound(id.into()))?;
        if f.status.is_closed() && new_status != FindingStatus::Open {
            return Err(DefectError::AlreadyClosed(f.status));
        }
        f.status = new_status;
        f.assigned_to = Some(actor.into());
        if new_status.is_closed() {
            f.closed_day = Some(today.into());
        } else if new_status == FindingStatus::Open {
            f.closed_day = None;
        }
        Ok(&*f)
    }

    /// Risk-accept a finding. Critical severity requires admin persona.
    pub fn accept_risk(
        &mut self,
        persona: ViewPersona,
        tenant: &str,
        id: &str,
        accepted_by: &str,
        reason: &str,
        accepted_day: &str,
        expires_day: &str,
    ) -> Result<&RiskAcceptance, DefectError> {
        if !is_iso_day(accepted_day) {
            return Err(DefectError::InvalidDay(accepted_day.into()));
        }
        if !is_iso_day(expires_day) {
            return Err(DefectError::InvalidDay(expires_day.into()));
        }
        if day_diff_days(accepted_day, expires_day) <= 0 {
            return Err(DefectError::BadExpiry);
        }
        let finding = self
            .findings
            .iter_mut()
            .find(|f| f.tenant == tenant && f.id == id)
            .ok_or_else(|| DefectError::NotFound(id.into()))?;
        if finding.severity == Severity::Critical && persona != ViewPersona::Admin {
            return Err(DefectError::AdminOnlyAcceptance(Severity::Critical));
        }
        if finding.status.is_closed() {
            return Err(DefectError::AlreadyClosed(finding.status));
        }
        if self.acceptances.iter().any(|a| a.finding_id == id) {
            return Err(DefectError::AlreadyAccepted(id.into()));
        }
        finding.status = FindingStatus::RiskAccepted;
        let acc = RiskAcceptance {
            finding_id: id.into(),
            accepted_by: accepted_by.into(),
            reason: reason.into(),
            accepted_day: accepted_day.into(),
            expires_day: expires_day.into(),
        };
        self.acceptances.push(acc);
        Ok(self.acceptances.last().unwrap())
    }

    pub fn acceptance(&self, id: &str) -> Option<&RiskAcceptance> {
        self.acceptances.iter().find(|a| a.finding_id == id)
    }

    pub fn expire_acceptances(&mut self, today: &str) -> usize {
        if !is_iso_day(today) {
            return 0;
        }
        let mut expired_ids = Vec::new();
        self.acceptances.retain(|a| {
            if day_diff_days(today, &a.expires_day) <= 0 {
                expired_ids.push(a.finding_id.clone());
                false
            } else {
                true
            }
        });
        for id in &expired_ids {
            if let Some(f) = self.findings.iter_mut().find(|f| f.id == *id) {
                if f.status == FindingStatus::RiskAccepted {
                    f.status = FindingStatus::Open;
                }
            }
        }
        expired_ids.len()
    }

    /// SLA report for a tenant: per-severity counts of "in SLA" / "breached"
    /// based on age of open findings vs. the severity's SLA-days policy.
    pub fn sla_report(&self, tenant: &str, today: &str) -> Vec<SlaRow> {
        let mut acc: HashMap<Severity, SlaRow> = HashMap::new();
        for f in self
            .findings
            .iter()
            .filter(|f| f.tenant == tenant && f.status.is_open())
        {
            let row = acc.entry(f.severity).or_insert(SlaRow {
                severity: f.severity,
                in_sla: 0,
                breached: 0,
                avg_age_days: 0.0,
                total: 0,
            });
            let age = day_diff_days(&f.created_day, today).max(0);
            let sla = f.severity.sla_days() as i64;
            if age > sla {
                row.breached += 1;
            } else {
                row.in_sla += 1;
            }
            row.total += 1;
            // accumulate sum-of-ages in avg_age_days, divide later.
            row.avg_age_days += age as f64;
        }
        for row in acc.values_mut() {
            if row.total > 0 {
                row.avg_age_days /= row.total as f64;
            }
        }
        let mut out: Vec<SlaRow> = acc.into_values().collect();
        out.sort_by(|a, b| b.severity.cmp(&a.severity));
        out
    }

    pub fn by_product(&self, tenant: &str) -> HashMap<String, ProductSummary> {
        let mut acc: HashMap<String, ProductSummary> = HashMap::new();
        for f in self.findings.iter().filter(|f| f.tenant == tenant) {
            let s = acc.entry(f.product.clone()).or_insert(ProductSummary {
                product: f.product.clone(),
                open: 0,
                closed: 0,
                critical_open: 0,
                high_open: 0,
            });
            if f.status.is_open() {
                s.open += 1;
                match f.severity {
                    Severity::Critical => s.critical_open += 1,
                    Severity::High => s.high_open += 1,
                    _ => {}
                }
            } else {
                s.closed += 1;
            }
        }
        acc
    }

    pub fn allowed_for(persona: ViewPersona) -> bool {
        // All personas can read findings; mutations are gated per-method.
        let _ = persona;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlaRow {
    pub severity: Severity,
    pub in_sla: u32,
    pub breached: u32,
    pub avg_age_days: f64,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductSummary {
    pub product: String,
    pub open: u32,
    pub closed: u32,
    pub critical_open: u32,
    pub high_open: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: &str, sev: Severity, created: &str) -> Finding {
        Finding {
            id: id.into(),
            tenant: "acme".into(),
            product: "web".into(),
            title: format!("Issue {id}"),
            severity: sev,
            status: FindingStatus::Open,
            cve: None,
            cwe: None,
            component: None,
            file_path: None,
            line: None,
            created_day: created.into(),
            last_seen_day: created.into(),
            closed_day: None,
            assigned_to: None,
            tags: Vec::new(),
        }
    }

    fn populated() -> DefectDojoPlugin {
        let mut p = DefectDojoPlugin::new();
        p.upsert(finding("a-1", Severity::Critical, "2026-04-01")).unwrap();
        p.upsert(finding("a-2", Severity::High, "2026-04-15")).unwrap();
        let mut m = finding("a-3", Severity::Medium, "2026-04-20");
        m.product = "api".into();
        p.upsert(m).unwrap();
        p.upsert(finding("a-4", Severity::Low, "2026-04-25")).unwrap();
        p
    }

    #[test]
    fn severity_sla_days_table() {
        assert_eq!(Severity::Critical.sla_days(), 7);
        assert_eq!(Severity::High.sla_days(), 30);
        assert_eq!(Severity::Medium.sla_days(), 90);
        assert_eq!(Severity::Low.sla_days(), 180);
        assert_eq!(Severity::Info.sla_days(), 365);
    }

    #[test]
    fn severity_ordering_critical_highest() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Info);
    }

    #[test]
    fn finding_status_open_set() {
        assert!(FindingStatus::Open.is_open());
        assert!(FindingStatus::Confirmed.is_open());
        assert!(!FindingStatus::Mitigated.is_open());
    }

    #[test]
    fn finding_status_closed_set() {
        assert!(FindingStatus::FalsePositive.is_closed());
        assert!(FindingStatus::Mitigated.is_closed());
        assert!(FindingStatus::OutOfScope.is_closed());
        assert!(!FindingStatus::RiskAccepted.is_closed());
        assert!(!FindingStatus::Open.is_closed());
    }

    #[test]
    fn upsert_inserts_new() {
        let mut p = DefectDojoPlugin::new();
        p.upsert(finding("a", Severity::Low, "2026-04-01")).unwrap();
        assert_eq!(p.count(), 1);
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut p = DefectDojoPlugin::new();
        p.upsert(finding("a", Severity::Low, "2026-04-01")).unwrap();
        let mut updated = finding("a", Severity::High, "2026-04-01");
        updated.title = "updated".into();
        p.upsert(updated).unwrap();
        assert_eq!(p.count(), 1);
        assert_eq!(p.find("acme", "a").unwrap().severity, Severity::High);
    }

    #[test]
    fn upsert_invalid_created_day() {
        let mut p = DefectDojoPlugin::new();
        let mut f = finding("a", Severity::Low, "01-04-2026");
        let _ = std::mem::replace(&mut f.created_day, "01-04-2026".into());
        let err = p.upsert(f).unwrap_err();
        assert!(matches!(err, DefectError::InvalidDay(_)));
    }

    #[test]
    fn upsert_invalid_last_seen_day() {
        let mut p = DefectDojoPlugin::new();
        let mut f = finding("a", Severity::Low, "2026-04-01");
        f.last_seen_day = "bad-date".into();
        let err = p.upsert(f).unwrap_err();
        assert!(matches!(err, DefectError::InvalidDay(_)));
    }

    #[test]
    fn query_filters_by_severity() {
        let p = populated();
        let q = FindingQuery {
            tenant: "acme".into(),
            severity: Some(Severity::Critical),
            ..Default::default()
        };
        let out = p.query(&q);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a-1");
    }

    #[test]
    fn query_filters_by_product() {
        let p = populated();
        let q = FindingQuery {
            tenant: "acme".into(),
            product: Some("api".into()),
            ..Default::default()
        };
        let out = p.query(&q);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a-3");
    }

    #[test]
    fn query_filters_by_status() {
        let mut p = populated();
        p.transition("acme", "a-2", FindingStatus::Mitigated, "alice", "2026-04-26").unwrap();
        let q = FindingQuery {
            tenant: "acme".into(),
            status: Some(FindingStatus::Mitigated),
            ..Default::default()
        };
        assert_eq!(p.query(&q).len(), 1);
    }

    #[test]
    fn query_returns_descending_severity() {
        let p = populated();
        let q = FindingQuery { tenant: "acme".into(), ..Default::default() };
        let out = p.query(&q);
        let sevs: Vec<Severity> = out.iter().map(|f| f.severity).collect();
        let mut sorted = sevs.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(sevs, sorted);
    }

    #[test]
    fn query_filters_by_tenant_id() {
        let mut p = populated();
        let mut g = finding("g-1", Severity::High, "2026-04-01");
        g.tenant = "globex".into();
        p.upsert(g).unwrap();
        let q = FindingQuery { tenant: "globex".into(), ..Default::default() };
        let out = p.query(&q);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tenant, "globex");
    }

    #[test]
    fn query_respects_limit() {
        let p = populated();
        let q = FindingQuery { tenant: "acme".into(), limit: Some(2), ..Default::default() };
        let out = p.query(&q);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn query_filters_by_cve() {
        let mut p = DefectDojoPlugin::new();
        let mut f = finding("a", Severity::High, "2026-04-01");
        f.cve = Some("CVE-2026-1".into());
        p.upsert(f).unwrap();
        let q = FindingQuery {
            tenant: "acme".into(),
            cve: Some("CVE-2026-1".into()),
            ..Default::default()
        };
        assert_eq!(p.query(&q).len(), 1);
    }

    #[test]
    fn query_filters_by_assignee() {
        let mut p = populated();
        p.transition("acme", "a-2", FindingStatus::Confirmed, "alice", "2026-04-26").unwrap();
        let q = FindingQuery {
            tenant: "acme".into(),
            assigned_to: Some("alice".into()),
            ..Default::default()
        };
        let out = p.query(&q);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a-2");
    }

    #[test]
    fn transition_to_mitigated_records_closed_day() {
        let mut p = populated();
        let f = p.transition("acme", "a-1", FindingStatus::Mitigated, "alice", "2026-04-26").unwrap();
        assert_eq!(f.status, FindingStatus::Mitigated);
        assert_eq!(f.closed_day.as_deref(), Some("2026-04-26"));
    }

    #[test]
    fn transition_unknown_finding() {
        let mut p = DefectDojoPlugin::new();
        let err = p.transition("acme", "ghost", FindingStatus::Open, "x", "2026-04-26").unwrap_err();
        assert!(matches!(err, DefectError::NotFound(_)));
    }

    #[test]
    fn transition_invalid_today() {
        let mut p = populated();
        let err = p.transition("acme", "a-1", FindingStatus::Confirmed, "x", "bad").unwrap_err();
        assert!(matches!(err, DefectError::InvalidDay(_)));
    }

    #[test]
    fn transition_closed_can_only_reopen() {
        let mut p = populated();
        p.transition("acme", "a-1", FindingStatus::Mitigated, "x", "2026-04-26").unwrap();
        let err = p.transition("acme", "a-1", FindingStatus::Confirmed, "x", "2026-04-26").unwrap_err();
        assert!(matches!(err, DefectError::AlreadyClosed(_)));
        // reopening is allowed
        let f = p.transition("acme", "a-1", FindingStatus::Open, "x", "2026-04-27").unwrap();
        assert_eq!(f.status, FindingStatus::Open);
        assert!(f.closed_day.is_none());
    }

    #[test]
    fn accept_risk_critical_requires_admin() {
        let mut p = populated();
        let err = p.accept_risk(
            ViewPersona::Tenant, "acme", "a-1", "alice", "fp", "2026-04-26", "2026-05-26",
        )
        .unwrap_err();
        assert!(matches!(err, DefectError::AdminOnlyAcceptance(_)));
    }

    #[test]
    fn accept_risk_critical_admin_ok() {
        let mut p = populated();
        let acc = p
            .accept_risk(
                ViewPersona::Admin, "acme", "a-1", "alice", "compensating ctrl", "2026-04-26", "2026-05-26",
            )
            .unwrap();
        assert_eq!(acc.accepted_by, "alice");
    }

    #[test]
    fn accept_risk_changes_status() {
        let mut p = populated();
        p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "alice", "r", "2026-04-26", "2026-05-26").unwrap();
        assert_eq!(p.find("acme", "a-2").unwrap().status, FindingStatus::RiskAccepted);
    }

    #[test]
    fn accept_risk_duplicate_rejected() {
        let mut p = populated();
        p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "alice", "r", "2026-04-26", "2026-05-26").unwrap();
        let err = p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "bob", "r", "2026-04-26", "2026-05-26").unwrap_err();
        assert!(matches!(err, DefectError::AlreadyAccepted(_)));
    }

    #[test]
    fn accept_risk_invalid_day() {
        let mut p = populated();
        let err = p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "x", "r", "bad", "2026-05-26").unwrap_err();
        assert!(matches!(err, DefectError::InvalidDay(_)));
    }

    #[test]
    fn accept_risk_expiry_must_be_after_acceptance() {
        let mut p = populated();
        let err = p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "x", "r", "2026-04-26", "2026-04-26").unwrap_err();
        assert_eq!(err, DefectError::BadExpiry);
    }

    #[test]
    fn accept_risk_unknown_finding() {
        let mut p = populated();
        let err = p.accept_risk(ViewPersona::Tenant, "acme", "ghost", "x", "r", "2026-04-26", "2026-05-26").unwrap_err();
        assert!(matches!(err, DefectError::NotFound(_)));
    }

    #[test]
    fn accept_risk_closed_finding_rejected() {
        let mut p = populated();
        p.transition("acme", "a-2", FindingStatus::Mitigated, "alice", "2026-04-26").unwrap();
        let err = p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "x", "r", "2026-04-26", "2026-05-26").unwrap_err();
        assert!(matches!(err, DefectError::AlreadyClosed(_)));
    }

    #[test]
    fn acceptance_lookup() {
        let mut p = populated();
        p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "alice", "r", "2026-04-26", "2026-05-26").unwrap();
        assert!(p.acceptance("a-2").is_some());
        assert!(p.acceptance("ghost").is_none());
    }

    #[test]
    fn expire_acceptances_reopens_finding() {
        let mut p = populated();
        p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "alice", "r", "2026-04-01", "2026-04-15").unwrap();
        let n = p.expire_acceptances("2026-04-26");
        assert_eq!(n, 1);
        assert_eq!(p.find("acme", "a-2").unwrap().status, FindingStatus::Open);
    }

    #[test]
    fn expire_acceptances_keeps_unexpired() {
        let mut p = populated();
        p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "alice", "r", "2026-04-26", "2026-06-26").unwrap();
        let n = p.expire_acceptances("2026-04-27");
        assert_eq!(n, 0);
    }

    #[test]
    fn expire_acceptances_invalid_today_noop() {
        let mut p = populated();
        p.accept_risk(ViewPersona::Tenant, "acme", "a-2", "x", "r", "2026-04-26", "2026-06-26").unwrap();
        assert_eq!(p.expire_acceptances("bad"), 0);
    }

    #[test]
    fn sla_report_buckets_by_severity() {
        let p = populated();
        let r = p.sla_report("acme", "2026-04-27");
        let critical = r.iter().find(|x| x.severity == Severity::Critical).unwrap();
        assert_eq!(critical.total, 1);
    }

    #[test]
    fn sla_report_marks_breach() {
        // Critical SLA = 7 days. a-1 created 2026-04-01, today 2026-04-15 → 14 days = breach.
        let p = populated();
        let r = p.sla_report("acme", "2026-04-15");
        let crit = r.iter().find(|x| x.severity == Severity::Critical).unwrap();
        assert_eq!(crit.breached, 1);
        assert_eq!(crit.in_sla, 0);
    }

    #[test]
    fn sla_report_marks_in_sla() {
        // High SLA = 30 days. a-2 created 2026-04-15, today 2026-04-20 → 5 days = in SLA.
        let p = populated();
        let r = p.sla_report("acme", "2026-04-20");
        let high = r.iter().find(|x| x.severity == Severity::High).unwrap();
        assert_eq!(high.in_sla, 1);
        assert_eq!(high.breached, 0);
    }

    #[test]
    fn sla_report_skips_closed() {
        let mut p = populated();
        p.transition("acme", "a-1", FindingStatus::Mitigated, "x", "2026-04-26").unwrap();
        let r = p.sla_report("acme", "2026-04-27");
        let crit = r.iter().find(|x| x.severity == Severity::Critical);
        assert!(crit.is_none());
    }

    #[test]
    fn sla_report_average_age() {
        let p = populated();
        let r = p.sla_report("acme", "2026-04-27");
        let crit = r.iter().find(|x| x.severity == Severity::Critical).unwrap();
        // age of a-1 from 2026-04-01 to 2026-04-27 = 26
        assert!((crit.avg_age_days - 26.0).abs() < 1e-9);
    }

    #[test]
    fn sla_report_descending_severity() {
        let p = populated();
        let r = p.sla_report("acme", "2026-04-27");
        let sevs: Vec<Severity> = r.iter().map(|x| x.severity).collect();
        let mut sorted = sevs.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(sevs, sorted);
    }

    #[test]
    fn by_product_aggregates_open_and_closed() {
        let mut p = populated();
        p.transition("acme", "a-1", FindingStatus::Mitigated, "x", "2026-04-26").unwrap();
        let by = p.by_product("acme");
        let web = &by["web"];
        assert_eq!(web.closed, 1);
        assert_eq!(web.open, 2); // a-2 (high), a-4 (low)
    }

    #[test]
    fn by_product_counts_critical_and_high_open() {
        let p = populated();
        let by = p.by_product("acme");
        let web = &by["web"];
        assert_eq!(web.critical_open, 1);
        assert_eq!(web.high_open, 1);
    }

    #[test]
    fn day_diff_matches_simple_cases() {
        assert_eq!(day_diff_days("2026-04-01", "2026-04-02"), 1);
        assert_eq!(day_diff_days("2026-04-01", "2026-04-01"), 0);
        assert_eq!(day_diff_days("2026-04-02", "2026-04-01"), -1);
        // month boundary
        assert_eq!(day_diff_days("2026-03-31", "2026-04-01"), 1);
        // year boundary
        assert_eq!(day_diff_days("2025-12-31", "2026-01-01"), 1);
    }

    #[test]
    fn day_diff_invalid_returns_zero() {
        assert_eq!(day_diff_days("bad", "2026-04-01"), 0);
        assert_eq!(day_diff_days("2026-04-01", "bad"), 0);
    }

    #[test]
    fn finding_round_trips_json() {
        let f = finding("a", Severity::High, "2026-04-01");
        let s = serde_json::to_string(&f).unwrap();
        let back: Finding = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }
}
