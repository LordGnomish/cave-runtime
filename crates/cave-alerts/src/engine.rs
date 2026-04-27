//! Top-level pipeline that ties matchers + routing + inhibit + silences +
//! grouping + receivers together. Also keeps the legacy convenience
//! functions used by older callers (`route_alert`, `is_silenced`,
//! `deduplicate`, `compute_fingerprint`, `matcher_matches`) so existing
//! integrations continue to compile.

use crate::matcher;
use crate::models::{Alert, AlertState, InhibitRule, Matcher, Receiver, Route, Silence};
use crate::receivers::{render_all, RenderedNotification};
use crate::routing::{route_alert_tree, RoutingDecision};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

// ─── Legacy compat surface (kept for back-compat with older tests) ─────────

pub fn compute_fingerprint(name: &str, labels: &HashMap<String, String>) -> String {
    matcher::compute_fingerprint(name, labels)
}

pub fn matcher_matches(m: &Matcher, labels: &HashMap<String, String>) -> bool {
    matcher::matcher_matches(m, labels)
}

pub fn route_matches(route: &Route, alert: &Alert) -> bool {
    matcher::all_match(&route.matchers, &alert.labels)
}

pub fn route_alert(alert: &Alert, routes: &[Route]) -> Vec<String> {
    let mut receivers = Vec::new();
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

pub fn is_silenced(alert: &Alert, silences: &[Silence]) -> bool {
    crate::silence::is_silenced(alert, silences, Utc::now())
}

pub fn deduplicate(alerts: Vec<Alert>) -> Vec<Alert> {
    crate::grouping::deduplicate(alerts)
}

// ─── Pipeline ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PipelineInput<'a> {
    pub root_route: &'a Route,
    pub silences: &'a [Silence],
    pub inhibit_rules: &'a [InhibitRule],
    pub receivers: &'a HashMap<String, Receiver>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PipelineGroupOutput {
    pub group_key: String,
    pub decision: RoutingDecision,
    pub firing: Vec<Alert>,
    pub resolved: Vec<Alert>,
    pub notifications: Vec<RenderedNotification>,
}

