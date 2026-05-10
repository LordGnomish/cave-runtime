//! WebAuthn-gated, RBAC-checked request context.
//!
//! Mirrors the `permission-react`/`permission-backend` plugin pair in
//! upstream Backstage v1.50.3. Every admin handler receives a [`RequestCtx`]
//! describing:
//!
//! * the **principal** (the user identity, derived from the auth cookie)
//! * the **tenant** the principal is operating *as*
//! * whether the principal completed a recent **WebAuthn** ceremony — admin
//!   views require `has_webauthn = true`
//! * the **permission set** granted to the principal in this tenant
//!
//! The same gate runs for every route, so a single `RequestCtx::authorise`
//! call is the only thing handlers need to do before reading state.

use crate::admin::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One concrete permission. Names follow Backstage's
/// `<plugin>.<resource>.<verb>` convention. The list here is exhaustive for
/// this batch — adding a new admin view should add the matching variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Permission {
    /// Read-only access to a tenant dashboard.
    DashboardRead,

    EtcdRead,
    EtcdWatch,

    CriRead,
    CriExec,

    ApiserverRead,

    IamRead,
    IamWrite,

    MeshRead,
    MeshWrite,

    PgRead,
    PgQuery,

    /// Read-only metadata + audit log (NOT secret values).
    VaultRead,

    /// Read worker contributions (night-pump batch outcomes per worker_id).
    ContributionsRead,

    /// Read KEDA ScaledObjects + scaler triggers + scale events.
    KedaRead,
    /// Pause / resume / delete KEDA ScaledObjects.
    KedaWrite,

    // ── 2026-05-10 batch: K8s core + data + autoscaler ────────────────
    SchedulerRead,
    SchedulerWrite,
    ControllerManagerRead,
    KubeletRead,
    KubeletExec,
    CloudControllerRead,
    KamajiRead,
    KamajiWrite,
    NetRead,
    NetWrite,
    RdbmsRead,
    RdbmsQuery,
    DocdbRead,
    DocdbQuery,
    CacheRead,
    CacheWrite,

    // ── 2026-05-10 batch B: rdbms-operator + lakehouse + streams ──────
    RdbmsOperatorRead,
    RdbmsOperatorFailover,
    RdbmsOperatorBackup,
    LakehouseRead,
    LakehouseSnapshot,
    StreamsRead,
    StreamsAdmin,
}

impl Permission {
    pub const fn name(self) -> &'static str {
        match self {
            Permission::DashboardRead => "portal.dashboard.read",
            Permission::EtcdRead => "etcd.kv.read",
            Permission::EtcdWatch => "etcd.kv.watch",
            Permission::CriRead => "cri.sandbox.read",
            Permission::CriExec => "cri.container.exec",
            Permission::ApiserverRead => "apiserver.resource.read",
            Permission::IamRead => "auth.user.read",
            Permission::IamWrite => "auth.role.write",
            Permission::MeshRead => "mesh.policy.read",
            Permission::MeshWrite => "mesh.policy.write",
            Permission::PgRead => "pg.table.read",
            Permission::PgQuery => "pg.query.exec",
            Permission::VaultRead => "vault.metadata.read",
            Permission::ContributionsRead => "cluster.contributions.read",
            Permission::KedaRead => "keda.scaledobject.read",
            Permission::KedaWrite => "keda.scaledobject.write",
            Permission::SchedulerRead => "scheduler.node.read",
            Permission::SchedulerWrite => "scheduler.policy.write",
            Permission::ControllerManagerRead => "controller-manager.lease.read",
            Permission::KubeletRead => "kubelet.pod.read",
            Permission::KubeletExec => "kubelet.pod.exec",
            Permission::CloudControllerRead => "cloud-controller.volume.read",
            Permission::KamajiRead => "kamaji.tcp.read",
            Permission::KamajiWrite => "kamaji.tcp.write",
            Permission::NetRead => "net.endpoint.read",
            Permission::NetWrite => "net.policy.write",
            Permission::RdbmsRead => "rdbms.cluster.read",
            Permission::RdbmsQuery => "rdbms.query.exec",
            Permission::DocdbRead => "docdb.collection.read",
            Permission::DocdbQuery => "docdb.query.exec",
            Permission::CacheRead => "cache.key.read",
            Permission::CacheWrite => "cache.key.write",
            Permission::RdbmsOperatorRead => "rdbms-operator.cluster.read",
            Permission::RdbmsOperatorFailover => "rdbms-operator.cluster.failover",
            Permission::RdbmsOperatorBackup => "rdbms-operator.cluster.backup",
            Permission::LakehouseRead => "lakehouse.table.read",
            Permission::LakehouseSnapshot => "lakehouse.snapshot.write",
            Permission::StreamsRead => "streams.topic.read",
            Permission::StreamsAdmin => "streams.topic.admin",
        }
    }
}

