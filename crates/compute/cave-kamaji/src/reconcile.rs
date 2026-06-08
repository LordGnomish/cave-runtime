// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TenantControlPlane reconcile flow — the ordered pipeline that provisions a
//! dedicated control plane for each tenant and drives its lifecycle phase.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   controllers/resources.go
//!     getDefaultResources  — the verbatim resource ordering
//!     GetDeletableResources — datastore teardown on delete
//!   controllers/tenantcontrolplane_controller.go — the Reconcile loop
//!
//! Upstream's controller drives a `[]resources.Resource` in order on every
//! Reconcile, each resource being idempotently created/updated against the
//! management cluster; once every resource is converged the TenantControlPlane
//! is marked Ready. The Cave port keeps that exact ordering and convergence
//! model. The datastore-setup step is wired to the real [`crate::ds_setup`]
//! state machine so reconciling genuinely carves the tenant's isolated
//! schema/user/grant out of the shared back-end (see [`crate::connection`]).
//! The remaining steps (certificates, kubeconfigs, the Deployment envelope,
//! konnectivity, ingress) are convergence markers — their concrete Kubernetes
//! objects are owned by cave-certs / cave-controller-manager / cave-net.

use crate::components::{Container, ControlPlaneInput, build_control_plane};
use crate::connection::{Connection, DatastoreError, run_migrate};
use crate::ds_setup::{SetupResource, run_setup, run_teardown};
use crate::lifecycle;
use crate::models::TenantControlPlane;
use crate::status::{Condition, ConditionStatus, ConditionType, set_condition};
use std::collections::BTreeSet;
use tracing::info;

/// The reconcile pipeline, in the exact order of `getDefaultResources`. Each
/// entry mirrors an upstream `resources.Resource` (the names follow the
/// resource `GetName()` where defined upstream, e.g. `ds.multitenancy`,
/// `datastore-config`, `datastore-setup`).
pub fn default_pipeline() -> Vec<&'static str> {
    vec![
        // getDataStoreMigratingResources
        "datastore-migrate",
        // getUpgradeResources
        "upgrade",
        // getKubernetesServiceResources
        "service",
        // getKubeadmConfigResources
        "kubeadmconfig",
        // getKubernetesCertificatesResources (CA first, then leaves)
        "ca",
        "front-proxy-ca",
        "sa-certificate",
        "apiserver-certificate",
        "apiserver-kubelet-client-certificate",
        "front-proxy-client-certificate",
        // getKubeconfigResources
        "admin-kubeconfig",
        "super-admin-kubeconfig",
        "controller-manager-kubeconfig",
        "scheduler-kubeconfig",
        // getKubernetesStorageResources
        "ds.multitenancy",
        "datastore-config",
        "datastore-setup",
        "datastore-certificate",
        // getKonnectivityServerRequirementsResources
        "konnectivity-egress-selector-configuration",
        "konnectivity-certificate",
        "konnectivity-kubeconfig",
        // getKubernetesDeploymentResources
        "deployment",
        // getKonnectivityServerPatchResources
        "konnectivity-deployment",
        "konnectivity-service",
        // getDataStoreMigratingCleanup
        "datastore-migrate-cleanup",
        // getKubernetesIngressResources
        "ingress",
    ]
}

/// External inputs a reconcile pass needs: the datastore connection it drives,
/// the per-tenant setup resource, and the resolved api-server endpoint to
/// publish once the control plane is Running.
pub struct ReconcileContext<'a> {
    pub connection: &'a mut dyn Connection,
    pub setup: SetupResource,
    pub api_server_endpoint: String,
    /// The control-plane the `deployment` step materialises for this tenant —
    /// the apiserver / controller-manager / scheduler container plan.
    pub control_plane: ControlPlaneInput,
}

/// The result of a single reconcile pass.
#[derive(Debug, Clone)]
pub struct ReconcilePass {
    /// Resource step-names applied (mutated) during this pass.
    pub applied: Vec<String>,
    /// True when every pipeline step was already converged (steady state).
    pub steady_state: bool,
}

/// Drives a TenantControlPlane towards its desired state, tracking which
/// pipeline steps have converged — one [`Reconciler`] per tenant.
#[derive(Debug, Default)]
pub struct Reconciler {
    converged: BTreeSet<String>,
    control_plane: Option<Vec<Container>>,
    conditions: Vec<Condition>,
}

impl Reconciler {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many pipeline steps have reached convergence.
    pub fn converged_steps(&self) -> usize {
        self.converged.len()
    }

    /// The dedicated control-plane containers built once the `deployment` step
    /// converged (kube-apiserver / kube-controller-manager / kube-scheduler),
    /// or `None` while the tenant has not yet reached that step.
    pub fn control_plane(&self) -> Option<&[Container]> {
        self.control_plane.as_deref()
    }

