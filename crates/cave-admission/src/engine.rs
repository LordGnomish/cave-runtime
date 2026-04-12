//! Policy evaluation engine for cave-admission.

use crate::models::{
    AdmissionResult, GenerateRule, MutationPatch, Operation, Policy, PolicyRule, PolicySpec,
    Resource, VerifyImagesRule, Violation,
};
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

/// Check whether a policy matches the given resource and operation.
pub fn matches_policy(policy: &Policy, resource: &Resource, operation: Operation) -> bool {
    if !policy.enabled {
        return false;
    }
    let m = &policy.match_criteria;

    if !m.operations.is_empty() && !m.operations.contains(&operation) {
        return false;
    }
    if !m.kinds.is_empty() && !m.kinds.iter().any(|k| k == "*" || k == &resource.kind) {
        return false;
    }
    if !m.namespaces.is_empty() {
        let ns = resource.metadata.namespace.as_deref().unwrap_or("default");
        if !m.namespaces.iter().any(|n| n == "*" || n == ns) {
            return false;
        }
    }
    if let Some(selector) = &m.label_selector {
        for (key, value) in selector {
            if resource.metadata.labels.get(key) != Some(value) {
                return false;
            }
        }
    }
    true
}

/// Evaluate a single validation rule against a resource, returning a violation
/// message if the rule is violated, or `None` if it passes.
pub fn evaluate_validation_rule(rule: &PolicyRule, resource: &Resource) -> Option<String> {
    match rule {
        PolicyRule::RequiredLabel { key, allowed_values } => {
            match resource.metadata.labels.get(key) {
                None => Some(format!("Required label '{}' is missing", key)),
                Some(v) if !allowed_values.is_empty() && !allowed_values.contains(v) => Some(
                    format!(
                        "Label '{}' has disallowed value '{}'; allowed: {:?}",
                        key, v, allowed_values
                    ),
                ),
                _ => None,
            }
        }
        PolicyRule::RequiredAnnotation { key } => {
            if resource.metadata.annotations.contains_key(key) {
                None
            } else {
                Some(format!("Required annotation '{}' is missing", key))
            }
        }
        PolicyRule::DisallowPrivileged => {
            let privileged = resource
                .spec
                .pointer("/securityContext/privileged")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if privileged {
                Some("Privileged containers are not allowed".to_string())
            } else {
                None
            }
        }
        PolicyRule::RequireResourceLimits => {
            if resource.spec.pointer("/resources/limits").is_some() {
                None
            } else {
                Some("Resource limits must be specified".to_string())
            }
        }
        PolicyRule::AllowedRegistries { registries } => {
            let image = resource
                .spec
                .pointer("/image")
                .and_then(Value::as_str)
                .unwrap_or("");
            if image.is_empty() || registries.iter().any(|r| image.starts_with(r.as_str())) {
                None
            } else {
                Some(format!(
                    "Image '{}' is not from an allowed registry: {:?}",
                    image, registries
                ))
            }
        }
        PolicyRule::MaxReplicas { max } => {
            let replicas = resource
                .spec
                .pointer("/replicas")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if replicas > *max as u64 {
                Some(format!("Replicas {} exceeds maximum {}", replicas, max))
            } else {
                None
            }
        }
        PolicyRule::RequiredNamespace { namespaces } => {
            let ns = resource.metadata.namespace.as_deref().unwrap_or("default");
            if namespaces.iter().any(|n| n == ns) {
                None
            } else {
                Some(format!("Resource must be in one of: {:?}", namespaces))
            }
        }
    }
}

/// Return the patches to apply for a Mutate policy (identity — caller applies them).
pub fn evaluate_mutation(patches: &[MutationPatch], _resource: &Resource) -> Vec<MutationPatch> {
    patches.to_vec()
}

/// Evaluate image verification against a resource. Returns a violation message or `None`.
pub fn evaluate_verify_images(rule: &VerifyImagesRule, resource: &Resource) -> Option<String> {
    let image = resource
        .spec
        .pointer("/image")
        .and_then(Value::as_str)
        .unwrap_or("");

    if !rule.allowed_registries.is_empty() && !image.is_empty() {
        if !rule
            .allowed_registries
            .iter()
            .any(|r| image.starts_with(r.as_str()))
        {
            return Some(format!(
                "Image '{}' registry is not in the allowed list",
                image
            ));
        }
    }

    if rule.require_signature && !image.is_empty() {
        // Simulate signature check: images without a digest reference are considered unsigned.
        if !image.contains('@') {
            return Some(format!(
                "Image '{}' has no verified signature (no digest reference)",
                image
            ));
        }
    }
    None
}

/// Build the companion resource document for a Generate policy.
fn build_generated_resource(generate: &GenerateRule, resource: &Resource) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": generate.api_version,
        "kind": generate.kind,
        "metadata": {
            "name": generate.name_template.replace("{{name}}", &resource.metadata.name),
            "namespace": resource.metadata.namespace,
        },
        "spec": generate.spec,
    })
}

