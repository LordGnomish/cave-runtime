// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub const DEFAULT_TENANT: &str = "anonymous";
pub const TENANT_LABEL: &str = "tenant_id";

// ─── Alerts ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alert {
    pub id: Uuid,
    pub name: String,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub severity: AlertSeverity,
    pub state: AlertState,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub fingerprint: String,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    #[serde(default)]
    pub generator_url: Option<String>,
}

fn default_tenant() -> String {
    DEFAULT_TENANT.to_string()
}

impl Alert {
    /// Construct a fresh alert from a name + labels; generates fingerprint and id.
    pub fn build(name: impl Into<String>, labels: HashMap<String, String>) -> Self {
        let name = name.into();
        let fingerprint = crate::matcher::compute_fingerprint(&name, &labels);
        Alert {
            id: Uuid::new_v4(),
            name,
            labels,
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint,
            tenant_id: DEFAULT_TENANT.to_string(),
            generator_url: None,
        }
    }

    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant_id = tenant.into();
        self.labels.insert(TENANT_LABEL.to_string(), self.tenant_id.clone());
        self
    }

    pub fn with_severity(mut self, severity: AlertSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_annotation(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.annotations.insert(k.into(), v.into());
        self
    }

    /// Whether this alert is currently active (firing and not yet ended).
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        matches!(self.state, AlertState::Firing) && self.ends_at.map_or(true, |e| e > now)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Firing,
    Resolved,
    Silenced,
}

// ─── Matchers ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    Equal,
    NotEqual,
    Regex,
    NotRegex,
}

impl Default for MatchType {
    fn default() -> Self {
        MatchType::Equal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matcher {
    pub label: String,
    pub value: String,
    /// Legacy boolean form — true means MatchType::Regex, false means Equal.
    /// Kept to preserve compatibility with the original models.
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub match_type: MatchType,
}

impl Matcher {
    pub fn equal(label: impl Into<String>, value: impl Into<String>) -> Self {
        Matcher {
            label: label.into(),
            value: value.into(),
            is_regex: false,
            match_type: MatchType::Equal,
        }
    }

    pub fn not_equal(label: impl Into<String>, value: impl Into<String>) -> Self {
        Matcher {
            label: label.into(),
            value: value.into(),
            is_regex: false,
            match_type: MatchType::NotEqual,
        }
    }

    pub fn regex(label: impl Into<String>, value: impl Into<String>) -> Self {
        Matcher {
            label: label.into(),
            value: value.into(),
            is_regex: true,
            match_type: MatchType::Regex,
        }
    }

    pub fn not_regex(label: impl Into<String>, value: impl Into<String>) -> Self {
        Matcher {
            label: label.into(),
            value: value.into(),
            is_regex: true,
            match_type: MatchType::NotRegex,
        }
    }

    /// Effective match type: prefers explicit `match_type`, falls back to legacy `is_regex`.
    pub fn effective_type(&self) -> MatchType {
        match (&self.match_type, self.is_regex) {
            (MatchType::Equal, true) => MatchType::Regex,
            (mt, _) => mt.clone(),
        }
    }
}

// ─── Routes (hierarchical) ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub matchers: Vec<Matcher>,
    #[serde(default)]
    pub receivers: Vec<String>,
    #[serde(default)]
    pub continue_matching: bool,
    #[serde(default)]
    pub group_by: Vec<String>,
    /// Wait this long after the first alert in a group before notifying.
    #[serde(default, with = "humantime_serde_opt")]
    pub group_wait: Option<Duration>,
    /// Wait this long before notifying about NEW alerts that join an existing group.
    #[serde(default, with = "humantime_serde_opt")]
    pub group_interval: Option<Duration>,
    /// Wait this long before re-notifying about an unchanged firing group.
    #[serde(default, with = "humantime_serde_opt")]
    pub repeat_interval: Option<Duration>,
    /// Tenant this route applies to. If None, applies to all tenants.
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Child routes (Alertmanager-style hierarchy).
    #[serde(default)]
    pub routes: Vec<Route>,
}

impl Route {
    pub fn root(receiver: impl Into<String>) -> Self {
        Route {
            id: Uuid::new_v4(),
            name: "root".to_string(),
            matchers: vec![],
            receivers: vec![receiver.into()],
            continue_matching: false,
            group_by: vec!["alertname".to_string()],
            group_wait: Some(Duration::seconds(30)),
            group_interval: Some(Duration::minutes(5)),
            repeat_interval: Some(Duration::hours(4)),
            tenant_id: None,
            routes: vec![],
        }
    }

