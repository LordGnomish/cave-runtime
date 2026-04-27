//! Hierarchical route walker — Alertmanager `route` semantics.
//!
//! Walks the route tree top-down. Stops at the first matching child unless
//! `continue` is set. Falls back to the root receivers if no child matches.
//! Effective settings (group_by, group_wait, group_interval, repeat_interval)
//! are inherited from ancestors.

use crate::matcher::all_match;
use crate::models::{Alert, Route};
use chrono::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    pub receivers: Vec<String>,
    pub group_by: Vec<String>,
    pub group_wait: Duration,
    pub group_interval: Duration,
    pub repeat_interval: Duration,
}

#[derive(Debug, Clone)]
struct Inherited {
    group_by: Vec<String>,
    group_wait: Duration,
    group_interval: Duration,
    repeat_interval: Duration,
}

const DEFAULT_GROUP_WAIT: i64 = 30; // seconds
const DEFAULT_GROUP_INTERVAL_SECS: i64 = 5 * 60;
const DEFAULT_REPEAT_INTERVAL_SECS: i64 = 4 * 60 * 60;

fn root_inherited(root: &Route) -> Inherited {
    Inherited {
        group_by: if root.group_by.is_empty() { vec!["alertname".into()] } else { root.group_by.clone() },
        group_wait: root.group_wait.unwrap_or_else(|| Duration::seconds(DEFAULT_GROUP_WAIT)),
        group_interval: root
            .group_interval
            .unwrap_or_else(|| Duration::seconds(DEFAULT_GROUP_INTERVAL_SECS)),
        repeat_interval: root
            .repeat_interval
            .unwrap_or_else(|| Duration::seconds(DEFAULT_REPEAT_INTERVAL_SECS)),
    }
}

fn merge_inherited(parent: &Inherited, child: &Route) -> Inherited {
    Inherited {
        group_by: if child.group_by.is_empty() { parent.group_by.clone() } else { child.group_by.clone() },
        group_wait: child.group_wait.unwrap_or(parent.group_wait),
        group_interval: child.group_interval.unwrap_or(parent.group_interval),
        repeat_interval: child.repeat_interval.unwrap_or(parent.repeat_interval),
    }
}

/// Walk the route tree and return all matched receivers + effective grouping.
///
/// Tenant filtering: a route with `tenant_id = Some(t)` matches only alerts
/// whose `tenant_id` equals `t`. `tenant_id = None` matches any tenant.
pub fn route_alert_tree(root: &Route, alert: &Alert) -> RoutingDecision {
    let inherited = root_inherited(root);
    let mut receivers = Vec::new();
    // Track the deepest matched node's effective inherited settings; when
    // children match, their settings override those of the root.
    let mut leaf_inherited: Inherited = inherited.clone();

    walk(root, alert, &inherited, &mut receivers, &mut leaf_inherited, true);

    // Default to root receivers if nothing was pushed (no match anywhere).
    if receivers.is_empty() {
        receivers.extend(root.receivers.iter().cloned());
    }

    // Always dedupe receivers, preserve order.
    let mut seen = std::collections::HashSet::new();
    receivers.retain(|r| seen.insert(r.clone()));

    RoutingDecision {
        receivers,
        group_by: leaf_inherited.group_by,
        group_wait: leaf_inherited.group_wait,
        group_interval: leaf_inherited.group_interval,
        repeat_interval: leaf_inherited.repeat_interval,
    }
}

fn walk(
    route: &Route,
    alert: &Alert,
    inherited: &Inherited,
    out: &mut Vec<String>,
    leaf: &mut Inherited,
    is_root: bool,
) -> bool {
    if !route_self_matches(route, alert) {
        return false;
    }

    let effective = if is_root {
        inherited.clone()
    } else {
        merge_inherited(inherited, route)
    };

    let mut child_matched = false;
    for child in &route.routes {
        if !route_self_matches(child, alert) {
            continue;
        }
        child_matched = true;
        walk(child, alert, &effective, out, leaf, false);
        if !child.continue_matching {
            break;
        }
    }

    if !child_matched {
        // We're a leaf for this branch — record our settings + receivers.
        out.extend(route.receivers.iter().cloned());
        *leaf = effective;
    }

    true
}

fn route_self_matches(route: &Route, alert: &Alert) -> bool {
    if let Some(t) = &route.tenant_id {
        if t != &alert.tenant_id {
            return false;
        }
    }
    all_match(&route.matchers, &alert.labels)
}

