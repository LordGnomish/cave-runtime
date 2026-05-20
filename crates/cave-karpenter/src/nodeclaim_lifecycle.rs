// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodeClaim lifecycle controller — launch, drain, terminate.
//!
//! Upstream reference (Karpenter v1.4.0):
//!   pkg/controllers/nodeclaim/lifecycle/launch.go
//!   pkg/controllers/nodeclaim/lifecycle/termination.go
//!   pkg/controllers/nodeclaim/lifecycle/registration.go
//!
//! The Karpenter lifecycle controller is a Reconcile loop that drives a
//! NodeClaim through:
//!   nil → Provisioned → Registered → Initialized → Terminating → (deleted)
//!
//! The Cave port collapses the loop into three idempotent functions that
//! callers wire into their reconcile cadence: [`launch`], [`drain`],
//! [`terminate`]. Provider-side calls go through the
//! [`crate::provider::CloudProvider`] trait so unit tests can swap in
//! [`crate::provider::StaticProvider`].

use crate::models::{NodeClaim, NodeClaimStatus};
use crate::provider::{CloudProvider, ProviderError, ProviderResult};
use std::time::{Duration, SystemTime};

/// Outcome of [`launch`].
#[derive(Debug, Clone)]
pub enum LaunchOutcome {
    /// Provider returned a new `provider_id`; status filled in.
    Launched { provider_id: String },
    /// Claim was already launched — idempotent no-op.
    AlreadyLaunched,
}

/// Drive a NodeClaim from `nil` → `Provisioned` by asking the provider
/// to allocate an instance. Idempotent: returns [`LaunchOutcome::AlreadyLaunched`]
/// if `status.provider_id` is already populated.
pub fn launch<P: CloudProvider>(
    claim: &mut NodeClaim,
    provider: &P,
) -> ProviderResult<LaunchOutcome> {
    if let Some(status) = claim.status.as_ref()
        && status.provider_id.is_some()
    {
        return Ok(LaunchOutcome::AlreadyLaunched);
    }

    // Pick the first instance-type / zone hint from the NodeClaim spec.
    let instance_hint = claim
        .spec
        .requirements
        .iter()
        .find(|r| r.key == "node.kubernetes.io/instance-type")
        .and_then(|r| r.values.first().cloned())
        .unwrap_or_else(|| "default-instance".to_string());
    let zone_hint = claim
        .spec
        .requirements
        .iter()
        .find(|r| r.key == "topology.kubernetes.io/zone")
        .and_then(|r| r.values.first().cloned())
        .unwrap_or_else(|| "default".to_string());

    let provider_id = provider.create(&instance_hint, &zone_hint)?;
    let mut status = claim.status.clone().unwrap_or_default();
    status.provider_id = Some(provider_id.clone());
    status.node_name = Some(format!("{}-node", claim.name));
    claim.status = Some(status);
    claim.created_at = Some(SystemTime::now());
    Ok(LaunchOutcome::Launched { provider_id })
}

/// Mark a NodeClaim as drained.
///
/// This is the *idempotent flip* used by [`terminate`] when the caller
/// has no pod inventory — it simply records the drained bit. For the
/// full PDB-respecting eviction loop, build a
/// [`crate::drain::DrainPlan`] over the pods scheduled to this claim's
/// node and drive [`DrainPlan::step`] to completion before calling
/// [`drain_with_pods`].
pub fn drain(claim: &mut NodeClaim, _grace_period: Duration) -> Result<(), ProviderError> {
    claim.drained = true;
    Ok(())
}

/// Drain by driving an in-crate [`DrainPlan`] over the supplied pod set.
/// Returns the evicted pods on success. If any pod cannot be evicted
/// because a PodDisruptionBudget would be violated, `drained` is left
/// `false` and the caller is expected to retry.
pub fn drain_with_pods(
    claim: &mut NodeClaim,
    pods: Vec<crate::drain::PodDescriptor>,
    pdbs: Vec<crate::drain::PodDisruptionBudget>,
    now: std::time::SystemTime,
) -> Result<Vec<crate::drain::PodDescriptor>, ProviderError> {
    let mut plan = crate::drain::DrainPlan::new(pods, pdbs);
    let _ = plan.drive_to_completion(now, 4096);
    match plan.status() {
        crate::drain::DrainStatus::Complete => {
            claim.drained = true;
            Ok(plan.evicted_pods().to_vec())
        }
        crate::drain::DrainStatus::InProgress { .. } => Err(ProviderError::Unavailable(
            "drain blocked by PodDisruptionBudget — retry after PDB allows eviction"
                .to_string(),
        )),
    }
}

