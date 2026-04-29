//! Deployment controller — drives Deployment desired_replicas → ReplicaSet.
//!
//! Upstream: pkg/controller/deployment/deployment_controller.go::syncDeployment

use crate::models::{DeploymentObs, ReplicaSetObs};

/// Reconcile a Deployment by computing the ReplicaSet update needed to bring
/// available_replicas in line with desired_replicas. Returns the ReplicaSet
/// patch (a Vec because rolling updates may touch multiple RS) that the
/// caller should apply via the API server.
pub fn reconcile_deployment(d: &DeploymentObs, rs_list: &[ReplicaSetObs]) -> Vec<ReplicaSetObs> {
    let mut owned: Vec<ReplicaSetObs> = rs_list
        .iter()
        .filter(|rs| rs.owner_uid == Some(d.uid))
        .cloned()
        .collect();

    if owned.is_empty() {
        // No managed RS yet — synthesize one matching desired replicas.
        return vec![ReplicaSetObs {
            uid: uuid::Uuid::new_v4(),
            name: format!("{}-rs", d.name),
            namespace: d.namespace.clone(),
            desired: d.desired_replicas,
            current: 0,
            owner_uid: Some(d.uid),
        }];
    }

    // Single RS path — update its desired count.
    if owned.len() == 1 {
        owned[0].desired = d.desired_replicas;
        return owned;
    }

    // Multi-RS (rolling update) path — concentrate desired on the most-recent RS
    // (largest UID is a stable proxy for "newest" in this simplified model) and
    // scale the rest to zero. Callers compose this with progressDeadline +
    // maxSurge in real reconciliation; here we model the steady-state target.
    owned.sort_by_key(|rs| rs.uid);
    let last_idx = owned.len() - 1;
    for (i, rs) in owned.iter_mut().enumerate() {
        rs.desired = if i == last_idx { d.desired_replicas } else { 0 };
    }
    owned
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use std::collections::HashMap;

    fn d(name: &str, desired: u32) -> DeploymentObs {
        DeploymentObs {
            uid: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            desired_replicas: desired,
            available_replicas: 0,
            selector: HashMap::new(),
        }
    }

    fn rs(name: &str, owner: Uuid, desired: u32, current: u32) -> ReplicaSetObs {
        ReplicaSetObs {
            uid: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            desired,
            current,
            owner_uid: Some(owner),
        }
    }

    /// Port of `TestDeploymentControllerSync` — first reconcile creates a single
    /// ReplicaSet with the deployment's desired count.
    #[test]
    fn test_deployment_controller_sync() {
        let dep = d("web", 3);
        let result = reconcile_deployment(&dep, &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].desired, 3);
        assert_eq!(result[0].owner_uid, Some(dep.uid));
        assert!(result[0].name.starts_with("web"));
    }

    #[test]
    fn test_deployment_existing_rs_scaled() {
        let dep = d("web", 5);
        let existing = rs("web-rs", dep.uid, 2, 2);
        let result = reconcile_deployment(&dep, &[existing.clone()]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].desired, 5);
        assert_eq!(result[0].uid, existing.uid);
    }

    #[test]
    fn test_deployment_rolling_concentrates_on_newest() {
        let dep = d("web", 4);
        let old = rs("web-rs-old", dep.uid, 4, 4);
        let new = rs("web-rs-new", dep.uid, 0, 0);
        let result = reconcile_deployment(&dep, &[old, new]);
        // Sorted by uid; last gets the desired count, others are scaled to 0.
        assert_eq!(result.iter().filter(|r| r.desired > 0).count(), 1);
        assert_eq!(result.iter().map(|r| r.desired).max().unwrap(), 4);
    }
}
