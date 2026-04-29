//! Node-lifecycle controller — marks nodes NotReady when their heartbeat is
//! older than `monitor_grace_period`, and ready when fresh.
//!
//! Upstream: pkg/controller/nodelifecycle/node_lifecycle_controller.go::monitorNodeHealth

use crate::models::NodeObs;
use chrono::{DateTime, Duration, Utc};

/// Default node-monitor grace period mirrors upstream
/// (`--node-monitor-grace-period=40s`).
pub const DEFAULT_GRACE_PERIOD_SECS: i64 = 40;

/// Reconcile a single node — return the node observation with `ready` set
/// according to whether `last_heartbeat` is within `grace_period` of `now`.
pub fn reconcile_node_lifecycle(
    mut node: NodeObs,
    now: DateTime<Utc>,
    grace_period_secs: i64,
) -> NodeObs {
    let stale = now.signed_duration_since(node.last_heartbeat)
        > Duration::seconds(grace_period_secs);
    node.ready = !stale;
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, hb: DateTime<Utc>) -> NodeObs {
        NodeObs { name: name.into(), last_heartbeat: hb, ready: true }
    }

    /// Port of `TestNodeLifecycleMarksNotReady` — a node whose heartbeat is
    /// older than the grace period is marked NotReady.
    #[test]
    fn test_node_lifecycle_marks_not_ready() {
        let now = Utc::now();
        let stale = node("n", now - Duration::seconds(120));
        let result = reconcile_node_lifecycle(stale, now, DEFAULT_GRACE_PERIOD_SECS);
        assert!(!result.ready);
    }

    #[test]
    fn test_node_lifecycle_keeps_ready_when_fresh() {
        let now = Utc::now();
        let fresh = node("n", now - Duration::seconds(5));
        let result = reconcile_node_lifecycle(fresh, now, DEFAULT_GRACE_PERIOD_SECS);
        assert!(result.ready);
    }

    #[test]
    fn test_node_lifecycle_boundary_at_grace_period() {
        let now = Utc::now();
        let exactly = node("n", now - Duration::seconds(DEFAULT_GRACE_PERIOD_SECS));
        let result = reconcile_node_lifecycle(exactly, now, DEFAULT_GRACE_PERIOD_SECS);
        // Equal-to-grace-period is still ready (strict greater-than).
        assert!(result.ready);
    }
}