/// Reasons a request can be refused.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("WebAuthn ceremony required for admin views")]
    WebAuthnRequired,
    #[error("principal {principal} cannot operate as tenant {tenant}")]
    TenantMismatch { principal: String, tenant: TenantId },
    #[error("missing permission {missing}")]
    MissingPermission { missing: &'static str },
}

/// Request context carried by every admin handler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCtx {
    /// SPIFFE-style principal of the caller.
    pub principal: String,
    /// Tenant the caller is acting as. Must appear in `tenant_grants`.
    pub tenant: TenantId,
    /// Whether the caller has a fresh WebAuthn assertion in this session.
    pub has_webauthn: bool,
    /// Tenants this principal is allowed to act as.
    pub tenant_grants: BTreeSet<String>,
    /// Permissions granted within `tenant`.
    pub permissions: BTreeSet<Permission>,
}

impl RequestCtx {
    /// One-stop authorisation: WebAuthn presence, tenant grant, then permission.
    pub fn authorise(&self, required: Permission) -> Result<(), AuthError> {
        if !self.has_webauthn {
            return Err(AuthError::WebAuthnRequired);
        }
        if !self.tenant_grants.contains(self.tenant.as_str()) {
            return Err(AuthError::TenantMismatch {
                principal: self.principal.clone(),
                tenant: self.tenant.clone(),
            });
        }
        if !self.permissions.contains(&required) {
            return Err(AuthError::MissingPermission { missing: required.name() });
        }
        Ok(())
    }

    /// Convenience: build a "developer" context for tests.
    pub fn developer(tenant: &str, perms: &[Permission]) -> Self {
        let mut grants = BTreeSet::new();
        grants.insert(tenant.to_string());
        Self {
            principal: format!("spiffe://cluster.local/ns/{tenant}/sa/dev"),
            tenant: TenantId::new(tenant).expect("test fixture"),
            has_webauthn: true,
            tenant_grants: grants,
            permissions: perms.iter().copied().collect(),
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/permission-react/src/index.ts", "PermissionApi");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    #[test]
    fn missing_webauthn_blocks_admin_view() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/index.ts",
            "createPermissionIntegrationRouter",
            "tenant-perm-webauthn"
        );
        let mut ctx = RequestCtx::developer("tenant-perm-webauthn", &[Permission::DashboardRead]);
        ctx.has_webauthn = false;
        let err = ctx.authorise(Permission::DashboardRead).unwrap_err();
        assert_eq!(err, AuthError::WebAuthnRequired);
    }

    #[test]
    fn cross_tenant_request_is_refused() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionPolicy.ts",
            "tenantOwnership",
            "tenant-perm-cross"
        );
        let mut ctx = RequestCtx::developer("acme", &[Permission::EtcdRead]);
        // Caller asks to operate as `evil` but only holds the `acme` grant.
        ctx.tenant = TenantId::new("evil").expect("test fixture");
        let err = ctx.authorise(Permission::EtcdRead).unwrap_err();
        assert!(matches!(err, AuthError::TenantMismatch { .. }));
    }

    #[test]
    fn missing_permission_blocks_with_named_error() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/index.ts",
            "PermissionPolicy.handle",
            "tenant-perm-missing"
        );
        let ctx = RequestCtx::developer("tenant-perm-missing", &[Permission::EtcdRead]);
        let err = ctx.authorise(Permission::EtcdWatch).unwrap_err();
        assert_eq!(err, AuthError::MissingPermission { missing: "etcd.kv.watch" });
    }

    #[test]
    fn fully_authorised_request_passes() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "tenant-perm-ok"
        );
        let ctx = RequestCtx::developer(
            "tenant-perm-ok",
            &[Permission::EtcdRead, Permission::EtcdWatch, Permission::CriRead],
        );
        for p in [Permission::EtcdRead, Permission::EtcdWatch, Permission::CriRead] {
            assert!(ctx.authorise(p).is_ok());
        }
    }
}
