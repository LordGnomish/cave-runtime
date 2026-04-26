//! Node lease + status reporter — heartbeat (`coordination.k8s.io/v1`
//! Lease) and node-condition transition tracking.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `pkg/kubelet/nodelease/controller.go` (`Controller.sync`,
//!     `retryUpdateNodeLease`).
//!   `pkg/kubelet/kubelet_node_status.go`
//!     (`tryUpdateNodeStatus`, `recordNodeStatusEvent`).
//!
//! Behavior:
//!
//!   * The kubelet renews a per-node Lease object every
//!     `node_lease_duration / 4` seconds (default 10s of 40s lease).
//!   * If the kubelet fails to renew within the lease duration, the node
//!     controller treats the node as `Unknown`.
//!   * Node conditions transition with timestamps; a transition records
//!     `last_transition_time` and `last_heartbeat_time` per upstream
//!     `NodeCondition`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LeaseError {
    #[error("lease holder mismatch: expected '{expected}', got '{got}'")]
    HolderMismatch { expected: String, got: String },
    #[error("lease expired: now > renew_time + duration")]
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLease {
    pub holder: String,
    pub tenant_id: String,
    /// Lease duration in seconds.
    pub duration_secs: u32,
    pub renew_time: DateTime<Utc>,
    pub acquire_time: DateTime<Utc>,
}

