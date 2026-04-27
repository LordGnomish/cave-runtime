//! Grafana wrap — alerting, folders, annotations, notification channels.
//!
//! Closes the most visible gap in the Grafana wrap: there was no alerting,
//! folder organization, annotation overlay, or notification routing — the
//! features operators actually open upstream Grafana for. Adding them here
//! keeps the single-pane-of-glass invariant: no "Open in Grafana" link.

use super::ViewPersona;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Folder organisation ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub tenant: String,
    pub title: String,
    pub parent: Option<String>,
}

impl Folder {
    pub fn new(id: impl Into<String>, tenant: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tenant: tenant.into(),
            title: title.into(),
            parent: None,
        }
    }

    pub fn with_parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
}

// ── Alert rules ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparator {
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    Equal,
    NotEqual,
}

impl Comparator {
    pub fn check(&self, value: f64, threshold: f64) -> bool {
        match self {
            Comparator::GreaterThan => value > threshold,
            Comparator::GreaterOrEqual => value >= threshold,
            Comparator::LessThan => value < threshold,
            Comparator::LessOrEqual => value <= threshold,
            Comparator::Equal => (value - threshold).abs() < f64::EPSILON,
            Comparator::NotEqual => (value - threshold).abs() >= f64::EPSILON,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: String,
    pub tenant: String,
    pub folder_id: Option<String>,
    pub title: String,
    pub query: String,
    pub comparator: Comparator,
    pub threshold_bits: u64, // f64 bits for Eq derive
    pub severity: AlertSeverity,
    /// How long the comparator must be true before firing.
    pub for_seconds: u64,
    pub labels: Vec<(String, String)>,
    pub channel_ids: Vec<String>,
    pub paused: bool,
}

impl AlertRule {
    pub fn new(
        id: impl Into<String>,
        tenant: impl Into<String>,
        title: impl Into<String>,
        query: impl Into<String>,
        comparator: Comparator,
        threshold: f64,
        severity: AlertSeverity,
    ) -> Self {
        Self {
            id: id.into(),
            tenant: tenant.into(),
            folder_id: None,
            title: title.into(),
            query: query.into(),
            comparator,
            threshold_bits: threshold.to_bits(),
            severity,
            for_seconds: 0,
            labels: Vec::new(),
            channel_ids: Vec::new(),
            paused: false,
        }
    }

    pub fn threshold(&self) -> f64 {
        f64::from_bits(self.threshold_bits)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Ok,
    Pending,
    Alerting,
    Error,
    NoData,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvaluation {
    pub rule_id: String,
    pub at_unix: u64,
    pub state: AlertState,
    pub value: f64,
    pub message: String,
}

// ── Annotations (events on graphs) ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub tenant: String,
    pub dashboard_id: Option<String>,
    pub panel_id: Option<String>,
    pub at_unix: u64,
    pub end_unix: Option<u64>,
    pub text: String,
    pub tags: Vec<String>,
}

// ── Notification channels ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Chat,
    Webhook,
    Email,
    Pager,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationChannel {
    pub id: String,
    pub tenant: String,
    pub kind: ChannelKind,
    pub name: String,
    pub target: String, // chat channel id, url, email, pager team
    pub min_severity: AlertSeverity,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum AlertingError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid id: {0}")]
    InvalidId(String),
    #[error("invalid threshold: {0}")]
    InvalidThreshold(f64),
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("rule paused")]
    RulePaused,
    #[error("cycle in folder hierarchy")]
    FolderCycle,
    #[error("annotation end_unix must be >= at_unix")]
    BadAnnotationRange,
    #[error("channel kind {0:?} requires non-empty target")]
    EmptyTarget(ChannelKind),
}

fn validate_id(id: &str) -> Result<(), AlertingError> {
    if id.is_empty() || id.len() > 64 {
        return Err(AlertingError::InvalidId(id.into()));
    }
    for ch in id.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_';
        if !ok {
            return Err(AlertingError::InvalidId(id.into()));
        }
    }
    Ok(())
}

// ── Plugin ────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct GrafanaAlertingPlugin {
    folders: Vec<Folder>,
    rules: Vec<AlertRule>,
    evaluations: Vec<AlertEvaluation>,
    annotations: Vec<Annotation>,
    channels: Vec<NotificationChannel>,
    /// Pending fires waiting for `for_seconds` to elapse.
    /// Maps rule_id → first time the rule went from Ok to firing condition.
    pending_since: HashMap<String, u64>,
}