/// Compute a stable group key for an alert under a routing decision.
/// Uses the `group_by` labels from the decision; missing labels yield "".
pub fn group_key(decision: &RoutingDecision, alert: &Alert) -> String {
    let mut parts: Vec<String> = decision
        .group_by
        .iter()
        .map(|k| {
            let v = if k == "alertname" {
                alert.name.clone()
            } else {
                alert.labels.get(k).cloned().unwrap_or_default()
            };
            format!("{k}={v}")
        })
        .collect();
    parts.sort();
    // Include receivers + tenant in the key so different routes don't collide.
    let recv = decision.receivers.join(",");
    format!("tenant={};recv={};{}", alert.tenant_id, recv, parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Alert, AlertSeverity, AlertState, Matcher, Route};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn alert(name: &str, labels: Vec<(&str, &str)>) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            name: name.into(),
            labels: labels.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: "fp".into(),
            tenant_id: "anonymous".into(),
            generator_url: None,
        }
    }

    #[test]
    fn test_root_only_falls_back_to_root_receiver() {
        let root = Route::root("default");
        let a = alert("X", vec![]);
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.receivers, vec!["default".to_string()]);
    }

    #[test]
    fn test_child_match_overrides_root_receiver() {
        let root = Route::root("default").with_child(Route::child(
            "crit",
            vec![Matcher::equal("severity", "critical")],
            vec!["pagerduty".into()],
        ));
        let a = alert("X", vec![("severity", "critical")]);
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.receivers, vec!["pagerduty".to_string()]);
    }

    #[test]
    fn test_child_no_match_falls_back_to_root() {
        let root = Route::root("default").with_child(Route::child(
            "crit",
            vec![Matcher::equal("severity", "critical")],
            vec!["pagerduty".into()],
        ));
        let a = alert("X", vec![("severity", "warning")]);
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.receivers, vec!["default".to_string()]);
    }

    #[test]
    fn test_continue_matches_multiple_children() {
        let root = Route::root("default")
            .with_child(
                Route::child("a", vec![Matcher::equal("env", "prod")], vec!["slack".into()])
                    .with_continue(true),
            )
            .with_child(Route::child(
                "b",
                vec![Matcher::equal("env", "prod")],
                vec!["webhook".into()],
            ));
        let a = alert("X", vec![("env", "prod")]);
        let d = route_alert_tree(&root, &a);
        assert!(d.receivers.contains(&"slack".to_string()));
        assert!(d.receivers.contains(&"webhook".to_string()));
    }

    #[test]
    fn test_no_continue_stops_after_first_match() {
        let root = Route::root("default")
            .with_child(Route::child("a", vec![Matcher::equal("env", "prod")], vec!["slack".into()]))
            .with_child(Route::child("b", vec![Matcher::equal("env", "prod")], vec!["webhook".into()]));
        let a = alert("X", vec![("env", "prod")]);
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.receivers, vec!["slack".to_string()]);
    }

    #[test]
    fn test_grandchild_inherits_grouping_from_parent() {
        let mut child = Route::child(
            "team",
            vec![Matcher::equal("team", "platform")],
            vec!["team-receiver".into()],
        );
        child.group_by = vec!["service".into()];
        let grandchild = Route::child(
            "crit",
            vec![Matcher::equal("severity", "critical")],
            vec!["pd".into()],
        );
        let child = child.with_child(grandchild);
        let root = Route::root("default").with_child(child);

        let a = alert(
            "X",
            vec![("team", "platform"), ("severity", "critical"), ("service", "auth")],
        );
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.receivers, vec!["pd".to_string()]);
        // Inherited group_by from team route
        assert_eq!(d.group_by, vec!["service".to_string()]);
    }

    #[test]
    fn test_default_group_by_is_alertname() {
        let root = Route::root("default");
        let a = alert("X", vec![]);
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.group_by, vec!["alertname".to_string()]);
    }

    #[test]
    fn test_tenant_isolation_excludes_route() {
        let mut child = Route::child(
            "acme-only",
            vec![],
            vec!["acme-pager".into()],
        );
        child.tenant_id = Some("acme".into());
        let root = Route::root("default").with_child(child);

        let a_anon = alert("X", vec![]);
        let mut a_acme = alert("X", vec![]);
        a_acme.tenant_id = "acme".into();

        assert_eq!(route_alert_tree(&root, &a_anon).receivers, vec!["default".to_string()]);
        assert_eq!(route_alert_tree(&root, &a_acme).receivers, vec!["acme-pager".to_string()]);
    }

    #[test]
    fn test_group_key_uses_alertname_when_present() {
        let d = RoutingDecision {
            receivers: vec!["x".into()],
            group_by: vec!["alertname".into()],
            group_wait: Duration::seconds(0),
            group_interval: Duration::seconds(0),
            repeat_interval: Duration::seconds(0),
        };
        let a = alert("HighCPU", vec![]);
        let k = group_key(&d, &a);
        assert!(k.contains("alertname=HighCPU"));
        assert!(k.contains("recv=x"));
    }

    #[test]
    fn test_group_key_includes_tenant() {
        let d = RoutingDecision {
            receivers: vec!["x".into()],
            group_by: vec!["service".into()],
            group_wait: Duration::seconds(0),
            group_interval: Duration::seconds(0),
            repeat_interval: Duration::seconds(0),
        };
        let mut a = alert("HighCPU", vec![("service", "api")]);
        a.tenant_id = "acme".into();
        let k = group_key(&d, &a);
        assert!(k.contains("tenant=acme"));
        assert!(k.contains("service=api"));
    }

    #[test]
    fn test_receiver_dedup() {
        let root = Route::root("default")
            .with_child(
                Route::child("a", vec![Matcher::equal("env", "prod")], vec!["x".into()])
                    .with_continue(true),
            )
            .with_child(Route::child(
                "b",
                vec![Matcher::equal("env", "prod")],
                vec!["x".into()],
            ));
        let a = alert("X", vec![("env", "prod")]);
        let d = route_alert_tree(&root, &a);
        assert_eq!(d.receivers, vec!["x".to_string()]);
    }
}
