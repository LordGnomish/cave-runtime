//! Controller reconcile loops.
//!
//! Each `reconcile_<kind>` is a pure-ish function: given the current state,
//! it computes what should change and applies it. Tests exercise the
//! invariants from the upstream Test* counterparts in
//! pkg/controller/<kind>/.

use crate::models::*;
use crate::state::ControllerManagerState;
use chrono::Utc;
use uuid::Uuid;

// ── DeploymentController ─────────────────────────────────────────────────────
// Mirrors pkg/controller/deployment/deployment_controller.go::syncDeployment.

/// Reconcile a Deployment: ensures a current ReplicaSet exists and that
/// `replicas` matches the spec; updates Deployment.status.observed_replicas.
pub fn reconcile_deployment(state: &ControllerManagerState, namespace: &str, name: &str) -> Option<()> {
    let mut dep = state.deployments.get_mut(&(namespace.to_string(), name.to_string()))?;
    // Find or create owning replicaset.
    let owned: Vec<Uuid> = state.replicasets.iter()
        .filter(|r| r.owner_deployment.as_deref() == Some(name) && r.namespace == namespace)
        .map(|r| r.uid)
        .collect();

    if owned.is_empty() {
        let rs = ReplicaSet {
            uid: Uuid::new_v4(),
            name: format!("{}-rs", name),
            namespace: namespace.to_string(),
            owner_deployment: Some(name.to_string()),
            replicas: dep.replicas,
            selector: dep.selector.clone(),
            template: dep.template.clone(),
            status: ReplicaSetStatus::default(),
            created_at: Utc::now(),
        };
        state.replicasets.insert(rs.uid, rs);
    } else {
        for uid in &owned {
            if let Some(mut rs) = state.replicasets.get_mut(uid) {
                rs.replicas = dep.replicas;
            }
        }
    }
    dep.status.observed_replicas = dep.replicas;
    Some(())
}

// ── ReplicaSetController ─────────────────────────────────────────────────────
// Mirrors pkg/controller/replicaset/replica_set.go::syncReplicaSet.

/// Reconcile a ReplicaSet: drives status.ready_replicas toward spec.replicas.
/// In a real cluster this would create/delete Pod objects; in cave-controller-
/// manager we just mark the count as ready (cave-kubelet/cave-cri own actual
/// pod execution).
pub fn reconcile_replicaset(state: &ControllerManagerState, uid: &Uuid) -> Option<()> {
    let mut rs = state.replicasets.get_mut(uid)?;
    rs.status.ready_replicas = rs.replicas;
    rs.status.available_replicas = rs.replicas;
    Some(())
}

// ── JobController ────────────────────────────────────────────────────────────
// Mirrors pkg/controller/job/job_controller.go::syncJob.

/// Reconcile a Job: mark it complete when succeeded == completions.
pub fn reconcile_job(state: &ControllerManagerState, namespace: &str, name: &str) -> Option<()> {
    let mut job = state.jobs.get_mut(&(namespace.to_string(), name.to_string()))?;
    if job.status.succeeded >= job.completions && job.status.completion_time.is_none() {
        job.status.completion_time = Some(Utc::now());
    }
    Some(())
}

// ── NamespaceController ──────────────────────────────────────────────────────
// Mirrors pkg/controller/namespace/namespace_controller.go::syncNamespace.

/// Reconcile a Namespace in Terminating phase: when no resources remain,
/// remove the namespace from state.
pub fn reconcile_namespace(state: &ControllerManagerState, name: &str) -> Option<()> {
    let phase = {
        let ns = state.namespaces.get(name)?;
        ns.phase.clone()
    };
    if !matches!(phase, NamespacePhase::Terminating) {
        return Some(());
    }
    // Are there any owned resources left in this namespace?
    let has_deps = state.deployments.iter().any(|d| d.namespace == name);
    let has_jobs = state.jobs.iter().any(|j| j.namespace == name);
    let has_rs = state.replicasets.iter().any(|r| r.namespace == name);
    let has_eps = state.endpoint_slices.iter().any(|e| e.namespace == name);
    if !has_deps && !has_jobs && !has_rs && !has_eps {
        state.namespaces.remove(name);
    }
    Some(())
}

