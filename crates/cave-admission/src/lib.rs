// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Admission — Admission controller replacing Kyverno + Gatekeeper.
//!
//! Policy types:
//! - **Validate**: enforce rules (required labels, resource limits, allowed registries, …)
//! - **Mutate**: inject patches (sidecar injection, default labels, …)
//! - **Generate**: create companion resources (NetworkPolicy, ResourceQuota, …)
//! - **VerifyImages**: verify container image signatures (cosign/notation)
//!
//! Matching: by resource kind, namespace, labels/annotations, and operation
//! (CREATE / UPDATE / DELETE).
//!
//! REST API: /api/v1/policies, /api/v1/admission/review,
//!           /api/v1/reports, /api/v1/violations

pub mod engine;
pub mod models;
pub mod routes;

use axum::Router;
use engine::builtin_policies;
use models::{Policy, Violation};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Module-wide shared state.
pub struct AdmissionState {
    pub policies: RwLock<Vec<Policy>>,
    pub violations: RwLock<Vec<Violation>>,
}

impl Default for AdmissionState {
    fn default() -> Self {
        Self {
            policies: RwLock::new(builtin_policies()),
            violations: RwLock::new(Vec::new()),
        }
    }
}

pub fn router(state: Arc<AdmissionState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "admission";

#[cfg(test)]
mod tests {
    use crate::engine::*;
    use crate::models::*;
    use std::collections::HashMap;

    fn make_resource(kind: &str, name: &str, ns: Option<&str>) -> Resource {
        Resource {
            api_version: "v1".to_string(),
            kind: kind.to_string(),
            metadata: ResourceMeta {
                name: name.to_string(),
                namespace: ns.map(str::to_string),
                labels: HashMap::new(),
                annotations: HashMap::new(),
            },
            spec: serde_json::json!({}),
        }
    }

    fn pod_with_app_label(name: &str) -> Resource {
        let mut r = make_resource("Pod", name, Some("default"));
        r.metadata.labels.insert("app".to_string(), "my-app".to_string());
        r
    }

    fn make_policy(kinds: Vec<String>, operations: Vec<Operation>, rules: Vec<PolicyRule>, audit: bool) -> Policy {
        Policy {
            id: uuid::Uuid::new_v4(),
            name: "test-policy".to_string(),
            description: "".to_string(),
            match_criteria: PolicyMatch {
                kinds,
                namespaces: Vec::new(),
                operations,
                label_selector: None,
            },
            spec: PolicySpec::Validate { rules },
            audit_mode: audit,
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // 1
    #[test]
    fn test_policy_matches_by_kind() {
        let policy = make_policy(vec!["Pod".to_string()], Vec::new(), Vec::new(), false);
        let pod = make_resource("Pod", "p", Some("default"));
        let dep = make_resource("Deployment", "d", Some("default"));
        assert!(matches_policy(&policy, &pod, Operation::Create));
        assert!(!matches_policy(&policy, &dep, Operation::Create));
    }

    // 2
    #[test]
    fn test_policy_matches_wildcard_kind() {
        let policy = make_policy(vec!["*".to_string()], Vec::new(), Vec::new(), false);
        let pod = make_resource("Pod", "p", Some("default"));
        let svc = make_resource("Service", "s", Some("default"));
        assert!(matches_policy(&policy, &pod, Operation::Create));
        assert!(matches_policy(&policy, &svc, Operation::Delete));
    }

    // 3
    #[test]
    fn test_policy_does_not_match_when_disabled() {
        let mut policy = make_policy(Vec::new(), Vec::new(), Vec::new(), false);
        policy.enabled = false;
        let pod = make_resource("Pod", "p", Some("default"));
        assert!(!matches_policy(&policy, &pod, Operation::Create));
    }

    // 4
    #[test]
    fn test_policy_matches_by_operation() {
        let policy = make_policy(Vec::new(), vec![Operation::Create], Vec::new(), false);
        let pod = make_resource("Pod", "p", Some("default"));
        assert!(matches_policy(&policy, &pod, Operation::Create));
        assert!(!matches_policy(&policy, &pod, Operation::Delete));
    }

    // 5
    #[test]
    fn test_policy_matches_by_namespace() {
        let mut policy = make_policy(Vec::new(), Vec::new(), Vec::new(), false);
        policy.match_criteria.namespaces = vec!["prod".to_string()];
        let pod_prod = make_resource("Pod", "p", Some("prod"));
        let pod_dev = make_resource("Pod", "p", Some("dev"));
        assert!(matches_policy(&policy, &pod_prod, Operation::Create));
        assert!(!matches_policy(&policy, &pod_dev, Operation::Create));
    }

    // 6
    #[test]
    fn test_required_label_missing_is_violation() {
        let rule = PolicyRule::RequiredLabel { key: "app".to_string(), allowed_values: Vec::new() };
        let pod = make_resource("Pod", "p", Some("default"));
        let result = evaluate_validation_rule(&rule, &pod);
        assert!(result.is_some());
        assert!(result.unwrap().contains("app"));
    }

    // 7
    #[test]
    fn test_required_label_present_passes() {
        let rule = PolicyRule::RequiredLabel { key: "app".to_string(), allowed_values: Vec::new() };
        assert!(evaluate_validation_rule(&rule, &pod_with_app_label("p")).is_none());
    }

    // 8
    #[test]
    fn test_required_label_wrong_value_is_violation() {
        let rule = PolicyRule::RequiredLabel {
            key: "env".to_string(),
            allowed_values: vec!["prod".to_string(), "staging".to_string()],
        };
        let mut pod = make_resource("Pod", "p", Some("default"));
        pod.metadata.labels.insert("env".to_string(), "dev".to_string());
        let result = evaluate_validation_rule(&rule, &pod);
        assert!(result.is_some());
        assert!(result.unwrap().contains("disallowed value"));
    }

    // 9
    #[test]
    fn test_disallow_privileged_container() {
        let rule = PolicyRule::DisallowPrivileged;
        let mut pod = make_resource("Pod", "p", Some("default"));
        pod.spec = serde_json::json!({"securityContext": {"privileged": true}});
        assert!(evaluate_validation_rule(&rule, &pod).is_some());
        pod.spec = serde_json::json!({"securityContext": {"privileged": false}});
        assert!(evaluate_validation_rule(&rule, &pod).is_none());
    }

    // 10
    #[test]
    fn test_require_resource_limits() {
        let rule = PolicyRule::RequireResourceLimits;
        let pod = make_resource("Pod", "p", Some("default"));
        assert!(evaluate_validation_rule(&rule, &pod).is_some());
        let mut pod2 = make_resource("Pod", "p", Some("default"));
        pod2.spec = serde_json::json!({"resources": {"limits": {"cpu": "100m"}}});
        assert!(evaluate_validation_rule(&rule, &pod2).is_none());
    }

    // 11
    #[test]
    fn test_allowed_registries_blocks_docker_hub() {
        let rule = PolicyRule::AllowedRegistries {
            registries: vec!["gcr.io/".to_string()],
        };
        let mut pod = make_resource("Pod", "p", Some("default"));
        pod.spec = serde_json::json!({"image": "docker.io/nginx:latest"});
        assert!(evaluate_validation_rule(&rule, &pod).is_some());
        pod.spec = serde_json::json!({"image": "gcr.io/my-project/app:v1"});
        assert!(evaluate_validation_rule(&rule, &pod).is_none());
    }

    // 12
    #[test]
    fn test_mutation_returns_patches_unchanged() {
        let patches = vec![MutationPatch {
            op: PatchOp::Add,
            path: "/metadata/labels/managed-by".to_string(),
            value: Some(serde_json::json!("cave")),
        }];
        let pod = make_resource("Pod", "p", Some("default"));
        let result = evaluate_mutation(&patches, &pod);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "/metadata/labels/managed-by");
    }

    // 13
    #[test]
    fn test_evaluate_all_policies_blocks_on_violation() {
        let policies = builtin_policies();
        // Pod without 'app' label — require-app-label is not audit mode → blocked
        let pod = make_resource("Pod", "no-label-pod", Some("default"));
        let result = evaluate_all_policies(&policies, &pod, Operation::Create);
        assert!(!result.allowed);
        assert!(!result.violations.is_empty());
    }

    // 14
    #[test]
    fn test_evaluate_all_policies_passes_with_required_labels() {
        let policies = builtin_policies();
        // Pod with 'app' label; require-resource-limits is audit-only → allowed
        let result =
            evaluate_all_policies(&policies, &pod_with_app_label("good-pod"), Operation::Create);
        assert!(result.allowed);
    }

    // 15
    #[test]
    fn test_verify_images_blocks_unsigned_image() {
        let rule = VerifyImagesRule {
            allowed_registries: vec!["gcr.io/".to_string()],
            require_signature: true,
            key_ref: None,
        };
        let mut pod = make_resource("Pod", "p", None);
        // Image with digest → considered signed
        pod.spec = serde_json::json!({"image": "gcr.io/proj/app@sha256:abc"});
        assert!(evaluate_verify_images(&rule, &pod).is_none());
        // Image without digest → unsigned
        pod.spec = serde_json::json!({"image": "gcr.io/proj/app:latest"});
        assert!(evaluate_verify_images(&rule, &pod).is_some());
    }

    // 16
    #[test]
    fn test_verify_images_blocks_disallowed_registry() {
        let rule = VerifyImagesRule {
            allowed_registries: vec!["gcr.io/".to_string()],
            require_signature: false,
            key_ref: None,
        };
        let mut pod = make_resource("Pod", "p", None);
        pod.spec = serde_json::json!({"image": "docker.io/nginx:latest"});
        assert!(evaluate_verify_images(&rule, &pod).is_some());
    }

    // 17
    #[test]
    fn test_audit_mode_allows_but_records_violation() {
        let policy = make_policy(
            vec!["Pod".to_string()],
            Vec::new(),
            vec![PolicyRule::RequiredLabel { key: "env".to_string(), allowed_values: Vec::new() }],
            true, // audit_mode = true
        );
        let pod = make_resource("Pod", "p", Some("default"));
        let result = evaluate_policy(&policy, &pod, Operation::Create);
        // Audit mode: allowed even though there's a violation
        assert!(result.allowed);
        assert!(!result.violations.is_empty());
    }

    // 18 (bonus)
    #[test]
    fn test_builtin_policies_all_enabled() {
        let policies = builtin_policies();
        assert!(policies.len() >= 3);
        assert!(policies.iter().all(|p| p.enabled));
    }
}
