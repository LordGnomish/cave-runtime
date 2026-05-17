// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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

    // ── 2026-05-10 batch C: charter compliance dashboard ──────────────
    /// View the per-crate Charter compliance matrix.
    AdminComplianceView,
    /// Trigger a manual refresh of the compliance snapshot.
    AdminComplianceRefresh,

    // ── 2026-05-10 batch D: tier1 admin pages ─────────────────────────
    PolicyRead,
    PolicyWrite,
    ArtifactsRead,
    AlertsRead,
    AlertsAck,
    BackupRead,
    BackupTrigger,
    IncidentsRead,
    IncidentsWrite,
    VulnsRead,
    WorkflowsRead,
    ChaosRead,
    ChaosTrigger,
    SloRead,

    // ── 2026-05-11 batch E: tier1 admin pages (15 crates) ─────────────
    AiObsRead,
    ChatRead,
    CostRead,
    DastRead,
    DevlakeRead,
    ForensicsRead,
    GatewayRead,
    InfraRead,
    PamRead,
    SbomRead,
    ScanRead,
    SecretsBrowserRead,
    UptimeRead,
    ClusterRead,
    KubeProxyRead,

    // ── 2026-05-11 batch F: 10 more tier1 admin pages ──────────────────
    StoreRead,
    MetricsRead,
    TraceRead,
    AuthSessionsRead,
    DashboardRead2,
    DnsRead,
    LogsRead,
    SecurityRead,
    HaRead,
    ErpRead,

    // ── 2026-05-11 batch G: 10 more tier1 admin pages ──────────────────
    DeployRead,
    PipelinesRead,
    RolloutsRead,
    KnativeRead,
    LlmGatewayRead,
    LocalLlmRead,
    TrackerRead,
    UpstreamRead,
    ContainerScanRead,
    AdmissionRead,

    // ── 2026-05-11 batch H: 10 more tier1 admin pages ──────────────────
    CdcRead,
    CertsRead,
    CrmRead,
    CrossplaneRead,
    GitopsRead,
    KarpenterRead,
    KubevirtRead,
    LedgerRead,
    OncallRead,
    SearchRead,

    // ── 2026-05-12 batch: KEDA real reimplementation ───────────────────
    /// Read ScaledObject detail (CRD-shaped). Stricter than `KedaRead`
    /// which only exposes the summary list.
    KedaScaledObjectRead,
    /// Create / edit / delete ScaledObjects.
    KedaScaledObjectWrite,
    /// Read ScaledJobs.
    KedaScaledJobRead,
    /// Create / edit / delete ScaledJobs.
    KedaScaledJobWrite,
    /// Read TriggerAuthentication / ClusterTriggerAuthentication.
    KedaTriggerAuthRead,
    /// Mutate TriggerAuthentication.
    KedaTriggerAuthWrite,
    /// Browse the scaler catalog (read-only static content).
    KedaScalerCatalog,
    /// Read per-scaler Prometheus-backed metrics (events/min, sync errors,
    /// scaling latency p50/p99).
    KedaMetricsRead,

    // ── 2026-05-11 batch I: 5 upstream-UI parity admin pages ──────────
    /// Read Grafana panel-render data (dashboards by uid / folder).
    GrafanaRead,
    /// Read Prometheus targets / alerts / rules.
    PrometheusRead,
    /// Run LogQL queries against the Loki-equivalent backend.
    LokiRead,
    /// Read Kubernetes Dashboard resources (workloads, configmaps,
    /// services — backed by cave-apiserver / cave-kubelet / etc.).
    K8sDashboardRead,
    /// Read Istio Kiali topology + traffic data (backed by cave-mesh).
    KialiRead,

    // ── 2026-05-13 batch: realtime + power-user ───────────────────────
    /// Subscribe to the live SSE event bus (multiplexed cluster events).
    EventsSubscribe,
    /// Read entries from the audit log (Platform admin gate also applies).
    AuditRead,
    /// Append a synthetic entry to the audit log (test + CLI helpers).
    AuditWrite,
    /// Read tour progress + advance the onboarding wizard.
    OnboardRead,
    /// Mark a tour step complete on behalf of a persona.
    OnboardWrite,
    /// Read the global search index.
    GlobalSearchRead,
    /// Trigger a quick-action fix from the compliance dashboard.
    QuickActionTrigger,
    /// Read the live cluster snapshot (Raft term / leader / WAL apply).
    ClusterLiveRead,
    /// Submit a bulk command across multiple admin resources.
    BulkOpsSubmit,

    // ── 2026-05-13 P1 scratch pages ───────────────────────────────────
    IcebergRead,
    MlflowRead,
    LiteLlmRead,
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
            Permission::AdminComplianceView => "admin.compliance.view",
            Permission::AdminComplianceRefresh => "admin.compliance.refresh",
            Permission::PolicyRead => "policy.rule.read",
            Permission::PolicyWrite => "policy.rule.write",
            Permission::ArtifactsRead => "artifacts.record.read",
            Permission::AlertsRead => "alerts.rule.read",
            Permission::AlertsAck => "alerts.rule.ack",
            Permission::BackupRead => "backup.job.read",
            Permission::BackupTrigger => "backup.job.trigger",
            Permission::IncidentsRead => "incidents.record.read",
            Permission::IncidentsWrite => "incidents.record.write",
            Permission::VulnsRead => "vulns.record.read",
            Permission::WorkflowsRead => "workflows.run.read",
            Permission::ChaosRead => "chaos.experiment.read",
            Permission::ChaosTrigger => "chaos.experiment.trigger",
            Permission::SloRead => "slo.objective.read",
            Permission::AiObsRead => "ai-obs.metric.read",
            Permission::ChatRead => "chat.thread.read",
            Permission::CostRead => "cost.report.read",
            Permission::DastRead => "dast.scan.read",
            Permission::DevlakeRead => "devlake.metric.read",
            Permission::ForensicsRead => "forensics.evidence.read",
            Permission::GatewayRead => "gateway.route.read",
            Permission::InfraRead => "infra.stack.read",
            Permission::PamRead => "pam.session.read",
            Permission::SbomRead => "sbom.component.read",
            Permission::ScanRead => "scan.result.read",
            Permission::SecretsBrowserRead => "secrets.metadata.read",
            Permission::UptimeRead => "uptime.probe.read",
            Permission::ClusterRead => "cluster.kube.read",
            Permission::KubeProxyRead => "kube-proxy.service.read",
            Permission::StoreRead => "store.bucket.read",
            Permission::MetricsRead => "metrics.series.read",
            Permission::TraceRead => "trace.service.read",
            Permission::AuthSessionsRead => "auth.session.read",
            Permission::DashboardRead2 => "dashboard.catalog.read",
            Permission::DnsRead => "dns.zone.read",
            Permission::LogsRead => "logs.stream.read",
            Permission::SecurityRead => "security.event.read",
            Permission::HaRead => "ha.failover.read",
            Permission::ErpRead => "erp.invoice.read",
            Permission::DeployRead => "deploy.activity.read",
            Permission::PipelinesRead => "pipelines.run.read",
            Permission::RolloutsRead => "rollouts.canary.read",
            Permission::KnativeRead => "knative.service.read",
            Permission::LlmGatewayRead => "llm-gateway.route.read",
            Permission::LocalLlmRead => "local-llm.model.read",
            Permission::TrackerRead => "tracker.issue.read",
            Permission::UpstreamRead => "upstream.project.read",
            Permission::ContainerScanRead => "container-scan.image.read",
            Permission::AdmissionRead => "admission.decision.read",
            Permission::CdcRead => "cdc.pipeline.read",
            Permission::CertsRead => "certs.certificate.read",
            Permission::CrmRead => "crm.account.read",
            Permission::CrossplaneRead => "crossplane.composition.read",
            Permission::GitopsRead => "gitops.config.read",
            Permission::KarpenterRead => "karpenter.nodepool.read",
            Permission::KubevirtRead => "kubevirt.vm.read",
            Permission::LedgerRead => "ledger.entry.read",
            Permission::OncallRead => "oncall.shift.read",
            Permission::SearchRead => "search.index.read",
            Permission::KedaScaledObjectRead => "keda.scaledobject.detail.read",
            Permission::KedaScaledObjectWrite => "keda.scaledobject.detail.write",
            Permission::KedaScaledJobRead => "keda.scaledjob.read",
            Permission::KedaScaledJobWrite => "keda.scaledjob.write",
            Permission::KedaTriggerAuthRead => "keda.triggerauthentication.read",
            Permission::KedaTriggerAuthWrite => "keda.triggerauthentication.write",
            Permission::KedaScalerCatalog => "keda.scaler.catalog",
            Permission::KedaMetricsRead => "keda.metrics.read",
            Permission::GrafanaRead => "grafana.panel.read",
            Permission::PrometheusRead => "prometheus.target.read",
            Permission::LokiRead => "loki.logql.read",
            Permission::K8sDashboardRead => "k8s-dashboard.resource.read",
            Permission::KialiRead => "kiali.topology.read",
            Permission::EventsSubscribe => "portal.events.subscribe",
            Permission::AuditRead => "portal.audit.read",
            Permission::AuditWrite => "portal.audit.write",
            Permission::OnboardRead => "portal.onboard.read",
            Permission::OnboardWrite => "portal.onboard.write",
            Permission::GlobalSearchRead => "portal.search.read",
            Permission::QuickActionTrigger => "portal.quickaction.trigger",
            Permission::ClusterLiveRead => "portal.cluster.live.read",
            Permission::BulkOpsSubmit => "portal.bulkops.submit",
            Permission::IcebergRead => "iceberg.catalog.read",
            Permission::MlflowRead => "mlflow.experiment.read",
            Permission::LiteLlmRead => "litellm.proxy.read",
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
    #[error("persona {actual:?} cannot access {required:?} surfaces")]
    PersonaForbidden { actual: Persona, required: Persona },
}