// ── EndpointController ───────────────────────────────────────────────────────
// Mirrors pkg/controller/endpoint/endpoints_controller.go::syncService.

/// Reconcile EndpointSlice for a service: keeps the addresses set fresh.
pub fn reconcile_endpoint_slice(
    state: &ControllerManagerState,
    namespace: &str,
    name: &str,
    addresses: Vec<String>,
) -> Option<()> {
    let key = (namespace.to_string(), name.to_string());
    let svc_name = state
        .endpoint_slices
        .get(&key)
        .map(|e| e.service_name.clone())
        .unwrap_or_else(|| name.to_string());
    let slice = EndpointSlice {
        name: name.to_string(),
        namespace: namespace.to_string(),
        service_name: svc_name,
        ready: !addresses.is_empty(),
        addresses,
    };
    state.endpoint_slices.insert(key, slice);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn template() -> PodTemplate {
        let mut labels = HashMap::new();
        labels.insert("app".into(), "web".into());
        PodTemplate {
            labels,
            containers: vec![ContainerTemplate { name: "c".into(), image: "nginx".into() }],
        }
    }

    fn make_deployment(state: &ControllerManagerState, name: &str, ns: &str, replicas: u32) {
        state.deployments.insert(
            (ns.to_string(), name.to_string()),
            Deployment {
                name: name.into(),
                namespace: ns.into(),
                replicas,
                selector: HashMap::new(),
                template: template(),
                status: DeploymentStatus::default(),
                created_at: Utc::now(),
            },
        );
    }

    // ── DeploymentController tests ──────────────────────────────────────────

    #[test]
    fn test_deployment_reconcile_creates_replicaset() {
        // Mirrors TestDeploymentSyncCreatesReplicaSet.
        let s = ControllerManagerState::new();
        make_deployment(&s, "web", "default", 3);
        reconcile_deployment(&s, "default", "web").unwrap();
        let owned: Vec<_> = s.replicasets.iter()
            .filter(|r| r.owner_deployment.as_deref() == Some("web"))
            .map(|r| r.value().clone()).collect();
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].replicas, 3);
    }

    #[test]
    fn test_deployment_reconcile_updates_observed_replicas() {
        // Mirrors TestDeploymentObservedReplicasMatch.
        let s = ControllerManagerState::new();
        make_deployment(&s, "web", "default", 5);
        reconcile_deployment(&s, "default", "web").unwrap();
        let dep = s.deployments.get(&("default".into(), "web".into())).unwrap();
        assert_eq!(dep.status.observed_replicas, 5);
    }

    #[test]
    fn test_deployment_reconcile_scales_existing_replicaset() {
        // Mirrors TestDeploymentScalesReplicaSet: a second reconcile after
        // the spec changes should resize the existing RS, not create a new one.
        let s = ControllerManagerState::new();
        make_deployment(&s, "web", "default", 2);
        reconcile_deployment(&s, "default", "web").unwrap();
        // Bump the spec.
        s.deployments.get_mut(&("default".into(), "web".into())).unwrap().replicas = 7;
        reconcile_deployment(&s, "default", "web").unwrap();
        let count = s.replicasets.iter()
            .filter(|r| r.owner_deployment.as_deref() == Some("web")).count();
        assert_eq!(count, 1, "should reuse existing RS");
        let rs = s.replicasets.iter()
            .find(|r| r.owner_deployment.as_deref() == Some("web")).unwrap();
        assert_eq!(rs.replicas, 7);
    }

    // ── ReplicaSetController tests ──────────────────────────────────────────

    #[test]
    fn test_replicaset_reconcile_drives_ready_to_replicas() {
        // Mirrors TestReplicaSetReady.
        let s = ControllerManagerState::new();
        let uid = Uuid::new_v4();
        s.replicasets.insert(uid, ReplicaSet {
            uid, name: "rs1".into(), namespace: "default".into(),
            owner_deployment: None, replicas: 4, selector: HashMap::new(),
            template: template(), status: ReplicaSetStatus::default(),
            created_at: Utc::now(),
        });
        reconcile_replicaset(&s, &uid).unwrap();
        let rs = s.replicasets.get(&uid).unwrap();
        assert_eq!(rs.status.ready_replicas, 4);
        assert_eq!(rs.status.available_replicas, 4);
    }

    #[test]
    fn test_replicaset_reconcile_unknown_uid_is_none() {
        let s = ControllerManagerState::new();
        assert!(reconcile_replicaset(&s, &Uuid::new_v4()).is_none());
    }

    // ── JobController tests ─────────────────────────────────────────────────

    #[test]
    fn test_job_reconcile_marks_complete_when_succeeded_meets_completions() {
        // Mirrors TestJobControllerCompletes.
        let s = ControllerManagerState::new();
        let mut job_status = JobStatus::default();
        job_status.succeeded = 3;
        s.jobs.insert(("default".into(), "j".into()), Job {
            name: "j".into(), namespace: "default".into(),
            completions: 3, parallelism: 1, template: template(),
            status: job_status, created_at: Utc::now(),
        });
        reconcile_job(&s, "default", "j").unwrap();
        let j = s.jobs.get(&("default".into(), "j".into())).unwrap();
        assert!(j.status.completion_time.is_some());
    }

    #[test]
    fn test_job_reconcile_does_not_mark_incomplete_jobs() {
        let s = ControllerManagerState::new();
        let mut job_status = JobStatus::default();
        job_status.succeeded = 1;
        s.jobs.insert(("default".into(), "j".into()), Job {
            name: "j".into(), namespace: "default".into(),
            completions: 5, parallelism: 1, template: template(),
            status: job_status, created_at: Utc::now(),
        });
        reconcile_job(&s, "default", "j").unwrap();
        let j = s.jobs.get(&("default".into(), "j".into())).unwrap();
        assert!(j.status.completion_time.is_none());
    }

    // ── NamespaceController tests ───────────────────────────────────────────

    #[test]
    fn test_namespace_reconcile_terminating_empty_is_removed() {
        // Mirrors TestNamespaceFinalize.
        let s = ControllerManagerState::new();
        s.namespaces.insert("doomed".into(), Namespace {
            name: "doomed".into(), phase: NamespacePhase::Terminating,
            created_at: Utc::now(), deletion_timestamp: Some(Utc::now()),
        });
        reconcile_namespace(&s, "doomed").unwrap();
        assert!(!s.namespaces.contains_key("doomed"));
    }

    #[test]
    fn test_namespace_reconcile_terminating_with_resources_is_kept() {
        let s = ControllerManagerState::new();
        s.namespaces.insert("busy".into(), Namespace {
            name: "busy".into(), phase: NamespacePhase::Terminating,
            created_at: Utc::now(), deletion_timestamp: Some(Utc::now()),
        });
        make_deployment(&s, "web", "busy", 1);
        reconcile_namespace(&s, "busy").unwrap();
        assert!(s.namespaces.contains_key("busy"),
            "namespace must remain while it still owns resources");
    }

    #[test]
    fn test_namespace_reconcile_active_is_noop() {
        let s = ControllerManagerState::new();
        s.namespaces.insert("ok".into(), Namespace {
            name: "ok".into(), phase: NamespacePhase::Active,
            created_at: Utc::now(), deletion_timestamp: None,
        });
        reconcile_namespace(&s, "ok").unwrap();
        assert!(s.namespaces.contains_key("ok"));
    }

    // ── EndpointController tests ────────────────────────────────────────────

    #[test]
    fn test_endpoint_slice_reconcile_populates_addresses() {
        // Mirrors TestEndpointSliceCreate.
        let s = ControllerManagerState::new();
        reconcile_endpoint_slice(&s, "default", "web", vec!["10.0.0.1".into(), "10.0.0.2".into()]).unwrap();
        let slice = s.endpoint_slices.get(&("default".into(), "web".into())).unwrap();
        assert_eq!(slice.addresses.len(), 2);
        assert!(slice.ready);
    }

    #[test]
    fn test_endpoint_slice_reconcile_empty_is_not_ready() {
        let s = ControllerManagerState::new();
        reconcile_endpoint_slice(&s, "default", "web", vec![]).unwrap();
        let slice = s.endpoint_slices.get(&("default".into(), "web".into())).unwrap();
        assert!(!slice.ready);
    }
}