/// Terminate a NodeClaim — drain (if requested) then ask the provider to
/// delete the instance. Idempotent: a second call after `terminated=true`
/// is a no-op.
pub fn terminate<P: CloudProvider>(
    claim: &mut NodeClaim,
    provider: &P,
    drain_first: bool,
) -> ProviderResult<()> {
    if claim.terminated {
        return Ok(());
    }
    if drain_first && !claim.drained {
        drain(claim, Duration::from_secs(30))?;
    }
    let Some(status) = claim.status.as_ref() else {
        // Never launched — just mark terminated.
        claim.terminated = true;
        return Ok(());
    };
    let Some(provider_id) = status.provider_id.as_ref() else {
        claim.terminated = true;
        return Ok(());
    };
    provider.delete(provider_id)?;
    let mut status = claim.status.clone().unwrap_or_default();
    status.provider_id = None;
    claim.status = Some(status);
    claim.terminated = true;
    Ok(())
}

/// Ensure the NodeClaim's status block exists. Mirrors upstream
/// `lifecycle/registration.go::Registration` initial population.
pub fn ensure_status(claim: &mut NodeClaim) -> &mut NodeClaimStatus {
    claim.status.get_or_insert_with(NodeClaimStatus::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Requirement;
    use crate::provider::StaticProvider;

    #[test]
    fn launch_records_instance_type_hint_in_provider_id() {
        let mut c = NodeClaim::default();
        c.name = "n".into();
        c.spec.requirements.push(Requirement {
            key: "node.kubernetes.io/instance-type".into(),
            operator: crate::models::RequirementOperator::In,
            values: vec!["m5.large".into()],
            min_values: None,
        });
        let outcome = launch(&mut c, &StaticProvider::new()).unwrap();
        match outcome {
            LaunchOutcome::Launched { provider_id } => {
                assert!(provider_id.contains("m5.large"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn launch_is_idempotent() {
        let mut c = NodeClaim::default();
        c.name = "n".into();
        let provider = StaticProvider::new();
        launch(&mut c, &provider).unwrap();
        let again = launch(&mut c, &provider).unwrap();
        assert!(matches!(again, LaunchOutcome::AlreadyLaunched));
    }

    #[test]
    fn terminate_unlaunched_claim_is_safe() {
        let mut c = NodeClaim::default();
        c.name = "n".into();
        terminate(&mut c, &StaticProvider::new(), false).unwrap();
        assert!(c.terminated);
    }

    #[test]
    fn drain_with_pods_completes_when_no_pdb() {
        use crate::drain::{PodDescriptor, PodOwnerKind};
        let mut c = NodeClaim::default();
        c.name = "n".into();
        let pods = vec![PodDescriptor {
            namespace: "default".into(),
            name: "p".into(),
            labels: vec![],
            owner_kind: PodOwnerKind::Deployment,
            grace_deadline: None,
        }];
        let evicted = drain_with_pods(&mut c, pods, vec![], SystemTime::now()).unwrap();
        assert_eq!(evicted.len(), 1);
        assert!(c.drained);
    }

    #[test]
    fn drain_with_pods_errors_when_pdb_blocks() {
        use crate::drain::{PodDescriptor, PodDisruptionBudget, PodOwnerKind};
        let mut c = NodeClaim::default();
        c.name = "n".into();
        let pods = vec![PodDescriptor {
            namespace: "default".into(),
            name: "p".into(),
            labels: vec![("app".into(), "x".into())],
            owner_kind: PodOwnerKind::Deployment,
            grace_deadline: None,
        }];
        let pdb = PodDisruptionBudget {
            namespace: "default".into(),
            name: "blocker".into(),
            selector: vec![("app".into(), "x".into())],
            min_available: 1,
        };
        let err = drain_with_pods(&mut c, pods, vec![pdb], SystemTime::now()).unwrap_err();
        assert!(err.to_string().contains("PodDisruptionBudget"));
        assert!(!c.drained);
    }
}