#[cfg(test)]
mod persona_can_access_tests {
    use super::Persona;

    #[test]
    fn platform_admin_can_access_every_tier() {
        assert!(Persona::PlatformAdmin.can_access(Persona::PlatformAdmin));
        assert!(Persona::PlatformAdmin.can_access(Persona::TenantAdmin));
        assert!(Persona::PlatformAdmin.can_access(Persona::Anonymous));
    }

    #[test]
    fn tenant_admin_blocked_from_platform_only() {
        assert!(!Persona::TenantAdmin.can_access(Persona::PlatformAdmin));
        assert!(Persona::TenantAdmin.can_access(Persona::TenantAdmin));
        assert!(Persona::TenantAdmin.can_access(Persona::Anonymous));
    }

    #[test]
    fn anonymous_can_only_access_anonymous_tier() {
        assert!(!Persona::Anonymous.can_access(Persona::PlatformAdmin));
        assert!(!Persona::Anonymous.can_access(Persona::TenantAdmin));
        assert!(Persona::Anonymous.can_access(Persona::Anonymous));
    }
}

/// High-level role the caller has at sign-in time. Derived from the
/// JWT cookie's `roles` claim (`platform_admin` → `PlatformAdmin`,
/// `tenant_admin` → `TenantAdmin`). Anonymous callers (no cookie)
/// land in `Anonymous`, which is exactly the dev `?tenant_id=...`
/// shortcut that already drives the smoke / portal-mount tests.
///
/// Cite: cave-auth dev users in
/// `crates/cave-runtime/src/portal/auth.rs::DEV_USERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Persona {
    /// Platform staff — full Cave control plane access (Charter
    /// dashboard, ADR Browser, upstream parity, sweep planner).
    PlatformAdmin,
    /// Tenant admin — manages a single tenant's workloads (KEDA
    /// ScaledObjects, Vault secrets, kubelet pods, …) but does NOT
    /// see Charter/ADR/cross-tenant compliance.
    TenantAdmin,
    /// No JWT cookie — dev shortcut that grants tenant-scoped
    /// surfaces but is blocked from platform-only ones.
    Anonymous,
}