/// Evaluate a single policy against a resource/operation pair.
pub fn evaluate_policy(
    policy: &Policy,
    resource: &Resource,
    _operation: Operation,
) -> AdmissionResult {
    let mut violations: Vec<Violation> = Vec::new();
    let mut mutations: Vec<MutationPatch> = Vec::new();
    let mut generated_resources: Vec<serde_json::Value> = Vec::new();

    match &policy.spec {
        PolicySpec::Validate { rules } => {
            for rule in rules {
                if let Some(msg) = evaluate_validation_rule(rule, resource) {
                    violations.push(Violation {
                        id: Uuid::new_v4(),
                        policy_id: policy.id,
                        policy_name: policy.name.clone(),
                        resource_kind: resource.kind.clone(),
                        resource_name: resource.metadata.name.clone(),
                        resource_namespace: resource.metadata.namespace.clone(),
                        message: msg,
                        timestamp: Utc::now(),
                    });
                }
            }
        }
        PolicySpec::Mutate { patches } => {
            mutations = evaluate_mutation(patches, resource);
        }
        PolicySpec::Generate { generate } => {
            generated_resources.push(build_generated_resource(generate, resource));
        }
        PolicySpec::VerifyImages { rule } => {
            if let Some(msg) = evaluate_verify_images(rule, resource) {
                violations.push(Violation {
                    id: Uuid::new_v4(),
                    policy_id: policy.id,
                    policy_name: policy.name.clone(),
                    resource_kind: resource.kind.clone(),
                    resource_name: resource.metadata.name.clone(),
                    resource_namespace: resource.metadata.namespace.clone(),
                    message: msg,
                    timestamp: Utc::now(),
                });
            }
        }
    }

    // Audit mode: record violations but still allow the request.
    let allowed = violations.is_empty() || policy.audit_mode;

    AdmissionResult { allowed, violations, mutations, generated_resources }
}

/// Evaluate all matching policies against a resource, merging results.
pub fn evaluate_all_policies(
    policies: &[Policy],
    resource: &Resource,
    operation: Operation,
) -> AdmissionResult {
    let mut allowed = true;
    let mut all_violations = Vec::new();
    let mut all_mutations = Vec::new();
    let mut all_generated = Vec::new();

    for policy in policies {
        if !matches_policy(policy, resource, operation) {
            continue;
        }
        let result = evaluate_policy(policy, resource, operation);
        if !result.allowed {
            allowed = false;
        }
        all_violations.extend(result.violations);
        all_mutations.extend(result.mutations);
        all_generated.extend(result.generated_resources);
    }

    AdmissionResult {
        allowed,
        violations: all_violations,
        mutations: all_mutations,
        generated_resources: all_generated,
    }
}

/// Built-in default policies.
pub fn builtin_policies() -> Vec<Policy> {
    use crate::models::{PolicyMatch, PolicyRule, PolicySpec};

    vec![
        Policy {
            id: Uuid::new_v4(),
            name: "require-app-label".to_string(),
            description: "All Pods must carry an 'app' label".to_string(),
            match_criteria: PolicyMatch {
                kinds: vec!["Pod".to_string()],
                namespaces: Vec::new(),
                operations: vec![Operation::Create, Operation::Update],
                label_selector: None,
            },
            spec: PolicySpec::Validate {
                rules: vec![PolicyRule::RequiredLabel {
                    key: "app".to_string(),
                    allowed_values: Vec::new(),
                }],
            },
            audit_mode: false,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        Policy {
            id: Uuid::new_v4(),
            name: "require-resource-limits".to_string(),
            description: "Containers must specify resource limits (audit only)".to_string(),
            match_criteria: PolicyMatch {
                kinds: vec!["Pod".to_string()],
                namespaces: Vec::new(),
                operations: vec![Operation::Create],
                label_selector: None,
            },
            spec: PolicySpec::Validate {
                rules: vec![PolicyRule::RequireResourceLimits],
            },
            audit_mode: true,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        Policy {
            id: Uuid::new_v4(),
            name: "allowed-registries".to_string(),
            description: "Only images from approved registries are permitted".to_string(),
            match_criteria: PolicyMatch {
                kinds: vec!["Pod".to_string()],
                namespaces: Vec::new(),
                operations: vec![Operation::Create],
                label_selector: None,
            },
            spec: PolicySpec::Validate {
                rules: vec![PolicyRule::AllowedRegistries {
                    registries: vec![
                        "gcr.io/".to_string(),
                        "ghcr.io/".to_string(),
                        "registry.k8s.io/".to_string(),
                    ],
                }],
            },
            audit_mode: false,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
        Policy {
            id: Uuid::new_v4(),
            name: "inject-managed-by".to_string(),
            description: "Inject 'managed-by: cave' label via mutation".to_string(),
            match_criteria: PolicyMatch {
                kinds: vec!["Pod".to_string()],
                namespaces: Vec::new(),
                operations: vec![Operation::Create],
                label_selector: None,
            },
            spec: PolicySpec::Mutate {
                patches: vec![crate::models::MutationPatch {
                    op: crate::models::PatchOp::Add,
                    path: "/metadata/labels/managed-by".to_string(),
                    value: Some(serde_json::json!("cave")),
                }],
            },
            audit_mode: false,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    ]
}
