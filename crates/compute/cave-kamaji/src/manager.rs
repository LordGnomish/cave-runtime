// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-tenant control-plane orchestrator — the loop that holds every
//! TenantControlPlane the Kamaji controller manages, drives each to its
//! desired state against the shared datastore, and enforces the tenant
//! isolation boundary between them.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   controllers/tenantcontrolplane_controller.go — the per-tenant Reconcile
//!     loop the controller-runtime Manager fans out across every TCP
//!   internal/utilities/utilities.go               — the labels enforcing
//!     cross-tenant isolation (KamajiLabels / ownership boundary)
//!   api/v1alpha1/datastore_types.go               — DataStore.status.usedBy
//!
//! Kamaji runs one shared datastore back-end behind many tenant control
//! planes; each tenant is carved its own isolated schema/user (SQL) or key
//! prefix (etcd) so two tenants can never read each other's API objects, and
//! every resource a tenant owns is stamped with its control-plane name so it
//! can never be selected or mutated by another. This module is the Cave-side
//! orchestration hold that materialises that model: it owns one [`Reconciler`]
//! per tenant, registers each tenant in the shared DataStore `usedBy`
//! registry, and refuses cross-tenant resource access.

use crate::components::{Container, ControlPlaneInput};
use crate::connection::{Connection, DatastoreError};
use crate::ds_setup::SetupResource;
use crate::isolation::{
    CONTROL_PLANE_LABEL_KEY, deregister_usage, owns_resource, register_usage, used_by_key,
};
use crate::models::TenantControlPlane;
use crate::reconcile::{ReconcileContext, Reconciler};
use crate::status::{Condition, ConditionStatus, ConditionType};
use std::collections::BTreeMap;
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

/// A tenant the manager owns: its reconciler plus the desired-state inputs
/// (control-plane plan + datastore setup secret) it converges towards.
struct TenantEntry {
    reconciler: Reconciler,
    control_plane: ControlPlaneInput,
    setup: SetupResource,
    used_by_key: String,
}

/// Refusal to cross the tenant isolation boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IsolationError {
    #[error("tenant {requested_by} may not access a resource owned by {owned_by:?}")]
    CrossTenant {
        requested_by: String,
        owned_by: Option<String>,
    },
}

/// The multi-tenant orchestrator.
#[derive(Default)]
pub struct TenantManager {
    tenants: HashMap<Uuid, TenantEntry>,
    used_by: Vec<String>,
}

impl TenantManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of tenants currently managed.
    pub fn tenant_count(&self) -> usize {
        self.tenants.len()
    }

    /// The shared DataStore `usedBy` registry — the namespaced keys of every
    /// tenant currently bound to the back-end (sorted).
    pub fn used_by(&self) -> &[String] {
        &self.used_by
    }

    /// Provision (or re-converge) a tenant against the shared datastore: store
    /// its desired state, drive the reconcile pipeline to steady state, and
    /// record it in the shared `usedBy` registry. Idempotent.
    pub fn provision(
        &mut self,
        tcp: &mut TenantControlPlane,
        conn: &mut dyn Connection,
        control_plane: ControlPlaneInput,
        setup: SetupResource,
        api_server_endpoint: impl Into<String>,
    ) -> Result<(), DatastoreError> {
        let key = used_by_key(&tcp.namespace, &tcp.name);
        let entry = self.tenants.entry(tcp.id).or_insert_with(|| TenantEntry {
            reconciler: Reconciler::new(),
            control_plane: control_plane.clone(),
            setup: setup.clone(),
            used_by_key: key.clone(),
        });
        // Keep the desired state current on re-provision.
        entry.control_plane = control_plane;
        entry.setup = setup;

        // Drive the reconcile pipeline to steady state against the shared
        // back-end. Bounded by the pipeline length so a mis-converging
        // reconciler can never spin forever.
        let mut ctx = ReconcileContext {
            connection: conn,
            setup: entry.setup.clone(),
            api_server_endpoint: api_server_endpoint.into(),
            control_plane: entry.control_plane.clone(),
        };
        let max_passes = crate::reconcile::default_pipeline().len() + 1;
        for _ in 0..max_passes {
            if entry.reconciler.reconcile(tcp, &mut ctx)?.steady_state {
                break;
            }
        }

        register_usage(&mut self.used_by, &key);
        info!(
            tenant = %tcp.name,
            namespace = %tcp.namespace,
            ready = self.is_ready(tcp.id),
            tenants = self.tenants.len(),
            "provisioned tenant on shared datastore"
        );
        Ok(())
    }

    /// Decommission a tenant: tear its isolated datastore down, forget it, and
    /// release its claim on the shared back-end — without touching any other
    /// tenant.
    pub fn delete(
        &mut self,
        tcp: &mut TenantControlPlane,
        conn: &mut dyn Connection,
    ) -> Result<(), DatastoreError> {
        if let Some(mut entry) = self.tenants.remove(&tcp.id) {
            let mut ctx = ReconcileContext {
                connection: conn,
                setup: entry.setup.clone(),
                api_server_endpoint: String::new(),
                control_plane: entry.control_plane.clone(),
            };
            entry.reconciler.reconcile_delete(tcp, &mut ctx)?;
            deregister_usage(&mut self.used_by, &entry.used_by_key);
            info!(
                tenant = %tcp.name,
                namespace = %tcp.namespace,
                remaining = self.tenants.len(),
                "decommissioned tenant; released shared-datastore claim"
            );
        }
        Ok(())
    }

    /// Enforce the tenant isolation boundary: a resource may only be touched by
    /// the control plane whose name its `kamaji.clastix.io/name` label carries.
    pub fn authorize(
        &self,
        requesting_tenant: &str,
        labels: &BTreeMap<String, String>,
    ) -> Result<(), IsolationError> {
        if owns_resource(labels, requesting_tenant) {
            return Ok(());
        }
        Err(IsolationError::CrossTenant {
            requested_by: requesting_tenant.to_string(),
            owned_by: labels.get(CONTROL_PLANE_LABEL_KEY).cloned(),
        })
    }

    /// The dedicated control-plane containers materialised for a tenant.
    pub fn control_plane(&self, id: Uuid) -> Option<&[Container]> {
        self.tenants
            .get(&id)
            .and_then(|e| e.reconciler.control_plane())
    }

    /// The reported status conditions for a tenant.
    pub fn conditions(&self, id: Uuid) -> Option<&[Condition]> {
        self.tenants.get(&id).map(|e| e.reconciler.conditions())
    }

    /// Whether a tenant has reached steady state (aggregate `Ready` True).
    pub fn is_ready(&self, id: Uuid) -> bool {
        self.tenants
            .get(&id)
            .and_then(|e| e.reconciler.condition_status(ConditionType::Ready))
            == Some(ConditionStatus::True)
    }
}