    pub fn child(name: impl Into<String>, matchers: Vec<Matcher>, receivers: Vec<String>) -> Self {
        Route {
            id: Uuid::new_v4(),
            name: name.into(),
            matchers,
            receivers,
            continue_matching: false,
            group_by: vec![],
            group_wait: None,
            group_interval: None,
            repeat_interval: None,
            tenant_id: None,
            routes: vec![],
        }
    }

    pub fn with_group_by(mut self, labels: Vec<String>) -> Self {
        self.group_by = labels;
        self
    }

    pub fn with_continue(mut self, c: bool) -> Self {
        self.continue_matching = c;
        self
    }

    pub fn with_child(mut self, child: Route) -> Self {
        self.routes.push(child);
        self
    }
}

// Helpers for serde Duration<Option> in human readable form.
mod humantime_serde_opt {
    use chrono::Duration;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_str(&format_duration(*d)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let s: Option<String> = Option::deserialize(d)?;
        match s {
            Some(s) => parse_duration(&s).map(Some).map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }

    fn format_duration(d: Duration) -> String {
        let secs = d.num_seconds();
        if secs % 3600 == 0 {
            format!("{}h", secs / 3600)
        } else if secs % 60 == 0 {
            format!("{}m", secs / 60)
        } else {
            format!("{}s", secs)
        }
    }

    pub fn parse_duration(s: &str) -> Result<Duration, String> {
        if let Some(rest) = s.strip_suffix('h') {
            rest.parse::<i64>().map(Duration::hours).map_err(|e| e.to_string())
        } else if let Some(rest) = s.strip_suffix('m') {
            rest.parse::<i64>().map(Duration::minutes).map_err(|e| e.to_string())
        } else if let Some(rest) = s.strip_suffix('s') {
            rest.parse::<i64>().map(Duration::seconds).map_err(|e| e.to_string())
        } else {
            s.parse::<i64>().map(Duration::seconds).map_err(|e| e.to_string())
        }
    }
}

pub use humantime_serde_opt::parse_duration;

// ─── Silences ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Silence {
    pub id: Uuid,
    pub matchers: Vec<Matcher>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub created_by: String,
    pub comment: String,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
}

impl Silence {
    pub fn new(
        matchers: Vec<Matcher>,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
        created_by: impl Into<String>,
        comment: impl Into<String>,
    ) -> Self {
        Silence {
            id: Uuid::new_v4(),
            matchers,
            starts_at,
            ends_at,
            created_by: created_by.into(),
            comment: comment.into(),
            tenant_id: DEFAULT_TENANT.to_string(),
        }
    }

    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.starts_at <= now && now <= self.ends_at
    }
}

// ─── Inhibit Rules ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InhibitRule {
    pub id: Uuid,
    pub name: String,
    /// If a "source" alert exists matching these, target alerts are inhibited.
    pub source_matchers: Vec<Matcher>,
    /// Only target alerts matching these are inhibited.
    pub target_matchers: Vec<Matcher>,
    /// Labels that must be equal between source and target for the inhibit
    /// to apply (Alertmanager's `equal` field).
    #[serde(default)]
    pub equal: Vec<String>,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
}

impl InhibitRule {
    pub fn new(
        name: impl Into<String>,
        source: Vec<Matcher>,
        target: Vec<Matcher>,
        equal: Vec<String>,
    ) -> Self {
        InhibitRule {
            id: Uuid::new_v4(),
            name: name.into(),
            source_matchers: source,
            target_matchers: target,
            equal,
            tenant_id: DEFAULT_TENANT.to_string(),
        }
    }
}

