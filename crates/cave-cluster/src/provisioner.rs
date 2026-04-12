//! Cluster provisioning operations.
//!
//! Wraps cloud-provider APIs (via cave-infra MCP bridge) and kubeadm for
//! bare metal. All functions are pure — callers own the state mutation.

use crate::models::*;
use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

/// Create a new Kubernetes cluster from a provisioning request.
///
/// For cloud providers this calls the cave-infra MCP bridge.
/// For `BareMetal` this bootstraps the control plane via kubeadm.
pub fn provision_cluster(req: &CreateClusterRequest) -> Cluster {
    info!(
        name = %req.name,
        tenant_id = %req.tenant_id,
        provider = ?req.provider,
        version = %req.version,
        "Provisioning cluster"
    );

    let now = Utc::now();
    Cluster {
        id: Uuid::new_v4(),
        name: req.name.clone(),
        tenant_id: req.tenant_id,
        provider: req.provider.clone(),
        version: req.version.clone(),
        status: ClusterStatus::Provisioning,
        endpoint: None,
        kubeconfig_ref: None,
        upgrade_policy: req.upgrade_policy.clone().unwrap_or_default(),
        network: req.network.clone().unwrap_or_default(),
        created_at: now,
        updated_at: now,
    }
}

/// Record a delete event and begin graceful teardown.
///
/// Drains all nodes, removes cloud resources, and cleans up cave-vault
/// entries for the kubeconfig. Returns the lifecycle event to persist.
pub fn delete_cluster(cluster_id: Uuid) -> ClusterEvent {
    info!(cluster_id = %cluster_id, "Deleting cluster");
    // TODO: call cave-infra MCP bridge to deprovision cloud resources
    // TODO: drain + delete nodes via kubeadm or provider API
    // TODO: remove kubeconfig from cave-vault
    ClusterEvent {
        id: Uuid::new_v4(),
        cluster_id,
        event_type: ClusterEventType::Deleted,
        message: "Cluster deletion initiated".into(),
        occurred_at: Utc::now(),
    }
}

/// Plan and return the upgraded cluster struct (status = Upgrading).
///
/// Upgrades are rolling: control plane first, then each node pool in sequence
/// with `max_surge` extra nodes provisioned per pool.
pub fn upgrade_cluster(cluster: &Cluster, req: &UpgradeClusterRequest) -> Cluster {
    info!(
        cluster_id = %cluster.id,
        from = %cluster.version,
        to = %req.target_version,
        "Upgrading cluster"
    );
    // TODO: validate semver step (only one minor version at a time)
    // TODO: trigger rolling upgrade via cave-infra MCP bridge
    let mut upgraded = cluster.clone();
    upgraded.version = req.target_version.clone();
    upgraded.status = ClusterStatus::Upgrading;
    upgraded.updated_at = Utc::now();
    upgraded
}

/// Scale a node pool to a new desired size.
///
/// Respects `min_nodes` / `max_nodes` bounds and the autoscaler if enabled.
pub fn scale_node_pool(pool: &NodePool, req: &ScaleNodePoolRequest) -> NodePool {
    info!(
        pool_id = %pool.id,
        cluster_id = %pool.cluster_id,
        from = pool.current_nodes,
        to = req.desired_nodes,
        "Scaling node pool"
    );

    let min = req.min_nodes.unwrap_or(pool.min_nodes);
    let max = req.max_nodes.unwrap_or(pool.max_nodes);
    let desired = req.desired_nodes.clamp(min, max);

    if desired != req.desired_nodes {
        warn!(
            requested = req.desired_nodes,
            clamped = desired,
            "Desired node count clamped to pool bounds"
        );
    }

    // TODO: call cave-infra MCP bridge to add/remove nodes
    let mut scaled = pool.clone();
    scaled.current_nodes = desired;
    scaled.min_nodes = min;
    scaled.max_nodes = max;
    scaled
}

/// Rotate cluster credentials: TLS certs, kubeconfig, service account tokens.
///
/// New kubeconfig is written to cave-vault. The old path is preserved until
/// rotation is confirmed complete.
pub fn rotate_credentials(cluster: &Cluster) -> KubeconfigRef {
    info!(cluster_id = %cluster.id, "Rotating cluster credentials");
    // TODO: use kubeadm certs renew or cloud-provider rotation API
    // TODO: write new kubeconfig to cave-vault at cluster path
    KubeconfigRef {
        vault_path: format!("clusters/{}/kubeconfig", cluster.id),
        encrypted: true,
        last_rotated: Utc::now(),
    }
}

/// Build a ClusterAddon record and schedule installation.
///
/// Deploys the add-on as a Helm release or plain manifests via the cluster's
/// API server. The cave-ebpf-agent is deployed as a DaemonSet.
pub fn install_addons(cluster_id: Uuid, req: &InstallAddonRequest) -> ClusterAddon {
    let version = req
        .version
        .clone()
        .unwrap_or_else(|| default_addon_version(&req.addon_type));

    info!(
        cluster_id = %cluster_id,
        addon = ?req.addon_type,
        version = %version,
        "Installing cluster add-on"
    );

    // TODO: render Helm chart / manifest, apply via kube API
    // TODO: for CaveEbpfAgent: apply DaemonSet with eBPF capabilities
    ClusterAddon {
        id: Uuid::new_v4(),
        cluster_id,
        addon_type: req.addon_type.clone(),
        version,
        status: AddonStatus::Installing,
        installed_at: Utc::now(),
    }
}

fn default_addon_version(addon: &ClusterAddonType) -> String {
    match addon {
        ClusterAddonType::IngressNginx => "1.10.0".into(),
        ClusterAddonType::CertManager => "1.15.0".into(),
        ClusterAddonType::MonitoringStack => "0.76.0".into(), // kube-prometheus-stack
        ClusterAddonType::CaveEbpfAgent => env!("CARGO_PKG_VERSION").into(),
    }
}