impl NodeLease {
    pub fn new(holder: &str, tenant_id: &str, duration_secs: u32, now: DateTime<Utc>) -> Self {
        Self {
            holder: holder.into(),
            tenant_id: tenant_id.into(),
            duration_secs,
            renew_time: now,
            acquire_time: now,
        }
    }
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.renew_time + Duration::seconds(self.duration_secs as i64)
    }
    pub fn is_valid(&self, now: DateTime<Utc>) -> bool {
        now <= self.expires_at()
    }
    pub fn renew(&mut self, holder: &str, now: DateTime<Utc>) -> Result<(), LeaseError> {
        if holder != self.holder {
            return Err(LeaseError::HolderMismatch {
                expected: self.holder.clone(),
                got: holder.into(),
            });
        }
        if !self.is_valid(now) {
            return Err(LeaseError::Expired);
        }
        self.renew_time = now;
        Ok(())
    }
    pub fn renew_interval(&self) -> Duration {
        Duration::seconds((self.duration_secs / 4).max(1) as i64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConditionRecord {
    pub condition_type: String,
    pub status: ConditionStatus,
    pub reason: String,
    pub message: String,
    pub last_heartbeat_time: DateTime<Utc>,
    pub last_transition_time: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct NodeConditionTracker {
    pub conditions: BTreeMap<String, NodeConditionRecord>,
}

impl NodeConditionTracker {
    /// Set a condition; if status changed, `last_transition_time` is bumped.
    /// Always bumps `last_heartbeat_time`.
    pub fn set(
        &mut self,
        condition_type: &str,
        status: ConditionStatus,
        reason: &str,
        message: &str,
        now: DateTime<Utc>,
    ) -> bool {
        match self.conditions.get_mut(condition_type) {
            Some(existing) => {
                let transitioned = existing.status != status;
                existing.status = status;
                existing.reason = reason.into();
                existing.message = message.into();
                existing.last_heartbeat_time = now;
                if transitioned {
                    existing.last_transition_time = now;
                }
                transitioned
            }
            None => {
                self.conditions.insert(
                    condition_type.into(),
                    NodeConditionRecord {
                        condition_type: condition_type.into(),
                        status,
                        reason: reason.into(),
                        message: message.into(),
                        last_heartbeat_time: now,
                        last_transition_time: now,
                    },
                );
                true
            }
        }
    }

    pub fn ready(&self) -> bool {
        self.conditions
            .get("Ready")
            .map(|c| c.status == ConditionStatus::True)
            .unwrap_or(false)
    }

    pub fn lost_heartbeat_since(&self, threshold: DateTime<Utc>) -> Vec<String> {
        self.conditions
            .iter()
            .filter(|(_, c)| c.last_heartbeat_time < threshold)
            .map(|(t, _)| t.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> DateTime<Utc> {
        Utc::now()
    }

    #[test]
    fn lease_is_valid_until_expiry() {
        let now = t0();
        let l = NodeLease::new("node-1", "acme", 40, now);
        assert!(l.is_valid(now));
        assert!(l.is_valid(now + Duration::seconds(39)));
        assert!(l.is_valid(now + Duration::seconds(40)));
        assert!(!l.is_valid(now + Duration::seconds(41)));
    }

    #[test]
    fn lease_renew_advances_renew_time() {
        let now = t0();
        let mut l = NodeLease::new("node-1", "acme", 40, now);
        let later = now + Duration::seconds(10);
        l.renew("node-1", later).unwrap();
        assert_eq!(l.renew_time, later);
        assert_eq!(l.acquire_time, now);
    }

    #[test]
    fn lease_renew_holder_mismatch_errors() {
        let now = t0();
        let mut l = NodeLease::new("node-1", "acme", 40, now);
        assert!(matches!(
            l.renew("node-2", now + Duration::seconds(5)),
            Err(LeaseError::HolderMismatch { .. })
        ));
    }

    #[test]
    fn lease_renew_after_expiry_errors() {
        let now = t0();
        let mut l = NodeLease::new("node-1", "acme", 40, now);
        assert!(matches!(
            l.renew("node-1", now + Duration::seconds(60)),
            Err(LeaseError::Expired)
        ));
    }

    #[test]
    fn renew_interval_is_quarter_of_duration() {
        let now = t0();
        let l = NodeLease::new("n", "t", 40, now);
        assert_eq!(l.renew_interval(), Duration::seconds(10));
    }

    #[test]
    fn renew_interval_floor_one_second() {
        let now = t0();
        let l = NodeLease::new("n", "t", 1, now);
        assert_eq!(l.renew_interval(), Duration::seconds(1));
    }

    #[test]
    fn condition_first_set_records_transition() {
        let now = t0();
        let mut t = NodeConditionTracker::default();
        let transitioned = t.set("Ready", ConditionStatus::True, "KubeletReady", "kubelet is ready", now);
        assert!(transitioned);
        assert!(t.ready());
    }

    #[test]
    fn condition_same_status_does_not_transition() {
        let now = t0();
        let mut t = NodeConditionTracker::default();
        t.set("Ready", ConditionStatus::True, "KubeletReady", "ok", now);
        let later = now + Duration::seconds(30);
        let transitioned = t.set("Ready", ConditionStatus::True, "KubeletReady", "still ok", later);
        assert!(!transitioned);
        let rec = t.conditions.get("Ready").unwrap();
        assert_eq!(rec.last_heartbeat_time, later);
        assert_eq!(rec.last_transition_time, now);
    }

    #[test]
    fn condition_status_change_bumps_transition_time() {
        let now = t0();
        let mut t = NodeConditionTracker::default();
        t.set("Ready", ConditionStatus::True, "KubeletReady", "", now);
        let later = now + Duration::seconds(60);
        let transitioned = t.set("Ready", ConditionStatus::False, "KubeletNotReady", "ouch", later);
        assert!(transitioned);
        let rec = t.conditions.get("Ready").unwrap();
        assert_eq!(rec.last_transition_time, later);
        assert!(!t.ready());
    }

    #[test]
    fn lost_heartbeat_detects_stale_conditions() {
        let now = t0();
        let mut t = NodeConditionTracker::default();
        t.set("Ready", ConditionStatus::True, "", "", now);
        t.set("MemoryPressure", ConditionStatus::False, "", "", now - Duration::seconds(120));
        let lost = t.lost_heartbeat_since(now - Duration::seconds(60));
        assert_eq!(lost, vec!["MemoryPressure".to_string()]);
    }

    #[test]
    fn unknown_status_models_lease_loss() {
        let now = t0();
        let mut t = NodeConditionTracker::default();
        t.set("Ready", ConditionStatus::True, "", "", now);
        t.set(
            "Ready",
            ConditionStatus::Unknown,
            "NodeStatusUnknown",
            "lease lost",
            now + Duration::seconds(50),
        );
        assert!(!t.ready());
    }
}
