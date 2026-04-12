use crate::models::{Alert, AlertState, Matcher, Route, Silence};
use chrono::Utc;

/// Compute a stable fingerprint for an alert based on its name + sorted labels
pub fn compute_fingerprint(name: &str, labels: &std::collections::HashMap<String, String>) -> String {
    let mut parts: Vec<String> = labels.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    parts.sort();
    format!("{name}:{}", parts.join(","))
}

/// Check if a matcher matches a label map
pub fn matcher_matches(matcher: &Matcher, labels: &std::collections::HashMap<String, String>) -> bool {
    match labels.get(&matcher.label) {
        Some(value) => {
            if matcher.is_regex {
                // Simple contains check as a lightweight regex substitute
                value.contains(&matcher.value)
            } else {
                value == &matcher.value
            }
        }
        None => false,
    }
}

/// Check if ALL matchers in a route match the alert labels
pub fn route_matches(route: &Route, alert: &Alert) -> bool {
    route.matchers.iter().all(|m| matcher_matches(m, &alert.labels))
}

/// Find all receivers for an alert given a list of routes
pub fn route_alert(alert: &Alert, routes: &[Route]) -> Vec<String> {
    let mut receivers = vec![];
    for route in routes {
        if route_matches(route, alert) {
            receivers.extend(route.receivers.iter().cloned());
            if !route.continue_matching {
                break;
            }
        }
    }
    receivers
}

/// Check if an alert is currently silenced
pub fn is_silenced(alert: &Alert, silences: &[Silence]) -> bool {
    let now = Utc::now();
    silences.iter().any(|s| {
        s.starts_at <= now && now <= s.ends_at
            && s.matchers.iter().all(|m| matcher_matches(m, &alert.labels))
    })
}

