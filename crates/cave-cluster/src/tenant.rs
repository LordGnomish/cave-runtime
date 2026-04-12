//! Tenant management: registration, quota enforcement, and network isolation.
//!
//! Tenants are the top-level billing and isolation boundary. Each tenant gets
//! their own clusters; cross-tenant network traffic is blocked via CNI
//! NetworkPolicies deployed by `isolate_tenant`.

use crate::models::*;
use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

/// Register a new tenant with the given quota and billing info.
pub fn create_tenant(req: &CreateTenantRequest) -> Tenant {
    info!(name = %req.name, "Creating tenant");
    Tenant {
        id: Uuid::new_v4(),
        name: req.name.clone(),
        quota: req.quota.clone(),
        billing_info: req.billing_info.clone(),
        created_at: Utc::now(),
    }
}

/// Calculate a tenant's current quota usage from their live clusters.
pub fn calculate_quota_usage(
    tenant: &Tenant,
    clusters: &[Cluster],
    node_pools: &[NodePool],
) -> QuotaUsage {
    let clusters_used = clusters.len() as u32;
    let nodes_used: u32 = node_pools.iter().map(|p| p.current_nodes).sum();

    // TODO: look up actual CPU/memory per instance type from cave-infra
    let cpu_used: u32 = nodes_used * 4; // placeholder: assume 4 vCPU per node
    let memory_used_gib: u64 = nodes_used as u64 * 8; // placeholder: 8 GiB per node

    QuotaUsage {
        clusters_used,
        clusters_limit: tenant.quota.max_clusters,
        nodes_used,
        nodes_limit: tenant.quota.max_nodes,
        cpu_used,
        cpu_limit: tenant.quota.max_cpu,
        memory_used_gib,
        memory_limit_gib: tenant.quota.max_memory_gib,
    }
}

/// Return `true` if the tenant can create one more cluster, `false` if at limit.
pub fn enforce_quota(tenant: &Tenant, quota_usage: &QuotaUsage) -> bool {
    let allowed = quota_usage.clusters_used < quota_usage.clusters_limit
        && quota_usage.nodes_used < quota_usage.nodes_limit
        && quota_usage.cpu_used < quota_usage.cpu_limit
        && quota_usage.memory_used_gib < quota_usage.memory_limit_gib;

    if !allowed {
        warn!(
            tenant_id = %tenant.id,
            clusters = %quota_usage.clusters_used,
            limit = %quota_usage.clusters_limit,
            "Tenant quota exceeded"
        );
    }

    allowed
}

/// Generate the `NetworkPolicy` that isolates a tenant's clusters.
///
/// The returned policy is applied to every new cluster for this tenant.
/// Cilium `CiliumNetworkPolicy` CRDs (TODO) are preferred over vanilla k8s
/// NetworkPolicies for cross-node enforcement.
pub fn isolate_tenant(tenant_id: Uuid) -> NetworkPolicy {
    info!(tenant_id = %tenant_id, "Building tenant network isolation policy");

    // TODO: allocate a unique pod/service CIDR per tenant from a central IPAM
    // TODO: generate Cilium policy that denies cross-tenant ingress/egress
    NetworkPolicy {
        pod_cidr: "10.244.0.0/16".into(),    // TODO: IPAM-allocated
        service_cidr: "10.96.0.0/12".into(), // TODO: IPAM-allocated
        cni_plugin: CniPlugin::Cilium,
        tenant_isolation: true,
    }
}

/// Aggregate all clusters for a tenant into a dashboard summary.
pub fn tenant_dashboard(
    tenant: &Tenant,
    clusters: Vec<Cluster>,
    node_pools: &[NodePool],
) -> TenantDashboard {
    let total_nodes: u32 = node_pools.iter().map(|p| p.current_nodes).sum();
    let quota_usage = calculate_quota_usage(tenant, &clusters, node_pools);

    TenantDashboard {
        tenant: tenant.clone(),
        clusters,
        total_nodes,
        quota_usage,
    }
}
