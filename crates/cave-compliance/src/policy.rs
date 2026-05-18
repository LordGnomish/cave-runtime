// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{PolicyEngine, PolicyMapping};
use uuid::Uuid;

pub fn create_mapping(control_id: Uuid, control_ref: &str, engine: PolicyEngine, policy_name: &str, namespace: Option<&str>, description: &str) -> PolicyMapping {
    PolicyMapping {
        id: Uuid::new_v4(),
        control_id,
        control_ref: control_ref.to_string(),
        policy_engine: engine,
        policy_name: policy_name.to_string(),
        policy_namespace: namespace.map(|s| s.to_string()),
        description: description.to_string(),
        created_at: chrono::Utc::now(),
    }
}

/// Get suggested policy mappings for common CIS controls.
pub fn suggested_mappings(control_ref: &str) -> Vec<(&'static str, &'static str, &'static str)> {
    // returns (engine, policy_name, description)
    match control_ref {
        "CIS-5.2.1" => vec![("kyverno", "disallow-privileged-containers", "Kyverno policy to block privileged containers")],
        "CIS-5.2.2" => vec![("kyverno", "disallow-host-pid", "Kyverno policy to block hostPID")],
        "CIS-5.3.1" => vec![("kyverno", "require-network-policy", "Kyverno policy to require NetworkPolicy in all namespaces")],
        "CIS-5.1.1" => vec![("opa", "rbac-required", "OPA rule requiring RBAC authorization mode")],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_create_mapping() {
        let id = Uuid::new_v4();
        let m = create_mapping(id, "CIS-5.2.1", PolicyEngine::Kyverno, "disallow-privileged", None, "Block privileged containers");
        assert_eq!(m.control_ref, "CIS-5.2.1");
    }
    #[test]
    fn test_suggested_mappings() {
        let suggestions = suggested_mappings("CIS-5.2.1");
        assert!(!suggestions.is_empty());
    }
}
