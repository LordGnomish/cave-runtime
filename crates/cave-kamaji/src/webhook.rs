// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Validating admission webhook for TenantControlPlane.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/webhook/{create,update}.go
//!
//! Kamaji uses a validating webhook to enforce invariants the CRD schema
//! can't express: data-store kind must be a known value, replicas must
//! be > 0, certain fields are immutable once set. The Cave port exposes
//! `validate_create` and `validate_update` so the cave-admission layer
//! can wire them into its admission chain.

use thiserror::Error;

use crate::models::TenantControlPlane;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("field {field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("replicas must be >= 1 (got {replicas})")]
    InvalidReplicas { replicas: u32 },
    #[error("data_store kind {kind:?} is not recognised — must be one of shared-etcd / postgres / mysql")]
    UnknownDataStore { kind: String },
    #[error("field {field} is immutable once set (old={old:?}, new={new:?})")]
    ImmutableField {
        field: &'static str,
        old: String,
        new: String,
    },
}

/// Validate a TenantControlPlane on CREATE. Returns a list of errors
/// folded into a single result: the first violation surfaces.
pub fn validate_create(tcp: &TenantControlPlane) -> Result<(), WebhookError> {
    if tcp.name.trim().is_empty() {
        return Err(WebhookError::EmptyField { field: "name" });
    }
    if tcp.namespace.trim().is_empty() {
        return Err(WebhookError::EmptyField { field: "namespace" });
    }
    if tcp.spec.replicas < 1 {
        return Err(WebhookError::InvalidReplicas {
            replicas: tcp.spec.replicas,
        });
    }
    if !is_known_data_store(&tcp.spec.data_store) {
        return Err(WebhookError::UnknownDataStore {
            kind: tcp.spec.data_store.clone(),
        });
    }
    Ok(())
}

/// Validate a TenantControlPlane on UPDATE. Same field checks as create,
/// plus an immutability check on `kubernetes_version` and `namespace`.
pub fn validate_update(
    old: &TenantControlPlane,
    new: &TenantControlPlane,
) -> Result<(), WebhookError> {
    validate_create(new)?;
    if old.spec.kubernetes_version != new.spec.kubernetes_version {
        return Err(WebhookError::ImmutableField {
            field: "spec.kubernetes_version",
            old: old.spec.kubernetes_version.clone(),
            new: new.spec.kubernetes_version.clone(),
        });
    }
    if old.namespace != new.namespace {
        return Err(WebhookError::ImmutableField {
            field: "namespace",
            old: old.namespace.clone(),
            new: new.namespace.clone(),
        });
    }
    Ok(())
}

fn is_known_data_store(s: &str) -> bool {
    matches!(
        s,
        "shared-etcd" | "postgres" | "mysql" | "etcd" | "postgresql"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TenantPhase, TenantSpec, TenantStatus};
    use chrono::Utc;
    use uuid::Uuid;

    fn ok_tcp() -> TenantControlPlane {
        let now = Utc::now();
        TenantControlPlane {
            id: Uuid::new_v4(),
            name: "x".into(),
            namespace: "y".into(),
            spec: TenantSpec {
                kubernetes_version: "v1.31.0".into(),
                data_store: "shared-etcd".into(),
                replicas: 2,
            },
            status: TenantStatus {
                phase: TenantPhase::Provisioning,
                api_server_endpoint: None,
                ready: false,
                message: None,
            },
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn create_accepts_well_formed_tcp() {
        validate_create(&ok_tcp()).unwrap();
    }

    #[test]
    fn update_rejects_namespace_change() {
        let old = ok_tcp();
        let mut new = old.clone();
        new.namespace = "different".into();
        assert!(matches!(
            validate_update(&old, &new).unwrap_err(),
            WebhookError::ImmutableField { field: "namespace", .. }
        ));
    }
}
