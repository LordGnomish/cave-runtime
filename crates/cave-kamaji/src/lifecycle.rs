use crate::models::{TenantControlPlane, TenantPhase};
use tracing::info;

pub fn provision(tcp: &mut TenantControlPlane) {
    info!(id = %tcp.id, name = %tcp.name, "Provisioning tenant control plane");
    tcp.status.phase = TenantPhase::Provisioning;
    tcp.status.message = Some("Control plane is being provisioned".into());
}

pub fn mark_running(tcp: &mut TenantControlPlane, endpoint: String) {
    info!(id = %tcp.id, name = %tcp.name, "Tenant control plane running");
    tcp.status.phase = TenantPhase::Running;
    tcp.status.api_server_endpoint = Some(endpoint);
    tcp.status.ready = true;
    tcp.status.message = None;
}

pub fn deprovision(tcp: &mut TenantControlPlane) {
    info!(id = %tcp.id, name = %tcp.name, "Deprovisioning tenant control plane");
    tcp.status.phase = TenantPhase::Deleting;
    tcp.status.ready = false;
    tcp.status.message = Some("Control plane is being deprovisioned".into());
}

pub fn health_check(tcp: &TenantControlPlane) -> bool {
    tcp.status.phase == TenantPhase::Running && tcp.status.ready
}

pub fn generate_kubeconfig(tcp: &TenantControlPlane) -> Option<serde_json::Value> {
    let endpoint = tcp.status.api_server_endpoint.as_ref()?;
    Some(serde_json::json!({
        "apiVersion": "v1",
        "kind": "Config",
        "clusters": [{
            "name": tcp.name,
            "cluster": { "server": endpoint }
        }],
        "contexts": [{
            "name": tcp.name,
            "context": { "cluster": tcp.name, "user": "admin" }
        }],
        "current-context": tcp.name,
        "users": [{ "name": "admin", "user": {} }]
    }))
}