    /// The Kubernetes-style status conditions, recomputed each reconcile pass
    /// from which pipeline stages have converged.
    pub fn conditions(&self) -> &[Condition] {
        &self.conditions
    }

    /// The status of a single condition type, if it has been reported yet.
    pub fn condition_status(&self, cond_type: ConditionType) -> Option<ConditionStatus> {
        self.conditions
            .iter()
            .find(|c| c.cond_type == cond_type)
            .map(|c| c.status)
    }

    /// Recompute the status conditions from convergence progress. Each
    /// sub-condition flips True once its owning pipeline stage has converged;
    /// `Ready` aggregates to True only at steady state.
    fn report_status(&mut self, steady_state: bool) {
        let datastore = self.converged.contains("datastore-setup");
        let control_plane = self.converged.contains("deployment");
        let konnectivity = self.converged.contains("konnectivity-service");
        let kubeconfig = self.converged.contains("scheduler-kubeconfig");
        let tri = |ok: bool| {
            if ok {
                ConditionStatus::True
            } else {
                ConditionStatus::False
            }
        };
        set_condition(
            &mut self.conditions,
            ConditionType::DataStoreHealthy,
            tri(datastore),
            if datastore { "Healthy" } else { "Provisioning" },
            "tenant datastore schema/user/grant materialised",
        );
        set_condition(
            &mut self.conditions,
            ConditionType::ControlPlaneHealthy,
            tri(control_plane),
            if control_plane { "Healthy" } else { "Provisioning" },
            "dedicated apiserver/controller-manager/scheduler materialised",
        );
        set_condition(
            &mut self.conditions,
            ConditionType::KubeconfigReady,
            tri(kubeconfig),
            "EndpointReachable",
            "admin/controller-manager/scheduler kubeconfigs generated",
        );
        set_condition(
            &mut self.conditions,
            ConditionType::KonnectivityHealthy,
            tri(konnectivity),
            if konnectivity { "Healthy" } else { "Provisioning" },
            "konnectivity tunnel established",
        );
        set_condition(
            &mut self.conditions,
            ConditionType::Ready,
            tri(steady_state),
            if steady_state { "Ready" } else { "NotReady" },
            "all reconcile stages converged",
        );
    }

    /// Run one reconcile pass: apply every not-yet-converged step in pipeline
    /// order, then update the lifecycle phase. The datastore-setup step runs
    /// the real setup state machine against the connection. When nothing was
    /// applied the control plane is at steady state and is marked Running.
    pub fn reconcile(
        &mut self,
        tcp: &mut TenantControlPlane,
        ctx: &mut ReconcileContext<'_>,
    ) -> Result<ReconcilePass, DatastoreError> {
        let mut applied = Vec::new();

        for step in default_pipeline() {
            if self.converged.contains(step) {
                continue;
            }
            if step == "datastore-migrate" {
                // Run the Kine schema migration for SQL/streaming back-ends.
                let migrated = run_migrate(ctx.connection)?;
                info!(tenant = %tcp.name, step, migrated, "reconciled datastore migration");
            } else if step == "datastore-setup" {
                // Genuinely materialise the tenant's isolated datastore.
                let op = run_setup(ctx.connection, &ctx.setup)?;
                info!(tenant = %tcp.name, step, ?op, "reconciled datastore setup");
            } else if step == "deployment" {
                // Materialise the tenant's dedicated control plane — the
                // apiserver / controller-manager / scheduler container plan,
                // wired to its bound datastore (etcd prefix vs Kine sidecar).
                let components = build_control_plane(&ctx.control_plane);
                info!(
                    tenant = %tcp.name,
                    step,
                    components = components.len(),
                    "materialised dedicated control plane"
                );
                self.control_plane = Some(components);
            } else {
                info!(tenant = %tcp.name, step, "reconciled resource");
            }
            self.converged.insert(step.to_string());
            applied.push(step.to_string());
        }

        let steady_state = applied.is_empty();
        if steady_state {
            lifecycle::mark_running(tcp, ctx.api_server_endpoint.clone());
        } else {
            lifecycle::provision(tcp);
        }
        self.report_status(steady_state);

        Ok(ReconcilePass {
            applied,
            steady_state,
        })
    }

    /// Run the deletable-resource flow (`GetDeletableResources`): tear the
    /// tenant's datastore down (Setup teardown), forget convergence, and move
    /// the control plane into the Deleting phase.
    pub fn reconcile_delete(
        &mut self,
        tcp: &mut TenantControlPlane,
        ctx: &mut ReconcileContext<'_>,
    ) -> Result<(), DatastoreError> {
        run_teardown(ctx.connection, &ctx.setup)?;
        self.converged.clear();
        self.control_plane = None;
        self.conditions.clear();
        lifecycle::deprovision(tcp);
        Ok(())
    }
}