/// Deduplicate alerts by fingerprint, keeping the most recent starts_at
pub fn deduplicate(alerts: Vec<Alert>) -> Vec<Alert> {
    let mut seen: std::collections::HashMap<String, Alert> = std::collections::HashMap::new();
    for alert in alerts {
        seen.entry(alert.fingerprint.clone())
            .and_modify(|existing| {
                if alert.starts_at > existing.starts_at {
                    *existing = alert.clone();
                }
            })
            .or_insert(alert);
    }
    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertSeverity, AlertState, Route};
    use std::collections::HashMap;
    use uuid::Uuid;
    use chrono::Duration;

    fn base_labels() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("env".to_string(), "prod".to_string());
        m.insert("team".to_string(), "platform".to_string());
        m
    }

    fn make_alert(name: &str, labels: HashMap<String, String>, fingerprint: &str) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: name.to_string(),
            labels,
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: fingerprint.to_string(),
        }
    }

    fn make_route(matchers: Vec<Matcher>, receivers: Vec<&str>, cont: bool) -> Route {
        Route {
            id: Uuid::new_v4(),
            name: "route".to_string(),
            matchers,
            receivers: receivers.iter().map(|s| s.to_string()).collect(),
            continue_matching: cont,
        }
    }

    fn exact_matcher(label: &str, value: &str) -> Matcher {
        Matcher { label: label.to_string(), value: value.to_string(), is_regex: false }
    }

    fn regex_matcher(label: &str, value: &str) -> Matcher {
        Matcher { label: label.to_string(), value: value.to_string(), is_regex: true }
    }

    #[test]
    fn test_compute_fingerprint_deterministic() {
        let labels = base_labels();
        let fp1 = compute_fingerprint("HighCPU", &labels);
        let fp2 = compute_fingerprint("HighCPU", &labels);
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_compute_fingerprint_label_order_independent() {
        let mut labels_a = HashMap::new();
        labels_a.insert("env".to_string(), "prod".to_string());
        labels_a.insert("team".to_string(), "platform".to_string());

        // Insert in different order
        let mut labels_b = HashMap::new();
        labels_b.insert("team".to_string(), "platform".to_string());
        labels_b.insert("env".to_string(), "prod".to_string());

        let fp_a = compute_fingerprint("HighCPU", &labels_a);
        let fp_b = compute_fingerprint("HighCPU", &labels_b);
        assert_eq!(fp_a, fp_b);
    }

    #[test]
    fn test_matcher_exact_match() {
        let labels = base_labels();
        let m = exact_matcher("env", "prod");
        assert!(matcher_matches(&m, &labels));
    }

    #[test]
    fn test_matcher_no_match() {
        let labels = base_labels();
        let m = exact_matcher("env", "staging");
        assert!(!matcher_matches(&m, &labels));
    }

    #[test]
    fn test_matcher_missing_label() {
        let labels = base_labels();
        let m = exact_matcher("datacenter", "us-east-1");
        assert!(!matcher_matches(&m, &labels));
    }

    #[test]
    fn test_matcher_regex_contains() {
        let mut labels = HashMap::new();
        labels.insert("namespace".to_string(), "production-api".to_string());
        let m = regex_matcher("namespace", "prod");
        assert!(matcher_matches(&m, &labels));
    }

    #[test]
    fn test_route_matches_all_matchers() {
        let labels = base_labels();
        let alert = make_alert("Test", labels, "fp1");
        let route = make_route(
            vec![exact_matcher("env", "prod"), exact_matcher("team", "platform")],
            vec!["slack"],
            false,
        );
        assert!(route_matches(&route, &alert));
    }

    #[test]
    fn test_route_no_match() {
        let labels = base_labels();
        let alert = make_alert("Test", labels, "fp1");
        let route = make_route(
            vec![exact_matcher("env", "prod"), exact_matcher("team", "security")],
            vec!["slack"],
            false,
        );
        assert!(!route_matches(&route, &alert));
    }

    #[test]
    fn test_route_alert_stops_at_first() {
        let labels = base_labels();
        let alert = make_alert("Test", labels, "fp1");
        let route1 = make_route(vec![exact_matcher("env", "prod")], vec!["pagerduty"], false);
        let route2 = make_route(vec![exact_matcher("env", "prod")], vec!["slack"], false);
        let receivers = route_alert(&alert, &[route1, route2]);
        // Should stop after route1 since continue_matching = false
        assert_eq!(receivers, vec!["pagerduty"]);
    }

    #[test]
    fn test_route_alert_continues() {
        let labels = base_labels();
        let alert = make_alert("Test", labels, "fp1");
        let route1 = make_route(vec![exact_matcher("env", "prod")], vec!["pagerduty"], true);
        let route2 = make_route(vec![exact_matcher("env", "prod")], vec!["slack"], false);
        let receivers = route_alert(&alert, &[route1, route2]);
        assert!(receivers.contains(&"pagerduty".to_string()));
        assert!(receivers.contains(&"slack".to_string()));
    }

    #[test]
    fn test_deduplicate_removes_duplicates() {
        let labels = base_labels();
        let a1 = make_alert("CPU", labels.clone(), "fp-cpu");
        let a2 = make_alert("CPU", labels.clone(), "fp-cpu");
        let a3 = make_alert("Memory", labels.clone(), "fp-mem");
        let result = deduplicate(vec![a1, a2, a3]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_deduplicate_keeps_most_recent() {
        let labels = base_labels();
        let mut older = make_alert("CPU", labels.clone(), "fp-cpu");
        older.starts_at = Utc::now() - Duration::minutes(10);

        let mut newer = make_alert("CPU", labels.clone(), "fp-cpu");
        newer.starts_at = Utc::now();
        // Give different IDs so we can identify which one was kept
        let newer_id = newer.id;

        let result = deduplicate(vec![older, newer]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, newer_id);
    }

    #[test]
    fn test_is_silenced_active_silence() {
        let labels = base_labels();
        let alert = make_alert("Test", labels, "fp1");
        let silence = Silence {
            id: Uuid::new_v4(),
            matchers: vec![exact_matcher("env", "prod")],
            starts_at: Utc::now() - Duration::minutes(5),
            ends_at: Utc::now() + Duration::hours(1),
            created_by: "alice".to_string(),
            comment: "maintenance".to_string(),
        };
        assert!(is_silenced(&alert, &[silence]));
    }

    #[test]
    fn test_is_silenced_expired_silence() {
        let labels = base_labels();
        let alert = make_alert("Test", labels, "fp1");
        let silence = Silence {
            id: Uuid::new_v4(),
            matchers: vec![exact_matcher("env", "prod")],
            starts_at: Utc::now() - Duration::hours(2),
            ends_at: Utc::now() - Duration::hours(1),
            created_by: "alice".to_string(),
            comment: "expired maintenance".to_string(),
        };
        assert!(!is_silenced(&alert, &[silence]));
    }
}
