// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Network policy defaults and management.

use serde::{Deserialize, Serialize};

/// A Kubernetes NetworkPolicy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub name: String,
    pub namespace: String,
    pub pod_selector: PodSelector,
    pub policy_types: Vec<PolicyType>,
    pub ingress_rules: Vec<IngressRule>,
    pub egress_rules: Vec<EgressRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSelector {
    pub match_labels: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyType {
    Ingress,
    Egress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressRule {
    pub from: Vec<NetworkPolicyPeer>,
    pub ports: Vec<NetworkPolicyPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressRule {
    pub to: Vec<NetworkPolicyPeer>,
    pub ports: Vec<NetworkPolicyPort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicyPeer {
    pub namespace_selector: Option<PodSelector>,
    pub pod_selector: Option<PodSelector>,
    pub ip_block: Option<IpBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpBlock {
    pub cidr: String,
    pub except: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicyPort {
    pub protocol: String,
    pub port: Option<i32>,
}

/// Generate the default network policies for a new namespace.
pub fn default_namespace_policies(namespace: &str) -> Vec<NetworkPolicy> {
    let mut deny_all_ingress_labels: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let deny_all = NetworkPolicy {
        name: format!("{namespace}-default-deny-ingress"),
        namespace: namespace.to_string(),
        pod_selector: PodSelector { match_labels: std::collections::HashMap::new() },
        policy_types: vec![PolicyType::Ingress],
        ingress_rules: vec![],
        egress_rules: vec![],
    };

    // Allow intra-namespace communication
    let mut ns_labels: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    ns_labels.insert("kubernetes.io/metadata.name".into(), namespace.to_string());
    let allow_same_namespace = NetworkPolicy {
        name: format!("{namespace}-allow-same-namespace"),
        namespace: namespace.to_string(),
        pod_selector: PodSelector { match_labels: std::collections::HashMap::new() },
        policy_types: vec![PolicyType::Ingress],
        ingress_rules: vec![IngressRule {
            from: vec![NetworkPolicyPeer {
                namespace_selector: None,
                pod_selector: Some(PodSelector { match_labels: std::collections::HashMap::new() }),
                ip_block: None,
            }],
            ports: vec![],
        }],
        egress_rules: vec![],
    };

    // Allow DNS egress (port 53 to kube-dns)
    let allow_dns_egress = NetworkPolicy {
        name: format!("{namespace}-allow-dns"),
        namespace: namespace.to_string(),
        pod_selector: PodSelector { match_labels: std::collections::HashMap::new() },
        policy_types: vec![PolicyType::Egress],
        ingress_rules: vec![],
        egress_rules: vec![EgressRule {
            to: vec![NetworkPolicyPeer {
                namespace_selector: Some(PodSelector {
                    match_labels: {
                        let mut m = std::collections::HashMap::new();
                        m.insert("kubernetes.io/metadata.name".into(), "kube-system".into());
                        m
                    },
                }),
                pod_selector: Some(PodSelector {
                    match_labels: {
                        let mut m = std::collections::HashMap::new();
                        m.insert("k8s-app".into(), "kube-dns".into());
                        m
                    },
                }),
                ip_block: None,
            }],
            ports: vec![
                NetworkPolicyPort { protocol: "UDP".into(), port: Some(53) },
                NetworkPolicyPort { protocol: "TCP".into(), port: Some(53) },
            ],
        }],
    };

    vec![deny_all, allow_same_namespace, allow_dns_egress]
}

/// Serialize a NetworkPolicy to a Kubernetes YAML manifest.
pub fn to_yaml(policy: &NetworkPolicy) -> String {
    format!(
        r#"apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {name}
  namespace: {namespace}
spec:
  podSelector: {{}}
  policyTypes:
{types}
"#,
        name = policy.name,
        namespace = policy.namespace,
        types = policy.policy_types.iter().map(|t| format!("    - {:?}", t)).collect::<Vec<_>>().join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policies_for_namespace() {
        let policies = default_namespace_policies("production");
        assert!(!policies.is_empty());
        // Should include deny-all-ingress, allow-same-namespace, allow-dns
        assert!(policies.iter().any(|p| p.name.contains("deny")));
        assert!(policies.iter().any(|p| p.name.contains("allow")));
    }

    #[test]
    fn yaml_output() {
        let policies = default_namespace_policies("staging");
        let yaml = to_yaml(&policies[0]);
        assert!(yaml.contains("NetworkPolicy"));
        assert!(yaml.contains("staging"));
    }
}
