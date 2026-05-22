// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alert grouping + dedup, plus Notification log throttling.
//!
//! Implements Alertmanager's group_wait / group_interval / repeat_interval:
//!
//! - `group_wait`: hold the *first* notification for a brand-new group long
//!   enough that related alerts that arrive shortly after are bundled.
//! - `group_interval`: minimum gap between successive notifications when the
//!   group's *contents* change (new alerts join, others resolve).
//! - `repeat_interval`: minimum gap between re-notifications when the group
//!   is unchanged (still firing the same set).

use crate::models::Alert;
use crate::routing::RoutingDecision;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Deduplicate alerts by fingerprint, keeping the most recent starts_at.
pub fn deduplicate(alerts: Vec<Alert>) -> Vec<Alert> {
    let mut seen: HashMap<String, Alert> = HashMap::new();
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

/// Bucket alerts by (group_key, decision). Caller must apply the same
/// `RoutingDecision` to every alert that should land in the same bucket.
pub fn group_by_key<'a>(
    decision: &RoutingDecision,
    alerts: &'a [Alert],
) -> HashMap<String, Vec<&'a Alert>> {
    let mut out: HashMap<String, Vec<&'a Alert>> = HashMap::new();
    for a in alerts {
        let k = crate::routing::group_key(decision, a);
        out.entry(k).or_default().push(a);
    }
    out
}

/// Per-group state for throttle decisions.
#[derive(Debug, Clone, Default)]
pub struct GroupState {
    pub last_notified_at: Option<DateTime<Utc>>,
    pub first_seen_at: Option<DateTime<Utc>>,
    pub last_firing_set: Vec<String>, // sorted fingerprints
}

/// Decision returned by the throttle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyDecision {
    /// Send a notification immediately.
    Send,
    /// Hold off — a previous notification is too recent / group still warming up.
    Hold,
}

/// Decide whether to send a notification for the group, given the route's
/// timing settings and the per-group state at `now`.
pub fn should_notify(
    decision: &RoutingDecision,
    state: &GroupState,
    firing_fingerprints: &[String],
    now: DateTime<Utc>,
) -> NotifyDecision {
    let mut sorted_now: Vec<String> = firing_fingerprints.to_vec();
    sorted_now.sort();

    // First-ever group sighting must wait `group_wait` past first_seen_at.
    if state.last_notified_at.is_none() {
        let first = state.first_seen_at.unwrap_or(now);
        let waited = now - first;
        if waited >= decision.group_wait {
            return NotifyDecision::Send;
        }
        return NotifyDecision::Hold;
    }

    let last = state.last_notified_at.unwrap();
    let elapsed = now - last;

    let changed = sorted_now != state.last_firing_set;

    if changed {
        if elapsed >= decision.group_interval {
            NotifyDecision::Send
        } else {
            NotifyDecision::Hold
        }
    } else {
        if elapsed >= decision.repeat_interval {
            NotifyDecision::Send
        } else {
            NotifyDecision::Hold
        }
    }
}