impl Persona {
    /// Parse from a list of JWT role strings. First matching role
    /// wins; missing input → `Anonymous`.
    pub fn from_roles<S: AsRef<str>>(roles: &[S]) -> Persona {
        for r in roles {
            match r.as_ref() {
                "platform_admin" => return Persona::PlatformAdmin,
                "tenant_admin" => return Persona::TenantAdmin,
                _ => {}
            }
        }
        Persona::Anonymous
    }

    /// Stable wire name for logs / JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            Persona::PlatformAdmin => "platform_admin",
            Persona::TenantAdmin => "tenant_admin",
            Persona::Anonymous => "anonymous",
        }
    }

    /// Whether this persona can access platform-wide surfaces (ADR
    /// browser, compliance dashboard, upstream parity).
    pub fn is_platform(&self) -> bool {
        matches!(self, Persona::PlatformAdmin)
    }

    /// True iff a caller of `self` is allowed to view a surface that
    /// requires at least `min`. PlatformAdmin can access everything;
    /// TenantAdmin can access TenantAdmin + Anonymous-tier surfaces;
    /// Anonymous can access only Anonymous-tier surfaces.
    ///
    /// Used by the command palette + shortcuts to filter their entry
    /// lists per persona without going through the full
    /// [`RequestCtx::require_persona`] error path. Same elevation
    /// rules as `require_persona`, with `Anonymous` treated as the
    /// public floor that everyone passes.
    pub fn can_access(self, min: Persona) -> bool {
        match (self, min) {
            (a, b) if a == b => true,
            (Persona::PlatformAdmin, _) => true,
            (Persona::TenantAdmin, Persona::Anonymous) => true,
            _ => false,
        }
    }
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
    /// High-level role derived from the JWT cookie's `roles` claim.
    /// Anonymous in the dev `?tenant_id=...` smoke flow.
    pub persona: Persona,
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

    /// Persona-only gate for routes that are platform-wide (Charter
    /// compliance, ADR Browser, upstream parity). Runs BEFORE
    /// permission/tenant checks so a misconfigured permission bag
    /// can't accidentally grant a tenant the Charter dashboard.
    pub fn require_persona(&self, required: Persona) -> Result<(), AuthError> {
        match (required, self.persona) {
            // Exact match always passes.
            (a, b) if a == b => Ok(()),
            // Platform admin can access anything a lower persona can.
            (Persona::TenantAdmin, Persona::PlatformAdmin) => Ok(()),
            _ => Err(AuthError::PersonaForbidden {
                actual: self.persona,
                required,
            }),
        }
    }

    /// Convenience: build a "developer" context for tests. Defaults
    /// to `Persona::PlatformAdmin` so existing tests (which pre-date
    /// the persona gate) keep passing.
    pub fn developer(tenant: &str, perms: &[Permission]) -> Self {
        let mut grants = BTreeSet::new();
        grants.insert(tenant.to_string());
        Self {
            principal: format!("spiffe://cluster.local/ns/{tenant}/sa/dev"),
            tenant: TenantId::new(tenant).expect("test fixture"),
            has_webauthn: true,
            tenant_grants: grants,
            permissions: perms.iter().copied().collect(),
            persona: Persona::PlatformAdmin,
        }
    }

    /// Like [`developer`] but with an explicit persona — used by
    /// tests that exercise the persona gate.
    pub fn developer_as(tenant: &str, perms: &[Permission], persona: Persona) -> Self {
        let mut ctx = Self::developer(tenant, perms);
        ctx.persona = persona;
        ctx
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

    // ── Persona gate ────────────────────────────────────────────

    #[test]
    fn persona_from_roles_picks_first_match() {
        assert_eq!(
            Persona::from_roles(&["platform_admin"]),
            Persona::PlatformAdmin
        );
        assert_eq!(
            Persona::from_roles(&["tenant_admin"]),
            Persona::TenantAdmin
        );
        assert_eq!(Persona::from_roles::<&str>(&[]), Persona::Anonymous);
        assert_eq!(Persona::from_roles(&["unknown"]), Persona::Anonymous);
        // Multiple roles: platform wins over tenant.
        assert_eq!(
            Persona::from_roles(&["tenant_admin", "platform_admin"]),
            Persona::TenantAdmin,
            "first-matching-role semantics — order matters in the JWT"
        );
    }

    #[test]
    fn platform_admin_can_access_tenant_surfaces() {
        let ctx = RequestCtx::developer_as(
            "tenant-platform-down",
            &[Permission::KedaRead],
            Persona::PlatformAdmin,
        );
        assert!(ctx.require_persona(Persona::TenantAdmin).is_ok());
        assert!(ctx.require_persona(Persona::PlatformAdmin).is_ok());
    }

    #[test]
    fn tenant_admin_cannot_access_platform_surfaces() {
        let ctx = RequestCtx::developer_as(
            "tenant-blocked",
            &[Permission::DashboardRead],
            Persona::TenantAdmin,
        );
        let err = ctx.require_persona(Persona::PlatformAdmin).unwrap_err();
        assert_eq!(
            err,
            AuthError::PersonaForbidden {
                actual: Persona::TenantAdmin,
                required: Persona::PlatformAdmin,
            }
        );
        // Tenant surfaces still work.
        assert!(ctx.require_persona(Persona::TenantAdmin).is_ok());
    }

    #[test]
    fn anonymous_cannot_access_platform_surfaces() {
        let ctx = RequestCtx::developer_as(
            "tenant-anon",
            &[Permission::DashboardRead],
            Persona::Anonymous,
        );
        assert!(ctx.require_persona(Persona::PlatformAdmin).is_err());
        assert!(ctx.require_persona(Persona::TenantAdmin).is_err());
    }

    #[test]
    fn persona_as_str_round_trips_to_wire_name() {
        assert_eq!(Persona::PlatformAdmin.as_str(), "platform_admin");
        assert_eq!(Persona::TenantAdmin.as_str(), "tenant_admin");
        assert_eq!(Persona::Anonymous.as_str(), "anonymous");
    }
}
