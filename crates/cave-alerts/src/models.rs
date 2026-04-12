use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Alert {
    pub id: Uuid,
    pub name: String,
    pub labels: std::collections::HashMap<String, String>,
    pub annotations: std::collections::HashMap<String, String>,
    pub severity: AlertSeverity,
    pub state: AlertState,
    pub starts_at: DateTime<Utc>,
    pub ends_at: Option<DateTime<Utc>>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Firing,
    Resolved,
    Silenced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub id: Uuid,
    pub name: String,
    pub matchers: Vec<Matcher>,
    pub receivers: Vec<String>,
    pub continue_matching: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matcher {
    pub label: String,
    pub value: String,
    pub is_regex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Silence {
    pub id: Uuid,
    pub matchers: Vec<Matcher>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub created_by: String,
    pub comment: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_alert() -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: "HighCPU".to_string(),
            labels: {
                let mut m = HashMap::new();
                m.insert("env".to_string(), "prod".to_string());
                m.insert("team".to_string(), "platform".to_string());
                m
            },
            annotations: {
                let mut m = HashMap::new();
                m.insert("summary".to_string(), "CPU is high".to_string());
                m
            },
            severity: AlertSeverity::Critical,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: "abc123".to_string(),
        }
    }

    #[test]
    fn test_alert_serde_roundtrip() {
        let alert = make_alert();
        let json = serde_json::to_string(&alert).unwrap();
        let restored: Alert = serde_json::from_str(&json).unwrap();
        assert_eq!(alert, restored);
    }

    #[test]
    fn test_alert_severity_serde() {
        let s = serde_json::to_string(&AlertSeverity::Critical).unwrap();
        assert_eq!(s, "\"critical\"");
        let restored: AlertSeverity = serde_json::from_str(&s).unwrap();
        assert_eq!(restored, AlertSeverity::Critical);
    }

    #[test]
    fn test_alert_state_serde() {
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
    fn test_route_serde_roundtrip() {
        let route = Route {
            id: Uuid::new_v4(),
            name: "critical-route".to_string(),
            matchers: vec![Matcher {
                label: "severity".to_string(),
                value: "critical".to_string(),
                is_regex: false,
            }],
            receivers: vec!["pagerduty".to_string()],
            continue_matching: false,
        };
        let json = serde_json::to_string(&route).unwrap();
        let restored: Route = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.name, route.name);
        assert_eq!(restored.receivers, route.receivers);
        assert_eq!(restored.continue_matching, route.continue_matching);
    }

    #[test]
    fn test_silence_serde_roundtrip() {
        let silence = Silence {
            id: Uuid::new_v4(),
            matchers: vec![Matcher {
                label: "env".to_string(),
                value: "staging".to_string(),
                is_regex: false,
            }],
            starts_at: Utc::now(),
            ends_at: Utc::now() + chrono::Duration::hours(2),
            created_by: "alice".to_string(),
            comment: "maintenance window".to_string(),
        };
        let json = serde_json::to_string(&silence).unwrap();
        let restored: Silence = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.created_by, silence.created_by);
        assert_eq!(restored.comment, silence.comment);
    }

    #[test]
    fn test_alert_with_ends_at_serde() {
        let mut alert = make_alert();
        alert.ends_at = Some(Utc::now() + chrono::Duration::minutes(5));
        alert.state = AlertState::Resolved;
        let json = serde_json::to_string(&alert).unwrap();
        let restored: Alert = serde_json::from_str(&json).unwrap();
        assert_eq!(alert, restored);
        assert!(restored.ends_at.is_some());
    }

    #[test]
    fn test_matcher_serde_roundtrip() {
        let m = Matcher {
            label: "namespace".to_string(),
            value: "prod.*".to_string(),
            is_regex: true,
        };
        let json = serde_json::to_string(&m).unwrap();
        let restored: Matcher = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.label, m.label);
        assert_eq!(restored.value, m.value);
        assert!(restored.is_regex);
    }
}