/// Update the group state to reflect a notification just sent.
pub fn record_notification(
    state: &mut GroupState,
    firing_fingerprints: &[String],
    now: DateTime<Utc>,
) {
    state.last_notified_at = Some(now);
    let mut sorted: Vec<String> = firing_fingerprints.to_vec();
    sorted.sort();
    state.last_firing_set = sorted;
    if state.first_seen_at.is_none() {
        state.first_seen_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Alert, AlertSeverity, AlertState, Matcher};
    use crate::routing::RoutingDecision;
    use chrono::Duration;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn alert(name: &str, fp: &str) -> Alert {
        let mut labels = HashMap::new();
        labels.insert("alertname".to_string(), name.into());
        Alert {
            id: Uuid::new_v4(),
            name: name.into(),
            labels,
            annotations: HashMap::new(),
            severity: AlertSeverity::Warning,
            state: AlertState::Firing,
            starts_at: Utc::now(),
            ends_at: None,
            fingerprint: fp.into(),
            tenant_id: "anonymous".into(),
            generator_url: None,
        }
    }

    fn decision() -> RoutingDecision {
        RoutingDecision {
            receivers: vec!["slack".into()],
            group_by: vec!["alertname".into()],
            group_wait: Duration::seconds(30),
            group_interval: Duration::minutes(5),
            repeat_interval: Duration::hours(4),
        }
    }

    #[test]
    fn test_deduplicate_removes_dupes() {
        let a1 = alert("X", "fp1");
        let mut a2 = alert("X", "fp1");
        a2.starts_at = Utc::now() + Duration::minutes(1);
        let id_kept = a2.id;
        let result = deduplicate(vec![a1, a2]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, id_kept);
    }

    #[test]
    fn test_deduplicate_distinct_fingerprints_kept() {
        let result = deduplicate(vec![alert("X", "fp1"), alert("Y", "fp2")]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_group_by_key_buckets_same_alertname() {
        let d = decision();
        let alerts = vec![
            alert("HighCPU", "fp1"),
            alert("HighCPU", "fp2"),
            alert("HighMem", "fp3"),
        ];
        let groups = group_by_key(&d, &alerts);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_first_notification_held_until_group_wait() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.first_seen_at = Some(now);
        let res = should_notify(&d, &state, &vec!["fp1".into()], now + Duration::seconds(10));
        assert_eq!(res, NotifyDecision::Hold);
    }

    #[test]
    fn test_first_notification_sent_after_group_wait() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.first_seen_at = Some(now);
        let res = should_notify(&d, &state, &vec!["fp1".into()], now + Duration::seconds(31));
        assert_eq!(res, NotifyDecision::Send);
    }

    #[test]
    fn test_change_within_group_interval_is_held() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.last_notified_at = Some(now);
        state.first_seen_at = Some(now - Duration::hours(1));
        state.last_firing_set = vec!["fp1".into()];
        // Now firing fp1 + fp2 (changed) at now+30s — < 5min
        let res = should_notify(
            &d,
            &state,
            &vec!["fp1".into(), "fp2".into()],
            now + Duration::seconds(30),
        );
        assert_eq!(res, NotifyDecision::Hold);
    }

    #[test]
    fn test_change_after_group_interval_sends() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.last_notified_at = Some(now);
        state.first_seen_at = Some(now - Duration::hours(1));
        state.last_firing_set = vec!["fp1".into()];
        let res = should_notify(
            &d,
            &state,
            &vec!["fp1".into(), "fp2".into()],
            now + Duration::minutes(6),
        );
        assert_eq!(res, NotifyDecision::Send);
    }

    #[test]
    fn test_unchanged_within_repeat_interval_is_held() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.last_notified_at = Some(now);
        state.first_seen_at = Some(now - Duration::hours(1));
        state.last_firing_set = vec!["fp1".into()];
        let res = should_notify(&d, &state, &vec!["fp1".into()], now + Duration::hours(1));
        assert_eq!(res, NotifyDecision::Hold);
    }

    #[test]
    fn test_unchanged_after_repeat_interval_sends() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.last_notified_at = Some(now);
        state.first_seen_at = Some(now - Duration::hours(5));
        state.last_firing_set = vec!["fp1".into()];
        let res = should_notify(&d, &state, &vec!["fp1".into()], now + Duration::hours(5));
        assert_eq!(res, NotifyDecision::Send);
    }

    #[test]
    fn test_record_notification_sets_state() {
        let mut state = GroupState::default();
        let now = Utc::now();
        record_notification(&mut state, &vec!["fp1".into(), "fp2".into()], now);
        assert_eq!(state.last_notified_at, Some(now));
        assert_eq!(state.first_seen_at, Some(now));
        assert_eq!(
            state.last_firing_set,
            vec!["fp1".to_string(), "fp2".to_string()]
        );
    }

    #[test]
    fn test_record_notification_sorts_fingerprints() {
        let mut state = GroupState::default();
        let now = Utc::now();
        record_notification(&mut state, &vec!["fp2".into(), "fp1".into()], now);
        // Stored sorted so equality checks work order-independently.
        assert_eq!(
            state.last_firing_set,
            vec!["fp1".to_string(), "fp2".to_string()]
        );
    }

    #[test]
    fn test_change_detection_order_independent() {
        let d = decision();
        let now = Utc::now();
        let mut state = GroupState::default();
        state.last_notified_at = Some(now);
        state.first_seen_at = Some(now - Duration::hours(1));
        state.last_firing_set = vec!["fp1".into(), "fp2".into()];
        // Same fingerprints in reverse order → unchanged
        let res = should_notify(
            &d,
            &state,
            &vec!["fp2".into(), "fp1".into()],
            now + Duration::hours(5),
        );
        assert_eq!(res, NotifyDecision::Send); // unchanged + repeat_interval elapsed
    }

    #[test]
    fn test_use_routing_decision_via_real_route() {
        // Smoke: matchers + routing + group key wires together.
        use crate::models::Route;
        use crate::routing::route_alert_tree;
        let r = Route::root("default");
        let a = alert("HighCPU", "fp1");
        let d = route_alert_tree(&r, &a);
        let key = crate::routing::group_key(&d, &a);
        assert!(key.contains("HighCPU"));
        let _ = Matcher::equal("a", "b"); // matcher ctor still callable
    }
}