impl GrafanaAlertingPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Folders ─────────────────────────────────────────────────────────────

    pub fn upsert_folder(
        &mut self,
        persona: ViewPersona,
        folder: Folder,
    ) -> Result<(), AlertingError> {
        if !matches!(persona, ViewPersona::Tenant | ViewPersona::Admin) {
            return Err(AlertingError::Forbidden("operator cannot edit folders"));
        }
        validate_id(&folder.id)?;
        if let Some(parent) = &folder.parent {
            if Self::would_form_cycle(&self.folders, &folder.id, parent) {
                return Err(AlertingError::FolderCycle);
            }
        }
        if let Some(idx) = self
            .folders
            .iter()
            .position(|f| f.tenant == folder.tenant && f.id == folder.id)
        {
            self.folders[idx] = folder;
        } else {
            self.folders.push(folder);
        }
        Ok(())
    }

    fn would_form_cycle(folders: &[Folder], child: &str, ancestor: &str) -> bool {
        if child == ancestor {
            return true;
        }
        let mut cur = ancestor.to_string();
        let mut seen = std::collections::HashSet::new();
        loop {
            if cur == child {
                return true;
            }
            if !seen.insert(cur.clone()) {
                return false;
            }
            match folders.iter().find(|f| f.id == cur).and_then(|f| f.parent.clone()) {
                Some(p) => cur = p,
                None => return false,
            }
        }
    }

    pub fn folder(&self, tenant: &str, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.tenant == tenant && f.id == id)
    }

    pub fn list_folders(&self, tenant: &str) -> Vec<&Folder> {
        let mut out: Vec<&Folder> =
            self.folders.iter().filter(|f| f.tenant == tenant).collect();
        out.sort_by(|a, b| a.title.cmp(&b.title));
        out
    }

    // ── Rules ───────────────────────────────────────────────────────────────

    pub fn upsert_rule(
        &mut self,
        persona: ViewPersona,
        rule: AlertRule,
    ) -> Result<(), AlertingError> {
        if !matches!(persona, ViewPersona::Tenant | ViewPersona::Admin) {
            return Err(AlertingError::Forbidden("operator cannot edit rules"));
        }
        validate_id(&rule.id)?;
        let t = rule.threshold();
        if t.is_nan() || t.is_infinite() {
            return Err(AlertingError::InvalidThreshold(t));
        }
        if let Some(folder_id) = &rule.folder_id {
            if !self.folders.iter().any(|f| f.tenant == rule.tenant && &f.id == folder_id) {
                return Err(AlertingError::NotFound(folder_id.clone()));
            }
        }
        if let Some(idx) = self
            .rules
            .iter()
            .position(|r| r.tenant == rule.tenant && r.id == rule.id)
        {
            self.rules[idx] = rule;
        } else {
            self.rules.push(rule);
        }
        Ok(())
    }

    pub fn rule(&self, tenant: &str, id: &str) -> Option<&AlertRule> {
        self.rules.iter().find(|r| r.tenant == tenant && r.id == id)
    }

    pub fn list_rules(&self, tenant: &str) -> Vec<&AlertRule> {
        let mut out: Vec<&AlertRule> = self.rules.iter().filter(|r| r.tenant == tenant).collect();
        out.sort_by(|a, b| a.title.cmp(&b.title));
        out
    }

    pub fn pause_rule(&mut self, tenant: &str, id: &str, paused: bool) -> Result<(), AlertingError> {
        let rule = self
            .rules
            .iter_mut()
            .find(|r| r.tenant == tenant && r.id == id)
            .ok_or_else(|| AlertingError::NotFound(id.into()))?;
        rule.paused = paused;
        if paused {
            self.pending_since.remove(id);
        }
        Ok(())
    }

    /// Evaluate a rule against a single sample. Updates internal pending
    /// state and returns the resulting evaluation (which is also stored).
    pub fn evaluate(
        &mut self,
        tenant: &str,
        rule_id: &str,
        value: f64,
        at_unix: u64,
    ) -> Result<AlertEvaluation, AlertingError> {
        let rule = self
            .rules
            .iter()
            .find(|r| r.tenant == tenant && r.id == rule_id)
            .ok_or_else(|| AlertingError::NotFound(rule_id.into()))?
            .clone();
        if rule.paused {
            return Err(AlertingError::RulePaused);
        }
        if value.is_nan() {
            let ev = AlertEvaluation {
                rule_id: rule_id.into(),
                at_unix,
                state: AlertState::NoData,
                value,
                message: "no data".into(),
            };
            self.evaluations.push(ev.clone());
            self.pending_since.remove(rule_id);
            return Ok(ev);
        }
        let firing = rule.comparator.check(value, rule.threshold());
        let state = if !firing {
            self.pending_since.remove(rule_id);
            AlertState::Ok
        } else {
            let since = *self.pending_since.entry(rule_id.into()).or_insert(at_unix);
            if at_unix.saturating_sub(since) >= rule.for_seconds {
                AlertState::Alerting
            } else {
                AlertState::Pending
            }
        };
        let message = match state {
            AlertState::Ok => format!("{} = {:.4} (within bounds)", rule.title, value),
            AlertState::Pending => format!(
                "{} = {:.4} {:?} {:.4} (pending)",
                rule.title, value, rule.comparator, rule.threshold()
            ),
            AlertState::Alerting => format!(
                "{} = {:.4} {:?} {:.4}",
                rule.title, value, rule.comparator, rule.threshold()
            ),
            AlertState::NoData => "no data".into(),
            AlertState::Error => "evaluation error".into(),
        };
        let ev = AlertEvaluation {
            rule_id: rule_id.into(),
            at_unix,
            state,
            value,
            message,
        };
        self.evaluations.push(ev.clone());
        Ok(ev)
    }

    pub fn evaluations_for(&self, rule_id: &str, limit: usize) -> Vec<&AlertEvaluation> {
        let mut out: Vec<&AlertEvaluation> = self
            .evaluations
            .iter()
            .filter(|e| e.rule_id == rule_id)
            .collect();
        out.sort_by(|a, b| b.at_unix.cmp(&a.at_unix));
        out.truncate(limit);
        out
    }

    pub fn alerting_rules(&self, tenant: &str) -> Vec<&AlertRule> {
        // A rule is "currently alerting" iff its latest evaluation was Alerting.
        let mut latest: HashMap<&str, &AlertEvaluation> = HashMap::new();
        for ev in &self.evaluations {
            latest
                .entry(ev.rule_id.as_str())
                .and_modify(|cur| {
                    if ev.at_unix > cur.at_unix {
                        *cur = ev;
                    }
                })
                .or_insert(ev);
        }
        self.rules
            .iter()
            .filter(|r| r.tenant == tenant)
            .filter(|r| {
                latest
                    .get(r.id.as_str())
                    .map(|ev| ev.state == AlertState::Alerting)
                    .unwrap_or(false)
            })
            .collect()
    }

    // ── Annotations ─────────────────────────────────────────────────────────

    pub fn add_annotation(
        &mut self,
        annotation: Annotation,
    ) -> Result<(), AlertingError> {
        validate_id(&annotation.id)?;
        if let Some(end) = annotation.end_unix {
            if end < annotation.at_unix {
                return Err(AlertingError::BadAnnotationRange);
            }
        }
        if let Some(idx) = self
            .annotations
            .iter()
            .position(|a| a.tenant == annotation.tenant && a.id == annotation.id)
        {
            self.annotations[idx] = annotation;
        } else {
            self.annotations.push(annotation);
        }
        Ok(())
    }

    pub fn annotations_for_dashboard(
        &self,
        tenant: &str,
        dashboard_id: &str,
        from_unix: u64,
        to_unix: u64,
    ) -> Vec<&Annotation> {
        self.annotations
            .iter()
            .filter(|a| a.tenant == tenant)
            .filter(|a| a.dashboard_id.as_deref() == Some(dashboard_id))
            .filter(|a| a.at_unix >= from_unix && a.at_unix <= to_unix)
            .collect()
    }

    pub fn annotations_by_tag(&self, tenant: &str, tag: &str) -> Vec<&Annotation> {
        self.annotations
            .iter()
            .filter(|a| a.tenant == tenant && a.tags.iter().any(|t| t == tag))
            .collect()
    }

    // ── Notification channels ───────────────────────────────────────────────

    pub fn upsert_channel(
        &mut self,
        persona: ViewPersona,
        channel: NotificationChannel,
    ) -> Result<(), AlertingError> {
        if !matches!(persona, ViewPersona::Tenant | ViewPersona::Admin) {
            return Err(AlertingError::Forbidden("operator cannot edit channels"));
        }
        validate_id(&channel.id)?;
        if channel.target.is_empty() {
            return Err(AlertingError::EmptyTarget(channel.kind));
        }
        if let Some(idx) = self
            .channels
            .iter()
            .position(|c| c.tenant == channel.tenant && c.id == channel.id)
        {
            self.channels[idx] = channel;
        } else {
            self.channels.push(channel);
        }
        Ok(())
    }

    pub fn channel(&self, tenant: &str, id: &str) -> Option<&NotificationChannel> {
        self.channels.iter().find(|c| c.tenant == tenant && c.id == id)
    }

    /// Compute which channels would receive the given alert evaluation.
    pub fn fanout(&self, tenant: &str, eval: &AlertEvaluation) -> Vec<&NotificationChannel> {
        if eval.state != AlertState::Alerting {
            return Vec::new();
        }
        let rule = match self.rules.iter().find(|r| r.tenant == tenant && r.id == eval.rule_id) {
            Some(r) => r,
            None => return Vec::new(),
        };
        rule.channel_ids
            .iter()
            .filter_map(|cid| self.channel(tenant, cid))
            .filter(|c| severity_rank(rule.severity) >= severity_rank(c.min_severity))
            .collect()
    }

    pub fn count_rules(&self) -> usize {
        self.rules.len()
    }
    pub fn count_folders(&self) -> usize {
        self.folders.len()
    }
    pub fn count_annotations(&self) -> usize {
        self.annotations.len()
    }
    pub fn count_channels(&self) -> usize {
        self.channels.len()
    }
    pub fn count_evaluations(&self) -> usize {
        self.evaluations.len()
    }
}

