//! ReplicaSet controller — drives `current` toward `desired` by computing
//! a Pod scale delta.
//!
//! Upstream: pkg/controller/replicaset/replica_set.go::syncReplicaSet

use crate::models::ReplicaSetObs;

/// Scale delta direction: positive = create N pods; negative = delete N pods;
/// zero = stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleAction {
    Create(u32),
    Delete(u32),
    NoOp,
}

/// Reconcile a ReplicaSet — compute how many pods to create or delete so that
/// `current` matches `desired`.
pub fn reconcile_replicaset(rs: &ReplicaSetObs) -> ScaleAction {
    if rs.current == rs.desired {
        ScaleAction::NoOp
    } else if rs.current < rs.desired {
        ScaleAction::Create(rs.desired - rs.current)
    } else {
        ScaleAction::Delete(rs.current - rs.desired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn rs(desired: u32, current: u32) -> ReplicaSetObs {
        ReplicaSetObs {
            uid: Uuid::new_v4(),
            name: "rs".into(),
            namespace: "default".into(),
            desired,
            current,
            owner_uid: None,
        }
    }

    /// Port of `TestReplicaSetControllerScaleUp` — when `current < desired`,
    /// produce a Create delta of the difference.
    #[test]
    fn test_replicaset_controller_scale_up() {
        assert_eq!(reconcile_replicaset(&rs(5, 2)), ScaleAction::Create(3));
        assert_eq!(reconcile_replicaset(&rs(3, 0)), ScaleAction::Create(3));
    }

    /// Port of `TestReplicaSetControllerScaleDown` — when `current > desired`,
    /// produce a Delete delta of the difference.
    #[test]
    fn test_replicaset_controller_scale_down() {
        assert_eq!(reconcile_replicaset(&rs(2, 5)), ScaleAction::Delete(3));
        assert_eq!(reconcile_replicaset(&rs(0, 4)), ScaleAction::Delete(4));
    }

    #[test]
    fn test_replicaset_noop_when_stable() {
        assert_eq!(reconcile_replicaset(&rs(3, 3)), ScaleAction::NoOp);
        assert_eq!(reconcile_replicaset(&rs(0, 0)), ScaleAction::NoOp);
    }
}
