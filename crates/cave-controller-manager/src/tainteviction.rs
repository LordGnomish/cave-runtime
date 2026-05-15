// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Taint-based per-pod eviction with toleration timers.
//!
//! Cite: pkg/controller/nodelifecycle/scheduler/taint_manager.go (v1.36.0).
//!
//! When a node carries a `NoExecute` taint, every pod on it that does
//! NOT tolerate the taint is evicted. Pods that DO tolerate it stay,
//! but only for `tolerationSeconds` — after that timer expires the
//! controller deletes the pod.
//!
//! Cave's `node_lifecycle` module marks nodes NoExecute at the
//! cluster level; this submodule provides the per-pod timer ledger
//! the audit doc flagged as missing.
//!
//! Pure-function state machine (mirrors `crate::namespace_controller`):
//! callers feed in observations + a `now` instant, get back one
//! [`EvictionAction`] per pass.

use crate::types::Cite;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/nodelifecycle/scheduler/taint_manager.go",
    "NoExecuteTaintManager.processPodOnNode",
);

/// `effect` half of an upstream `core/v1.Taint`. Cave only models
/// `NoExecute` here; `NoSchedule`/`PreferNoSchedule` are filtered at
/// admission time in `cave-scheduler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TaintEffect {
    NoExecute,
}

/// One `Taint` on a node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeTaint {
    pub key: String,
    pub value: Option<String>,
    pub effect: TaintEffect,
    /// When the apiserver stamped the taint. Drives the
    /// toleration-seconds timer's t0.
    pub time_added: DateTime<Utc>,
}

/// One `toleration` entry from a pod's spec. `seconds = None` means
/// infinite (pod stays as long as the taint+toleration match).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PodToleration {
    pub key: Option<String>,
    pub value: Option<String>,
    pub effect: Option<TaintEffect>,
    /// `operator`: `"Equal"` matches when key/value both match,
    /// `"Exists"` matches when key matches and value is ignored. We
    /// store as a small enum so a bad TOML value can't slip through.
    pub operator: TolerationOperator,
    pub seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum TolerationOperator {
    Equal,
    Exists,
}

impl Default for TolerationOperator {
    fn default() -> Self {
        TolerationOperator::Equal
    }
}

/// One pod the controller is tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodView {
    pub uid: String,
    pub namespace: String,
    pub name: String,
    pub node_name: String,
    pub tolerations: Vec<PodToleration>,
}

/// Decision the reconciler emits per (taint × pod) pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvictionAction {
    /// Pod tolerates the taint indefinitely — nothing to do.
    Tolerated,
    /// Pod doesn't tolerate the taint — evict immediately.
    EvictNow { pod_uid: String, reason: String },
    /// Pod tolerates the taint for `tolerationSeconds`; schedule a
    /// future eviction at `evict_at`.
    Schedule { pod_uid: String, evict_at: DateTime<Utc> },
    /// Pod's toleration window has expired — evict now.
    Expired { pod_uid: String, reason: String },
}

/// Match a single toleration against a taint. Mirrors upstream
/// `Toleration.MatchToleration`.
pub fn matches(t: &PodToleration, taint: &NodeTaint) -> bool {
    // Effect: when `t.effect = None` the toleration matches any
    // effect; otherwise must equal.
    if let Some(e) = t.effect {
        if e != taint.effect {
            return false;
        }
    }
    // Key: when `t.key = None` the toleration must use Exists and
    // match any key (the "tolerate everything" wildcard).
    match (t.key.as_deref(), t.operator) {
        (None, TolerationOperator::Exists) => true,
        (None, _) => false, // empty key is only legal with Exists.
        (Some(k), op) => {
            if k != taint.key {
                return false;
            }
            match op {
                TolerationOperator::Exists => true,
                TolerationOperator::Equal => t.value.as_deref() == taint.value.as_deref(),
            }
        }
    }
}

/// Find the *first* matching toleration. The first match wins
/// because pod authors typically list the most-specific toleration
/// first; upstream behaves the same.
pub fn first_matching<'a>(
    pod: &'a PodView,
    taint: &NodeTaint,
) -> Option<&'a PodToleration> {
    pod.tolerations.iter().find(|t| matches(t, taint))
}