fn severity_rank(s: AlertSeverity) -> u8 {
    match s {
        AlertSeverity::Info => 0,
        AlertSeverity::Warning => 1,
        AlertSeverity::Critical => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, threshold: f64, cmp: Comparator) -> AlertRule {
        AlertRule::new(id, "acme", "T", "up", cmp, threshold, AlertSeverity::Warning)
    }

    fn admin() -> ViewPersona { ViewPersona::Admin }
    fn tenant() -> ViewPersona { ViewPersona::Tenant }
    fn op() -> ViewPersona { ViewPersona::Operator }

    // ── Comparator ──────────────────────────────────────────────────────────

    #[test]
    fn comparator_gt() {
        assert!(Comparator::GreaterThan.check(2.0, 1.0));
        assert!(!Comparator::GreaterThan.check(1.0, 1.0));
    }

    #[test]
    fn comparator_ge() {
        assert!(Comparator::GreaterOrEqual.check(1.0, 1.0));
        assert!(!Comparator::GreaterOrEqual.check(0.5, 1.0));
    }

    #[test]
    fn comparator_lt_le() {
        assert!(Comparator::LessThan.check(0.5, 1.0));
        assert!(Comparator::LessOrEqual.check(1.0, 1.0));
        assert!(!Comparator::LessThan.check(1.0, 1.0));
    }

    #[test]
    fn comparator_eq_neq() {
        assert!(Comparator::Equal.check(1.0, 1.0));
        assert!(!Comparator::Equal.check(1.5, 1.0));
        assert!(Comparator::NotEqual.check(1.5, 1.0));
        assert!(!Comparator::NotEqual.check(1.0, 1.0));
    }

    // ── Folders ─────────────────────────────────────────────────────────────

    #[test]
    fn folder_upsert_admin_succeeds() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_folder(admin(), Folder::new("ops", "acme", "Ops")).unwrap();
        assert_eq!(p.count_folders(), 1);
    }

    #[test]
    fn folder_upsert_tenant_succeeds() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_folder(tenant(), Folder::new("dev", "acme", "Dev")).unwrap();
        assert_eq!(p.count_folders(), 1);
    }

    #[test]
    fn folder_upsert_operator_denied() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_folder(op(), Folder::new("x", "acme", "X")).unwrap_err();
        assert!(matches!(err, AlertingError::Forbidden(_)));
    }

    #[test]
    fn folder_invalid_id() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_folder(admin(), Folder::new("BAD", "acme", "X")).unwrap_err();
        assert!(matches!(err, AlertingError::InvalidId(_)));
    }

    #[test]
    fn folder_replaces_same_id_tenant() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_folder(admin(), Folder::new("ops", "acme", "Old")).unwrap();
        p.upsert_folder(admin(), Folder::new("ops", "acme", "New")).unwrap();
        assert_eq!(p.count_folders(), 1);
        assert_eq!(p.folder("acme", "ops").unwrap().title, "New");
    }

    #[test]
    fn folder_self_parent_cycle() {
        let mut p = GrafanaAlertingPlugin::new();
        let f = Folder::new("a", "acme", "A").with_parent("a");
        let err = p.upsert_folder(admin(), f).unwrap_err();
        assert_eq!(err, AlertingError::FolderCycle);
    }

    #[test]
    fn folder_indirect_cycle() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_folder(admin(), Folder::new("a", "acme", "A")).unwrap();
        p.upsert_folder(admin(), Folder::new("b", "acme", "B").with_parent("a")).unwrap();
        // making a's parent be b would form cycle
        let f = Folder::new("a", "acme", "A").with_parent("b");
        let err = p.upsert_folder(admin(), f).unwrap_err();
        assert_eq!(err, AlertingError::FolderCycle);
    }

    #[test]
    fn folder_list_sorted_by_title() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_folder(admin(), Folder::new("z", "acme", "Zeta")).unwrap();
        p.upsert_folder(admin(), Folder::new("a", "acme", "Alpha")).unwrap();
        let titles: Vec<&str> = p.list_folders("acme").iter().map(|f| f.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Zeta"]);
    }

    // ── Rules ───────────────────────────────────────────────────────────────

    #[test]
    fn rule_upsert_tenant_succeeds() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(tenant(), rule("hot-cpu", 80.0, Comparator::GreaterThan)).unwrap();
        assert_eq!(p.count_rules(), 1);
    }

    #[test]
    fn rule_upsert_operator_denied() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_rule(op(), rule("x", 1.0, Comparator::GreaterThan)).unwrap_err();
        assert!(matches!(err, AlertingError::Forbidden(_)));
    }

    #[test]
    fn rule_threshold_nan_rejected() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_rule(admin(), rule("x", f64::NAN, Comparator::GreaterThan)).unwrap_err();
        assert!(matches!(err, AlertingError::InvalidThreshold(_)));
    }

    #[test]
    fn rule_threshold_inf_rejected() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_rule(admin(), rule("x", f64::INFINITY, Comparator::GreaterThan)).unwrap_err();
        assert!(matches!(err, AlertingError::InvalidThreshold(_)));
    }

    #[test]
    fn rule_with_unknown_folder_rejected() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut r = rule("x", 1.0, Comparator::GreaterThan);
        r.folder_id = Some("ghost".into());
        let err = p.upsert_rule(admin(), r).unwrap_err();
        assert!(matches!(err, AlertingError::NotFound(_)));
    }

    #[test]
    fn rule_with_known_folder_ok() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_folder(admin(), Folder::new("ops", "acme", "Ops")).unwrap();
        let mut r = rule("x", 1.0, Comparator::GreaterThan);
        r.folder_id = Some("ops".into());
        assert!(p.upsert_rule(admin(), r).is_ok());
    }

    #[test]
    fn rule_threshold_round_trips() {
        let r = rule("x", 80.5, Comparator::GreaterThan);
        assert!((r.threshold() - 80.5).abs() < f64::EPSILON);
    }

    #[test]
    fn rule_pause_resets_pending() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut r = rule("hot", 80.0, Comparator::GreaterThan);
        r.for_seconds = 60;
        p.upsert_rule(admin(), r).unwrap();
        p.evaluate("acme", "hot", 90.0, 0).unwrap();
        assert!(p.pending_since.contains_key("hot"));
        p.pause_rule("acme", "hot", true).unwrap();
        assert!(!p.pending_since.contains_key("hot"));
    }

    #[test]
    fn rule_pause_unknown() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.pause_rule("acme", "ghost", true).unwrap_err();
        assert!(matches!(err, AlertingError::NotFound(_)));
    }

    #[test]
    fn list_rules_sorted_by_title() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut a = rule("a", 1.0, Comparator::GreaterThan);
        a.title = "Zeta".into();
        let mut b = rule("b", 1.0, Comparator::GreaterThan);
        b.title = "Alpha".into();
        p.upsert_rule(admin(), a).unwrap();
        p.upsert_rule(admin(), b).unwrap();
        let titles: Vec<&str> = p.list_rules("acme").iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Zeta"]);
    }

    // ── Evaluation ──────────────────────────────────────────────────────────

    #[test]
    fn evaluate_unknown_rule() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.evaluate("acme", "ghost", 1.0, 0).unwrap_err();
        assert!(matches!(err, AlertingError::NotFound(_)));
    }

    #[test]
    fn evaluate_paused_rule_errors() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        p.pause_rule("acme", "hot", true).unwrap();
        let err = p.evaluate("acme", "hot", 90.0, 0).unwrap_err();
        assert_eq!(err, AlertingError::RulePaused);
    }

    #[test]
    fn evaluate_below_threshold_is_ok() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        let ev = p.evaluate("acme", "hot", 70.0, 0).unwrap();
        assert_eq!(ev.state, AlertState::Ok);
    }

    #[test]
    fn evaluate_above_threshold_no_for_alerts_immediately() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        let ev = p.evaluate("acme", "hot", 90.0, 0).unwrap();
        assert_eq!(ev.state, AlertState::Alerting);
    }

    #[test]
    fn evaluate_for_window_pending_then_alerting() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut r = rule("hot", 80.0, Comparator::GreaterThan);
        r.for_seconds = 60;
        p.upsert_rule(admin(), r).unwrap();
        let ev1 = p.evaluate("acme", "hot", 90.0, 0).unwrap();
        assert_eq!(ev1.state, AlertState::Pending);
        let ev2 = p.evaluate("acme", "hot", 90.0, 30).unwrap();
        assert_eq!(ev2.state, AlertState::Pending);
        let ev3 = p.evaluate("acme", "hot", 90.0, 60).unwrap();
        assert_eq!(ev3.state, AlertState::Alerting);
    }

    #[test]
    fn evaluate_back_to_ok_resets_pending() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut r = rule("hot", 80.0, Comparator::GreaterThan);
        r.for_seconds = 60;
        p.upsert_rule(admin(), r).unwrap();
        p.evaluate("acme", "hot", 90.0, 0).unwrap();
        p.evaluate("acme", "hot", 70.0, 30).unwrap();
        // recovers, pending should clear
        assert!(!p.pending_since.contains_key("hot"));
        let ev = p.evaluate("acme", "hot", 90.0, 60).unwrap();
        assert_eq!(ev.state, AlertState::Pending);
    }

    #[test]
    fn evaluate_nan_returns_no_data() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        let ev = p.evaluate("acme", "hot", f64::NAN, 0).unwrap();
        assert_eq!(ev.state, AlertState::NoData);
    }

    #[test]
    fn evaluate_message_includes_value() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        let ev = p.evaluate("acme", "hot", 95.5, 0).unwrap();
        assert!(ev.message.contains("95.5"));
    }

    #[test]
    fn evaluations_for_returns_descending_by_time() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        for t in [0, 100, 50] {
            p.evaluate("acme", "hot", 90.0, t).unwrap();
        }
        let evs = p.evaluations_for("hot", 10);
        let times: Vec<u64> = evs.iter().map(|e| e.at_unix).collect();
        assert_eq!(times, vec![100, 50, 0]);
    }

    #[test]
    fn alerting_rules_lists_currently_firing() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        p.upsert_rule(admin(), rule("cold", 10.0, Comparator::LessThan)).unwrap();
        p.evaluate("acme", "hot", 90.0, 0).unwrap();
        p.evaluate("acme", "cold", 50.0, 0).unwrap(); // not firing
        let firing: Vec<&str> = p.alerting_rules("acme").iter().map(|r| r.id.as_str()).collect();
        assert_eq!(firing, vec!["hot"]);
    }

    #[test]
    fn alerting_rules_excludes_recovered() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_rule(admin(), rule("hot", 80.0, Comparator::GreaterThan)).unwrap();
        p.evaluate("acme", "hot", 90.0, 0).unwrap();
        p.evaluate("acme", "hot", 70.0, 60).unwrap();
        assert!(p.alerting_rules("acme").is_empty());
    }

    // ── Annotations ─────────────────────────────────────────────────────────

    fn annotation(id: &str, dash: Option<&str>, at: u64) -> Annotation {
        Annotation {
            id: id.into(),
            tenant: "acme".into(),
            dashboard_id: dash.map(String::from),
            panel_id: None,
            at_unix: at,
            end_unix: None,
            text: format!("ev {id}"),
            tags: Vec::new(),
        }
    }

    #[test]
    fn annotation_add_basic() {
        let mut p = GrafanaAlertingPlugin::new();
        p.add_annotation(annotation("a", Some("d1"), 100)).unwrap();
        assert_eq!(p.count_annotations(), 1);
    }

    #[test]
    fn annotation_replaces_same_id() {
        let mut p = GrafanaAlertingPlugin::new();
        p.add_annotation(annotation("a", Some("d1"), 100)).unwrap();
        let mut a2 = annotation("a", Some("d1"), 200);
        a2.text = "updated".into();
        p.add_annotation(a2).unwrap();
        assert_eq!(p.count_annotations(), 1);
    }

    #[test]
    fn annotation_invalid_id() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.add_annotation(annotation("BAD", None, 0)).unwrap_err();
        assert!(matches!(err, AlertingError::InvalidId(_)));
    }

    #[test]
    fn annotation_bad_range_rejected() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut a = annotation("a", None, 100);
        a.end_unix = Some(50);
        let err = p.add_annotation(a).unwrap_err();
        assert_eq!(err, AlertingError::BadAnnotationRange);
    }

    #[test]
    fn annotation_for_dashboard_filters_by_time() {
        let mut p = GrafanaAlertingPlugin::new();
        for (id, t) in [("a", 50), ("b", 150), ("c", 250)] {
            p.add_annotation(annotation(id, Some("d1"), t)).unwrap();
        }
        let out = p.annotations_for_dashboard("acme", "d1", 100, 200);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "b");
    }

    #[test]
    fn annotation_for_dashboard_filters_by_dashboard() {
        let mut p = GrafanaAlertingPlugin::new();
        p.add_annotation(annotation("a", Some("d1"), 100)).unwrap();
        p.add_annotation(annotation("b", Some("d2"), 100)).unwrap();
        assert_eq!(p.annotations_for_dashboard("acme", "d1", 0, 1000).len(), 1);
    }

    #[test]
    fn annotation_by_tag_filters() {
        let mut p = GrafanaAlertingPlugin::new();
        let mut a = annotation("a", None, 0);
        a.tags = vec!["deploy".into()];
        p.add_annotation(a).unwrap();
        let mut b = annotation("b", None, 0);
        b.tags = vec!["incident".into()];
        p.add_annotation(b).unwrap();
        assert_eq!(p.annotations_by_tag("acme", "deploy").len(), 1);
    }

    // ── Channels ────────────────────────────────────────────────────────────

    fn chan(id: &str, kind: ChannelKind, target: &str, min: AlertSeverity) -> NotificationChannel {
        NotificationChannel {
            id: id.into(),
            tenant: "acme".into(),
            kind,
            name: id.into(),
            target: target.into(),
            min_severity: min,
        }
    }

    #[test]
    fn channel_upsert_admin_succeeds() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_channel(admin(), chan("oncall", ChannelKind::Pager, "team-a", AlertSeverity::Warning)).unwrap();
        assert_eq!(p.count_channels(), 1);
    }

    #[test]
    fn channel_upsert_operator_denied() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_channel(op(), chan("x", ChannelKind::Chat, "#alerts", AlertSeverity::Info)).unwrap_err();
        assert!(matches!(err, AlertingError::Forbidden(_)));
    }

    #[test]
    fn channel_empty_target_rejected() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_channel(admin(), chan("x", ChannelKind::Webhook, "", AlertSeverity::Info)).unwrap_err();
        assert!(matches!(err, AlertingError::EmptyTarget(_)));
    }

    #[test]
    fn channel_invalid_id() {
        let mut p = GrafanaAlertingPlugin::new();
        let err = p.upsert_channel(admin(), chan("BAD", ChannelKind::Email, "x@y", AlertSeverity::Info)).unwrap_err();
        assert!(matches!(err, AlertingError::InvalidId(_)));
    }

    // ── Fanout ──────────────────────────────────────────────────────────────

    fn populated_with_routing() -> GrafanaAlertingPlugin {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_channel(admin(), chan("low", ChannelKind::Chat, "#general", AlertSeverity::Info)).unwrap();
        p.upsert_channel(admin(), chan("oncall", ChannelKind::Pager, "team-a", AlertSeverity::Warning)).unwrap();
        p.upsert_channel(admin(), chan("crit", ChannelKind::Pager, "exec", AlertSeverity::Critical)).unwrap();
        let mut r = rule("hot", 80.0, Comparator::GreaterThan);
        r.severity = AlertSeverity::Warning;
        r.channel_ids = vec!["low".into(), "oncall".into(), "crit".into()];
        p.upsert_rule(admin(), r).unwrap();
        p
    }

    #[test]
    fn fanout_alerting_state_required() {
        let mut p = populated_with_routing();
        let ev = p.evaluate("acme", "hot", 70.0, 0).unwrap();
        let out = p.fanout("acme", &ev);
        assert!(out.is_empty());
    }

    #[test]
    fn fanout_severity_floor_applied() {
        let mut p = populated_with_routing();
        let ev = p.evaluate("acme", "hot", 90.0, 0).unwrap();
        // rule sev = Warning, channel mins are info/warning/critical;
        // critical channel should be EXCLUDED.
        let names: Vec<&str> = p.fanout("acme", &ev).iter().map(|c| c.id.as_str()).collect();
        assert!(names.contains(&"low"));
        assert!(names.contains(&"oncall"));
        assert!(!names.contains(&"crit"));
    }

    #[test]
    fn fanout_only_listed_channels() {
        let mut p = GrafanaAlertingPlugin::new();
        p.upsert_channel(admin(), chan("a", ChannelKind::Chat, "#a", AlertSeverity::Info)).unwrap();
        p.upsert_channel(admin(), chan("b", ChannelKind::Chat, "#b", AlertSeverity::Info)).unwrap();
        let mut r = rule("hot", 80.0, Comparator::GreaterThan);
        r.severity = AlertSeverity::Critical;
        r.channel_ids = vec!["a".into()]; // only a
        p.upsert_rule(admin(), r).unwrap();
        let ev = p.evaluate("acme", "hot", 90.0, 0).unwrap();
        let names: Vec<&str> = p.fanout("acme", &ev).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn fanout_unknown_rule_no_routing() {
        let p = GrafanaAlertingPlugin::new();
        let ev = AlertEvaluation {
            rule_id: "ghost".into(),
            at_unix: 0,
            state: AlertState::Alerting,
            value: 0.0,
            message: "x".into(),
        };
        assert!(p.fanout("acme", &ev).is_empty());
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn alert_severity_serializes() {
        let s = serde_json::to_string(&AlertSeverity::Critical).unwrap();
        assert_eq!(s, "\"critical\"");
    }

    #[test]
    fn channel_kind_serializes() {
        let s = serde_json::to_string(&ChannelKind::Webhook).unwrap();
        assert_eq!(s, "\"webhook\"");
    }

    #[test]
    fn alert_state_serializes() {
        let s = serde_json::to_string(&AlertState::Alerting).unwrap();
        assert_eq!(s, "\"alerting\"");
    }

    #[test]
    fn folder_round_trips_json() {
        let f = Folder::new("ops", "acme", "Ops").with_parent("root");
        let s = serde_json::to_string(&f).unwrap();
        let back: Folder = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn alert_rule_round_trips_json() {
        let r = rule("hot", 80.5, Comparator::GreaterThan);
        let s = serde_json::to_string(&r).unwrap();
        let back: AlertRule = serde_json::from_str(&s).unwrap();
        assert_eq!(back, r);
    }
}