// ─── Receivers ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReceiverConfig {
    Webhook(WebhookConfig),
    Slack(SlackConfig),
    Email(EmailConfig),
    PagerDuty(PagerDutyConfig),
    OpsGenie(OpsGenieConfig),
    GrafanaOnCall(GrafanaOnCallConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    #[serde(default = "default_true")]
    pub send_resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackConfig {
    pub webhook_url: String,
    pub channel: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default = "default_true")]
    pub send_resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    pub to: Vec<String>,
    pub from: String,
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub auth_username: Option<String>,
    #[serde(default)]
    pub auth_password: Option<String>,
    #[serde(default = "default_true")]
    pub require_tls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PagerDutyConfig {
    pub routing_key: String,
    #[serde(default = "default_true")]
    pub send_resolved: bool,
    #[serde(default)]
    pub severity_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsGenieConfig {
    pub api_key: String,
    #[serde(default = "default_opsgenie_url")]
    pub api_url: String,
    #[serde(default = "default_true")]
    pub send_resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrafanaOnCallConfig {
    pub webhook_url: String,
    #[serde(default = "default_true")]
    pub send_resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receiver {
    pub name: String,
    #[serde(default)]
    pub configs: Vec<ReceiverConfig>,
}

impl Receiver {
    pub fn new(name: impl Into<String>) -> Self {
        Receiver { name: name.into(), configs: vec![] }
    }

    pub fn with_config(mut self, c: ReceiverConfig) -> Self {
        self.configs.push(c);
        self
    }
}

fn default_true() -> bool { true }
fn default_smtp_port() -> u16 { 587 }
fn default_opsgenie_url() -> String { "https://api.opsgenie.com".to_string() }

// ─── Notification log entries ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationLogEntry {
    pub group_key: String,
    pub receiver: String,
    pub sent_at: DateTime<Utc>,
    pub firing_fingerprints: Vec<String>,
    pub resolved_fingerprints: Vec<String>,
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn base_labels() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("env".to_string(), "prod".to_string());
        m.insert("team".to_string(), "platform".to_string());
        m
    }

    fn sample_alert() -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: "HighCPU".to_string(),
            labels: base_labels(),
            annotations: HashMap::new(),
            severity: AlertSeverity::Critical,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: "abc123".to_string(),
            tenant_id: DEFAULT_TENANT.to_string(),
            generator_url: None,
        }
    }

    #[test]
    fn test_alert_build_assigns_fingerprint_and_id() {
        let a = Alert::build("X", base_labels());
        assert!(!a.fingerprint.is_empty());
        assert!(!a.id.is_nil());
    }

    #[test]
    fn test_alert_with_tenant_sets_label() {
        let a = Alert::build("Y", base_labels()).with_tenant("acme");
        assert_eq!(a.tenant_id, "acme");
        assert_eq!(a.labels.get(TENANT_LABEL), Some(&"acme".to_string()));
    }

    #[test]
    fn test_alert_is_active_default() {
        let a = sample_alert();
        assert!(a.is_active(Utc::now()));
    }

    #[test]
    fn test_alert_is_active_resolved_false() {
        let mut a = sample_alert();
        a.state = AlertState::Resolved;
        assert!(!a.is_active(Utc::now()));
    }

    #[test]
    fn test_alert_is_active_ended_false() {
        let mut a = sample_alert();
        a.ends_at = Some(Utc::now() - Duration::seconds(1));
        assert!(!a.is_active(Utc::now()));
    }

    #[test]
    fn test_alert_serde_roundtrip() {
        let a = sample_alert();
        let json = serde_json::to_string(&a).unwrap();
        let restored: Alert = serde_json::from_str(&json).unwrap();
        assert_eq!(a, restored);
    }

    #[test]
    fn test_alert_severity_serde() {
        let s = serde_json::to_string(&AlertSeverity::Critical).unwrap();
        assert_eq!(s, "\"critical\"");
        let restored: AlertSeverity = serde_json::from_str(&s).unwrap();
        assert_eq!(restored, AlertSeverity::Critical);
    }

    #[test]
    fn test_alert_state_serde_all_variants() {
        for (variant, expected) in [
            (AlertState::Firing, "\"firing\""),
            (AlertState::Resolved, "\"resolved\""),
            (AlertState::Silenced, "\"silenced\""),
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected);
            let restored: AlertState = serde_json::from_str(&s).unwrap();
            assert_eq!(restored, variant);
        }
    }

    #[test]
    fn test_matcher_constructors() {
        assert_eq!(Matcher::equal("a", "b").effective_type(), MatchType::Equal);
        assert_eq!(Matcher::not_equal("a", "b").effective_type(), MatchType::NotEqual);
        assert_eq!(Matcher::regex("a", "b").effective_type(), MatchType::Regex);
        assert_eq!(Matcher::not_regex("a", "b").effective_type(), MatchType::NotRegex);
    }

    #[test]
    fn test_matcher_legacy_is_regex_true_means_regex() {
        let m = Matcher {
            label: "a".to_string(),
            value: "b".to_string(),
            is_regex: true,
            match_type: MatchType::Equal,
        };
        assert_eq!(m.effective_type(), MatchType::Regex);
    }

    #[test]
    fn test_route_serde_roundtrip() {
        let route = Route::child("alpha", vec![Matcher::equal("a", "b")], vec!["slack".into()])
            .with_group_by(vec!["alertname".into()])
            .with_continue(true);
        let json = serde_json::to_string(&route).unwrap();
        let restored: Route = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, route.name);
        assert_eq!(restored.continue_matching, true);
        assert_eq!(restored.group_by, vec!["alertname".to_string()]);
    }

    #[test]
    fn test_route_root_has_defaults() {
        let r = Route::root("default");
        assert_eq!(r.receivers, vec!["default".to_string()]);
        assert!(r.group_wait.is_some());
        assert!(r.group_interval.is_some());
        assert!(r.repeat_interval.is_some());
    }

    #[test]
    fn test_route_with_child() {
        let r = Route::root("default")
            .with_child(Route::child("crit", vec![Matcher::equal("severity", "critical")], vec!["pd".into()]));
        assert_eq!(r.routes.len(), 1);
        assert_eq!(r.routes[0].name, "crit");
    }

    #[test]
    fn test_silence_active_window() {
        let now = Utc::now();
        let s = Silence::new(vec![], now - Duration::minutes(5), now + Duration::minutes(5), "alice", "x");
        assert!(s.is_active_at(now));
        assert!(!s.is_active_at(now - Duration::minutes(10)));
        assert!(!s.is_active_at(now + Duration::minutes(10)));
    }

    #[test]
    fn test_silence_serde_roundtrip() {
        let s = Silence::new(
            vec![Matcher::equal("env", "staging")],
            Utc::now(),
            Utc::now() + Duration::hours(2),
            "bob",
            "deploy",
        );
        let json = serde_json::to_string(&s).unwrap();
        let restored: Silence = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.created_by, s.created_by);
        assert_eq!(restored.comment, s.comment);
    }

    #[test]
    fn test_inhibit_rule_serde() {
        let rule = InhibitRule::new(
            "cluster-down-suppresses-pod",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".to_string()],
        );
        let json = serde_json::to_string(&rule).unwrap();
        let restored: InhibitRule = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, "cluster-down-suppresses-pod");
        assert_eq!(restored.equal, vec!["cluster".to_string()]);
    }

    #[test]
    fn test_receiver_with_configs() {
        let r = Receiver::new("oncall")
            .with_config(ReceiverConfig::Webhook(WebhookConfig {
                url: "https://example.com/hook".into(),
                send_resolved: true,
            }))
            .with_config(ReceiverConfig::Slack(SlackConfig {
                webhook_url: "https://hooks.slack.com/T/B/X".into(),
                channel: "#alerts".into(),
                username: Some("CAVE".into()),
                send_resolved: true,
            }));
        assert_eq!(r.configs.len(), 2);
    }

    #[test]
    fn test_receiver_serde_tag_field() {
        let r = Receiver::new("pd").with_config(ReceiverConfig::PagerDuty(PagerDutyConfig {
            routing_key: "k".into(),
            send_resolved: true,
            severity_label: Some("severity".into()),
        }));
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"type\":\"pager_duty\""));
    }

    #[test]
    fn test_duration_parse_seconds() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::seconds(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::minutes(5));
        assert_eq!(parse_duration("2h").unwrap(), Duration::hours(2));
        assert_eq!(parse_duration("90").unwrap(), Duration::seconds(90));
    }

    #[test]
    fn test_duration_parse_bad() {
        assert!(parse_duration("nope").is_err());
        assert!(parse_duration("xyzh").is_err());
    }

    #[test]
    fn test_default_tenant_value() {
        let s = serde_json::from_str::<Alert>(
            r#"{"id":"00000000-0000-0000-0000-000000000000","name":"X","labels":{},"annotations":{},"severity":"warning","state":"firing","starts_at":"2024-01-01T00:00:00Z","ends_at":null,"fingerprint":"x"}"#
        ).unwrap();
        assert_eq!(s.tenant_id, DEFAULT_TENANT);
    }
}