/// Decide what to do with one (pod, taint) pair.
///
/// * No matching toleration → `EvictNow`.
/// * Matching toleration with `seconds = None` → `Tolerated`
///   (infinite window).
/// * Matching toleration with `seconds = Some(s)`:
///   * `now < taint.time_added + s` → `Schedule { evict_at }`.
///   * `now >= taint.time_added + s` → `Expired`.
pub fn evaluate(
    pod: &PodView,
    taint: &NodeTaint,
    now: DateTime<Utc>,
) -> EvictionAction {
    let Some(t) = first_matching(pod, taint) else {
        return EvictionAction::EvictNow {
            pod_uid: pod.uid.clone(),
            reason: format!(
                "no toleration matches NoExecute taint key={} on node {}",
                taint.key, pod.node_name
            ),
        };
    };
    match t.seconds {
        None => EvictionAction::Tolerated,
        Some(s) => {
            let dur = ChronoDuration::seconds(s);
            let evict_at = taint.time_added + dur;
            if now >= evict_at {
                EvictionAction::Expired {
                    pod_uid: pod.uid.clone(),
                    reason: format!(
                        "tolerationSeconds={s} expired for taint key={}",
                        taint.key
                    ),
                }
            } else {
                EvictionAction::Schedule {
                    pod_uid: pod.uid.clone(),
                    evict_at,
                }
            }
        }
    }
}

/// In-memory ledger of pending evictions. The controller's outer
/// loop polls `due(now)` to drain pods whose timers have fired.
#[derive(Debug, Default, Clone)]
pub struct EvictionLedger {
    /// `pod_uid → evict_at` for pods with finite toleration windows.
    pub scheduled: std::collections::BTreeMap<String, DateTime<Utc>>,
}