/// Run a full alert pipeline:
/// 1. Deduplicate alerts.
/// 2. Apply silences (mutates state).
/// 3. Apply inhibit rules.
/// 4. Route each alert through the route tree.
/// 5. Group by `group_key` from the routing decision.
/// 6. Render notifications for each receiver in each group.
pub fn run_pipeline(input: PipelineInput, alerts: Vec<Alert>) -> Vec<PipelineGroupOutput> {
    let alerts = crate::grouping::deduplicate(alerts);
    let mut alerts = alerts;
    crate::silence::apply_silences(&mut alerts, input.silences, input.now);

    let active: Vec<Alert> = alerts.iter().filter(|a| a.state == AlertState::Firing).cloned().collect();
    let post_inhibit = crate::inhibit::filter_inhibited(&active, input.inhibit_rules);

    let mut groups: HashMap<String, PipelineGroupOutput> = HashMap::new();

    for alert in alerts.iter() {
        let decision = route_alert_tree(input.root_route, alert);
        let key = crate::routing::group_key(&decision, alert);
        let inhibited = !post_inhibit.iter().any(|a| a.fingerprint == alert.fingerprint);
        let entry = groups.entry(key.clone()).or_insert_with(|| PipelineGroupOutput {
            group_key: key,
            decision: decision.clone(),
            firing: vec![],
            resolved: vec![],
            notifications: vec![],
        });
        match alert.state {
            AlertState::Firing if !inhibited => entry.firing.push(alert.clone()),
            AlertState::Resolved => entry.resolved.push(alert.clone()),
            _ => {} // silenced or inhibited → don't notify
        }
    }

    for output in groups.values_mut() {
        for receiver_name in &output.decision.receivers {
            if let Some(receiver) = input.receivers.get(receiver_name) {
                let mut rendered = render_all(receiver, &output.firing, &output.resolved);
                output.notifications.append(&mut rendered);
            }
        }
    }

    groups.into_values().collect()
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AlertSeverity, AlertState, Matcher, Receiver, ReceiverConfig, Route, Silence, WebhookConfig,
    };
    use chrono::Duration;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn alert_with(name: &str, labels: Vec<(&str, &str)>, state: AlertState, fp: &str) -> Alert {
        let labels: HashMap<String, String> = labels.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        Alert {
            id: Uuid::new_v4(),
            name: name.into(),
            labels,
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: fp.into(),
            tenant_id: "anonymous".into(),
            generator_url: None,
        }
    }

    fn webhook_receiver(name: &str, url: &str) -> Receiver {
        Receiver::new(name).with_config(ReceiverConfig::Webhook(WebhookConfig {
            url: url.into(),
            send_resolved: true,
        }))
    }

    // ─── Legacy compat tests ─────────────────────────────────────────────

    #[test]
    fn test_legacy_compute_fingerprint_stable() {
        let mut labels = HashMap::new();
        labels.insert("a".into(), "1".into());
        let f1 = compute_fingerprint("X", &labels);
        let f2 = compute_fingerprint("X", &labels);
        assert_eq!(f1, f2);
    }

    #[test]
    fn test_legacy_route_alert_first_wins() {
        let a = alert_with("X", vec![("env", "prod")], AlertState::Firing, "fp1");
        let r1 = Route::child("r1", vec![Matcher::equal("env", "prod")], vec!["pd".into()]);
        let r2 = Route::child("r2", vec![Matcher::equal("env", "prod")], vec!["slack".into()]);
        assert_eq!(route_alert(&a, &[r1, r2]), vec!["pd".to_string()]);
    }

    #[test]
    fn test_legacy_route_alert_continue_collects_all() {
        let a = alert_with("X", vec![("env", "prod")], AlertState::Firing, "fp1");
        let r1 = Route::child("r1", vec![Matcher::equal("env", "prod")], vec!["pd".into()]).with_continue(true);
        let r2 = Route::child("r2", vec![Matcher::equal("env", "prod")], vec!["slack".into()]);
        let recv = route_alert(&a, &[r1, r2]);
        assert!(recv.contains(&"pd".to_string()));
        assert!(recv.contains(&"slack".to_string()));
    }

    #[test]
    fn test_legacy_is_silenced() {
        let a = alert_with("X", vec![("env", "prod")], AlertState::Firing, "fp1");
        let s = Silence::new(
            vec![Matcher::equal("env", "prod")],
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::hours(1),
            "alice",
            "x",
        );
        assert!(is_silenced(&a, &[s]));
    }

    #[test]
    fn test_legacy_deduplicate_passthrough() {
        let a1 = alert_with("X", vec![], AlertState::Firing, "fp1");
        let a2 = alert_with("X", vec![], AlertState::Firing, "fp1");
        let result = deduplicate(vec![a1, a2]);
        assert_eq!(result.len(), 1);
    }

    // ─── Pipeline tests ──────────────────────────────────────────────────

    #[test]
    fn test_pipeline_routes_to_root_default() {
        let root = Route::root("default");
        let receivers = std::iter::once(("default".to_string(), webhook_receiver("default", "http://x"))).collect();
        let firing = alert_with("X", vec![], AlertState::Firing, "fp1");
        let out = run_pipeline(
            PipelineInput {
                root_route: &root,
                silences: &[],
                inhibit_rules: &[],
                receivers: &receivers,
                now: Utc::now(),
            },
            vec![firing],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].notifications.len(), 1);
        assert_eq!(out[0].notifications[0].kind, "webhook");
    }

    #[test]
    fn test_pipeline_silenced_alert_no_notification() {
        let root = Route::root("default");
        let receivers = std::iter::once(("default".to_string(), webhook_receiver("default", "http://x"))).collect();
        let firing = alert_with("X", vec![("env", "prod")], AlertState::Firing, "fp1");
        let silence = Silence::new(
            vec![Matcher::equal("env", "prod")],
            Utc::now() - Duration::minutes(1),
            Utc::now() + Duration::hours(1),
            "alice",
            "deploy",
        );
        let out = run_pipeline(
            PipelineInput {
                root_route: &root,
                silences: &[silence],
                inhibit_rules: &[],
                receivers: &receivers,
                now: Utc::now(),
            },
            vec![firing],
        );
        assert_eq!(out[0].firing.len(), 0);
        // Notifications array still empty because firing is empty and resolved is empty.
        assert_eq!(out[0].notifications.len(), 0);
    }

    #[test]
    fn test_pipeline_inhibited_alert_excluded() {
        let root = Route::root("default");
        let receivers: HashMap<_, _> = std::iter::once(("default".to_string(), webhook_receiver("default", "http://x"))).collect();
        let cluster_down = alert_with(
            "ClusterDown",
            vec![("cluster", "c1"), ("severity", "critical"), ("alertname", "ClusterDown")],
            AlertState::Firing,
            "fp-cd",
        );
        let pod_high = alert_with(
            "PodHigh",
            vec![("cluster", "c1"), ("severity", "warning")],
            AlertState::Firing,
            "fp-pod",
        );
        let rule = InhibitRule::new(
            "rule",
            vec![Matcher::equal("alertname", "ClusterDown")],
            vec![Matcher::equal("severity", "warning")],
            vec!["cluster".into()],
        );
        let out = run_pipeline(
            PipelineInput {
                root_route: &root,
                silences: &[],
                inhibit_rules: &[rule],
                receivers: &receivers,
                now: Utc::now(),
            },
            vec![cluster_down, pod_high],
        );
        // PodHigh should have been suppressed by inhibit; only ClusterDown notifies.
        let total_firing: usize = out.iter().map(|g| g.firing.len()).sum();
        assert_eq!(total_firing, 1);
    }

    #[test]
    fn test_pipeline_resolved_routed_separately() {
        let root = Route::root("default");
        let receivers: HashMap<_, _> = std::iter::once(("default".to_string(), webhook_receiver("default", "http://x"))).collect();
        let mut a = alert_with("X", vec![], AlertState::Firing, "fp1");
        a.state = AlertState::Resolved;
        let out = run_pipeline(
            PipelineInput {
                root_route: &root,
                silences: &[],
                inhibit_rules: &[],
                receivers: &receivers,
                now: Utc::now(),
            },
            vec![a],
        );
        assert_eq!(out[0].resolved.len(), 1);
        assert_eq!(out[0].firing.len(), 0);
    }

    #[test]
    fn test_pipeline_groups_by_alertname() {
        let root = Route::root("default");
        let receivers: HashMap<_, _> = std::iter::once(("default".to_string(), webhook_receiver("default", "http://x"))).collect();
        let a1 = alert_with("HighCPU", vec![("alertname", "HighCPU"), ("instance", "a")], AlertState::Firing, "fp1");
        let a2 = alert_with("HighCPU", vec![("alertname", "HighCPU"), ("instance", "b")], AlertState::Firing, "fp2");
        let a3 = alert_with("HighMem", vec![("alertname", "HighMem"), ("instance", "a")], AlertState::Firing, "fp3");
        let out = run_pipeline(
            PipelineInput {
                root_route: &root,
                silences: &[],
                inhibit_rules: &[],
                receivers: &receivers,
                now: Utc::now(),
            },
            vec![a1, a2, a3],
        );
        // Two groups: HighCPU (2 alerts) and HighMem (1 alert)
        assert_eq!(out.len(), 2);
        let cpu_group = out.iter().find(|g| g.firing.iter().any(|a| a.name == "HighCPU")).unwrap();
        assert_eq!(cpu_group.firing.len(), 2);
    }

    #[test]
    fn test_pipeline_unknown_receiver_skipped() {
        let mut root = Route::root("default");
        root.receivers = vec!["does-not-exist".into()];
        let receivers: HashMap<String, Receiver> = HashMap::new();
        let firing = alert_with("X", vec![], AlertState::Firing, "fp1");
        let out = run_pipeline(
            PipelineInput {
                root_route: &root,
                silences: &[],
                inhibit_rules: &[],
                receivers: &receivers,
                now: Utc::now(),
            },
            vec![firing],
        );
        assert_eq!(out[0].notifications.len(), 0);
    }
}