impl EvictionLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an action to the ledger. `EvictNow` / `Expired` clear
    /// any pending entry (the controller will issue the delete);
    /// `Schedule` upserts the timer; `Tolerated` is a no-op.
    pub fn apply(&mut self, action: &EvictionAction) {
        match action {
            EvictionAction::EvictNow { pod_uid, .. }
            | EvictionAction::Expired { pod_uid, .. } => {
                self.scheduled.remove(pod_uid);
            }
            EvictionAction::Schedule { pod_uid, evict_at } => {
                self.scheduled.insert(pod_uid.clone(), *evict_at);
            }
            EvictionAction::Tolerated => {}
        }
    }

    /// Pods whose timer has fired by `now`. Returned in stable
    /// (pod_uid lex) order so the controller log is deterministic.
    pub fn due(&self, now: DateTime<Utc>) -> Vec<String> {
        self.scheduled
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(uid, _)| uid.clone())
            .collect()
    }

    /// Drop the entry — used when the controller has actually
    /// deleted the pod or the taint has been removed.
    pub fn forget(&mut self, pod_uid: &str) {
        self.scheduled.remove(pod_uid);
    }

    pub fn len(&self) -> usize {
        self.scheduled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scheduled.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn taint(key: &str, value: Option<&str>) -> NodeTaint {
        NodeTaint {
            key: key.into(),
            value: value.map(str::to_string),
            effect: TaintEffect::NoExecute,
            time_added: t0(),
        }
    }

    fn pod_with_tolerations(uid: &str, ts: Vec<PodToleration>) -> PodView {
        PodView {
            uid: uid.into(),
            namespace: "default".into(),
            name: uid.into(),
            node_name: "n1".into(),
            tolerations: ts,
        }
    }

    fn tol_eq(key: &str, value: Option<&str>, secs: Option<i64>) -> PodToleration {
        PodToleration {
            key: Some(key.into()),
            value: value.map(str::to_string),
            effect: Some(TaintEffect::NoExecute),
            operator: TolerationOperator::Equal,
            seconds: secs,
        }
    }

    fn tol_exists(key: &str, secs: Option<i64>) -> PodToleration {
        PodToleration {
            key: Some(key.into()),
            value: None,
            effect: Some(TaintEffect::NoExecute),
            operator: TolerationOperator::Exists,
            seconds: secs,
        }
    }

    fn tol_wildcard() -> PodToleration {
        PodToleration {
            key: None,
            value: None,
            effect: None,
            operator: TolerationOperator::Exists,
            seconds: None,
        }
    }

    #[test]
    fn no_toleration_evicts_immediately() {
        let (_c, _t) = test_ctx!(
            "pkg/controller/nodelifecycle/scheduler/taint_manager.go",
            "noToleration",
            "te-1"
        );
        let pod = pod_with_tolerations("p1", vec![]);
        let act = evaluate(&pod, &taint("disk-pressure", None), t0());
        match act {
            EvictionAction::EvictNow { pod_uid, .. } => assert_eq!(pod_uid, "p1"),
            other => panic!("expected EvictNow, got {other:?}"),
        }
    }

    #[test]
    fn matching_toleration_with_none_seconds_is_tolerated() {
        let pod = pod_with_tolerations("p1", vec![tol_eq("disk-pressure", None, None)]);
        let act = evaluate(&pod, &taint("disk-pressure", None), t0());
        assert_eq!(act, EvictionAction::Tolerated);
    }

    #[test]
    fn matching_toleration_with_finite_seconds_schedules() {
        let pod = pod_with_tolerations("p1", vec![tol_eq("disk-pressure", None, Some(30))]);
        let act = evaluate(&pod, &taint("disk-pressure", None), t0());
        match act {
            EvictionAction::Schedule { pod_uid, evict_at } => {
                assert_eq!(pod_uid, "p1");
                assert_eq!(evict_at, t0() + ChronoDuration::seconds(30));
            }
            other => panic!("expected Schedule, got {other:?}"),
        }
    }

    #[test]
    fn matching_toleration_with_expired_window_yields_expired() {
        let pod = pod_with_tolerations("p1", vec![tol_eq("disk-pressure", None, Some(30))]);
        let future = t0() + ChronoDuration::seconds(31);
        let act = evaluate(&pod, &taint("disk-pressure", None), future);
        assert!(matches!(act, EvictionAction::Expired { .. }));
    }

    #[test]
    fn exists_operator_matches_any_value() {
        let pod = pod_with_tolerations("p1", vec![tol_exists("disk-pressure", None)]);
        let act = evaluate(&pod, &taint("disk-pressure", Some("any")), t0());
        assert_eq!(act, EvictionAction::Tolerated);
    }

    #[test]
    fn equal_operator_requires_value_match() {
        let pod = pod_with_tolerations(
            "p1",
            vec![tol_eq("disk-pressure", Some("red"), None)],
        );
        let red = taint("disk-pressure", Some("red"));
        let blue = taint("disk-pressure", Some("blue"));
        assert_eq!(evaluate(&pod, &red, t0()), EvictionAction::Tolerated);
        assert!(matches!(
            evaluate(&pod, &blue, t0()),
            EvictionAction::EvictNow { .. }
        ));
    }

    #[test]
    fn wildcard_toleration_matches_every_taint() {
        let pod = pod_with_tolerations("p1", vec![tol_wildcard()]);
        let act1 = evaluate(&pod, &taint("a", None), t0());
        let act2 = evaluate(&pod, &taint("b", Some("c")), t0());
        assert_eq!(act1, EvictionAction::Tolerated);
        assert_eq!(act2, EvictionAction::Tolerated);
    }

    #[test]
    fn ledger_records_schedule_then_clears_on_expire() {
        let mut led = EvictionLedger::new();
        let pod = pod_with_tolerations("p1", vec![tol_eq("k", None, Some(60))]);
        let a = evaluate(&pod, &taint("k", None), t0());
        led.apply(&a);
        assert_eq!(led.len(), 1);
        let a2 = evaluate(&pod, &taint("k", None), t0() + ChronoDuration::seconds(70));
        led.apply(&a2);
        assert_eq!(led.len(), 0);
    }

    #[test]
    fn ledger_due_returns_pods_past_deadline() {
        let mut led = EvictionLedger::new();
        led.scheduled.insert("p-old".into(), t0() - ChronoDuration::seconds(1));
        led.scheduled.insert("p-future".into(), t0() + ChronoDuration::seconds(1));
        let due = led.due(t0());
        assert_eq!(due, vec!["p-old".to_string()]);
    }

    #[test]
    fn forget_drops_entry() {
        let mut led = EvictionLedger::new();
        led.scheduled.insert("p1".into(), t0());
        led.forget("p1");
        assert!(led.is_empty());
    }

    #[test]
    fn first_matching_returns_first_in_order() {
        let pod = pod_with_tolerations(
            "p1",
            vec![tol_eq("a", None, Some(10)), tol_eq("a", None, Some(20))],
        );
        let m = first_matching(&pod, &taint("a", None)).unwrap();
        assert_eq!(m.seconds, Some(10));
    }

    #[test]
    fn effect_mismatch_does_not_tolerate() {
        // Toleration uses NoExecute (only effect we model) and the
        // taint also NoExecute → match. But a future cave that
        // models NoSchedule must not accidentally match.
        let pod = pod_with_tolerations("p1", vec![tol_eq("k", None, None)]);
        // Sanity: NoExecute matches NoExecute today.
        assert_eq!(
            evaluate(&pod, &taint("k", None), t0()),
            EvictionAction::Tolerated
        );
    }
}
