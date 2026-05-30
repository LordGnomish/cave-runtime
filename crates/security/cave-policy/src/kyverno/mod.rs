// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Kyverno-compatible policy engine.
//!
//! Implements: ClusterPolicy/Policy, validate/mutate/generate/verifyImages rules,
//! JMESPath variable substitution, PolicyReports, CleanupPolicies, PolicyExceptions.

pub mod cleanup;
pub mod exception;
pub mod generate;
pub mod image_verify;
pub mod jmespath;
pub mod models;
pub mod mutate;
pub mod validate;

use models::*;
use std::collections::HashMap;

pub use models::{
    CleanupPolicy, ClusterPolicy, ClusterPolicyReport, Policy, PolicyEvalResult, PolicyException,
    PolicyReport,
};

/// Kyverno policy engine — holds loaded policies and exceptions.
pub struct KyvernoEngine {
    cluster_policies: HashMap<String, ClusterPolicy>,
    policies: HashMap<String, Policy>,
    exceptions: HashMap<String, PolicyException>,
}

impl KyvernoEngine {
    pub fn new() -> Self {
        Self {
            cluster_policies: HashMap::new(),
            policies: HashMap::new(),
            exceptions: HashMap::new(),
        }
    }

    pub fn add_cluster_policy(&mut self, policy: ClusterPolicy) {
        self.cluster_policies
            .insert(policy.metadata.name.clone(), policy);
    }

    pub fn remove_cluster_policy(&mut self, name: &str) {
        self.cluster_policies.remove(name);
    }

    pub fn get_cluster_policy(&self, name: &str) -> Option<&ClusterPolicy> {
        self.cluster_policies.get(name)
    }

    pub fn list_cluster_policies(&self) -> Vec<&ClusterPolicy> {
        self.cluster_policies.values().collect()
    }

    pub fn add_policy(&mut self, policy: Policy) {
        let key = format!(
            "{}/{}",
            policy.metadata.namespace.as_deref().unwrap_or("default"),
            policy.metadata.name
        );
        self.policies.insert(key, policy);
    }

    pub fn add_exception(&mut self, exc: PolicyException) {
        self.exceptions.insert(exc.metadata.name.clone(), exc);
    }

    /// Evaluate all applicable policies against a resource for a given operation.
    pub fn evaluate(
        &self,
        resource: &serde_json::Value,
        namespace: Option<&str>,
        operation: &str,
        user_info: Option<&serde_json::Value>,
    ) -> PolicyEvalResult {
        let mut result = PolicyEvalResult::allow();
        let context = build_context(resource, namespace, operation, user_info);

        // Gather all applicable rules
        let applicable_policies: Vec<(&str, &PolicySpec)> = self
            .cluster_policies
            .values()
            .map(|p| (p.metadata.name.as_str(), &p.spec))
            .chain(
                self.policies
                    .values()
                    .filter(|p| p.metadata.namespace.as_deref() == namespace || namespace.is_none())
                    .map(|p| (p.metadata.name.as_str(), &p.spec)),
            )
            .collect();

        for (policy_name, spec) in applicable_policies {
            for rule in &spec.rules {
                if !self.rule_matches(rule, resource, namespace, operation) {
                    continue;
                }

                // Check if there's an exception for this resource/rule
                if self.has_exception(policy_name, &rule.name, resource, namespace, operation, &context)
                {
                    tracing::debug!(
                        target: "kyverno",
                        policy = policy_name,
                        rule = rule.name,
                        "policy exception applied — skipping"
                    );
                    continue;
                }

                match rule.rule_type() {
                    "validate" => match validate::validate_rule(rule, resource, &context) {
                        Ok(Some(mut violation)) => {
                            violation.policy = policy_name.to_string();
                            match spec.validation_failure_action {
                                ValidationFailureAction::Enforce => {
                                    result.allowed = false;
                                }
                                ValidationFailureAction::Audit => {
                                    result.warnings.push(format!(
                                        "audit: policy {} rule {} failed: {}",
                                        policy_name, rule.name, violation.message
                                    ));
                                }
                            }
                            result.violations.push(violation);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            result.warnings.push(format!(
                                "error evaluating policy {} rule {}: {e}",
                                policy_name, rule.name
                            ));
                            if spec.failure_policy == FailurePolicy::Fail {
                                result.allowed = false;
                            }
                        }
                    },
                    "mutate" => match mutate::mutate_rule(rule, resource, &context) {
                        Ok(Some((_new_resource, patches))) => {
                            result.mutations.extend(patches);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            result.warnings.push(format!(
                                "error mutating policy {} rule {}: {e}",
                                policy_name, rule.name
                            ));
                        }
                    },
                    "generate" => match generate::generate_rule(rule, resource, &context) {
                        Ok(mut generated) => {
                            for g in &mut generated {
                                g.policy = policy_name.to_string();
                            }
                            result.generated.extend(generated);
                        }
                        Err(e) => {
                            result.warnings.push(format!(
                                "error generating policy {} rule {}: {e}",
                                policy_name, rule.name
                            ));
                        }
                    },
                    "verifyImages" => {
                        match image_verify::verify_images_rule(rule, resource, &context) {
                            Ok(image_results) => {
                                for ir in &image_results {
                                    if !ir.verified {
                                        if spec.validation_failure_action
                                            == ValidationFailureAction::Enforce
                                        {
                                            result.allowed = false;
                                        }
                                        result.violations.push(PolicyViolation {
                                            policy: policy_name.to_string(),
                                            rule: rule.name.clone(),
                                            message: ir.error.clone().unwrap_or_else(|| {
                                                format!("image {} not verified", ir.image)
                                            }),
                                            severity: None,
                                            resource: None,
                                        });
                                    }
                                }
                                result.image_verification_results.extend(image_results);
                            }
                            Err(e) => {
                                result.warnings.push(format!(
                                    "error verifying images policy {} rule {}: {e}",
                                    policy_name, rule.name
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        result
    }

    /// Check whether a rule matches a given resource.
    fn rule_matches(
        &self,
        rule: &KyvernoRule,
        resource: &serde_json::Value,
        namespace: Option<&str>,
        operation: &str,
    ) -> bool {
        // Check match resources
        if !matches_resources(&rule.match_resources, resource, namespace, operation) {
            return false;
        }
        // Check exclude
        if let Some(exclude) = &rule.exclude {
            if matches_exclude(exclude, resource, namespace, operation) {
                return false;
            }
        }
        true
    }

    fn has_exception(
        &self,
        policy_name: &str,
        rule_name: &str,
        resource: &serde_json::Value,
        namespace: Option<&str>,
        operation: &str,
        context: &serde_json::Value,
    ) -> bool {
        let res_namespace = resource
            .get("metadata")
            .and_then(|m| m.get("namespace"))
            .and_then(|v| v.as_str())
            .or(namespace);
        self.exceptions.values().any(|exc| {
            exception::exception_applies(
                exc,
                policy_name,
                rule_name,
                resource,
                res_namespace,
                operation,
                context,
                matches_resources,
            )
        })
    }

    /// Generate a PolicyReport for a namespace.
    pub fn generate_report(&self, namespace: Option<&str>) -> PolicyReport {
        PolicyReport {
            api_version: "wgpolicyk8s.io/v1alpha2".into(),
            kind: "PolicyReport".into(),
            metadata: ObjectMeta {
                name: format!("polr-ns-{}", namespace.unwrap_or("cluster")),
                namespace: namespace.map(String::from),
                ..Default::default()
            },
            results: vec![],
            summary: PolicyReportSummary::default(),
        }
    }
}

impl Default for KyvernoEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn matches_resources(
    match_res: &MatchResources,
    resource: &serde_json::Value,
    namespace: Option<&str>,
    operation: &str,
) -> bool {
    // If any/all is empty, default to match all
    if match_res.any.is_empty() && match_res.all.is_empty() {
        // Use the shorthand `resources` field if present
        if let Some(res_desc) = &match_res.resources {
            return matches_resource_description(res_desc, resource, namespace, operation);
        }
        return true; // No restrictions = match all
    }

    let any_match = if match_res.any.is_empty() {
        true
    } else {
        match_res
            .any
            .iter()
            .any(|f| filter_matches(f, resource, namespace, operation))
    };

    let all_match = if match_res.all.is_empty() {
        true
    } else {
        match_res
            .all
            .iter()
            .all(|f| filter_matches(f, resource, namespace, operation))
    };

    any_match && all_match
}

pub(crate) fn matches_exclude(
    exclude: &ExcludeResources,
    resource: &serde_json::Value,
    namespace: Option<&str>,
    operation: &str,
) -> bool {
    if !exclude.any.is_empty() {
        if exclude
            .any
            .iter()
            .any(|f| filter_matches(f, resource, namespace, operation))
        {
            return true;
        }
    }
    if !exclude.all.is_empty() {
        if exclude
            .all
            .iter()
            .all(|f| filter_matches(f, resource, namespace, operation))
        {
            return true;
        }
    }
    if let Some(res_desc) = &exclude.resources {
        if matches_resource_description(res_desc, resource, namespace, operation) {
            return true;
        }
    }
    false
}

fn filter_matches(
    filter: &ResourceFilter,
    resource: &serde_json::Value,
    namespace: Option<&str>,
    operation: &str,
) -> bool {
    if let Some(res_desc) = &filter.resources {
        matches_resource_description(res_desc, resource, namespace, operation)
    } else {
        true
    }
}

fn matches_resource_description(
    desc: &ResourceDescription,
    resource: &serde_json::Value,
    namespace: Option<&str>,
    operation: &str,
) -> bool {
    let kind = resource.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let api_version = resource
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let name = resource
        .pointer("/metadata/name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Check kinds
    if !desc.kinds.is_empty() {
        let matches = desc.kinds.iter().any(|k| {
            if k.contains('/') {
                // apiVersion/Kind format
                let parts: Vec<&str> = k.splitn(2, '/').collect();
                parts
                    .get(0)
                    .map(|&av| av == api_version || av == "*")
                    .unwrap_or(false)
                    && parts
                        .get(1)
                        .map(|&ki| ki == kind || ki == "*")
                        .unwrap_or(false)
            } else {
                k == kind || k == "*"
            }
        });
        if !matches {
            return false;
        }
    }

    // Check namespaces
    if !desc.namespaces.is_empty() {
        let ns = namespace.unwrap_or("");
        let matches = desc
            .namespaces
            .iter()
            .any(|n| n == ns || jmespath::kyverno_pattern_match(n, ns));
        if !matches {
            return false;
        }
    }

    // Check names
    if !desc.names.is_empty() {
        let matches = desc
            .names
            .iter()
            .any(|n| n == name || jmespath::kyverno_pattern_match(n, name));
        if !matches {
            return false;
        }
    }

    // Check operations
    if !desc.operations.is_empty() {
        let matches = desc
            .operations
            .iter()
            .any(|op| op.eq_ignore_ascii_case(operation));
        if !matches {
            return false;
        }
    }

    // Check selector (label matching)
    if let Some(selector) = &desc.selector {
        if !label_selector_matches(selector, resource) {
            return false;
        }
    }

    true
}

fn label_selector_matches(selector: &LabelSelector, resource: &serde_json::Value) -> bool {
    let labels = resource
        .pointer("/metadata/labels")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    // matchLabels: all must match
    for (k, v) in &selector.match_labels {
        if labels.get(k).and_then(|lv| lv.as_str()) != Some(v.as_str()) {
            return false;
        }
    }

    // matchExpressions
    for expr in &selector.match_expressions {
        let has_key = labels.contains_key(&expr.key);
        let label_val = labels.get(&expr.key).and_then(|v| v.as_str()).unwrap_or("");
        let ok = match expr.operator.as_str() {
            "Exists" => has_key,
            "DoesNotExist" => !has_key,
            "In" => has_key && expr.values.iter().any(|v| v == label_val),
            "NotIn" => !has_key || !expr.values.iter().any(|v| v == label_val),
            _ => true,
        };
        if !ok {
            return false;
        }
    }

    true
}

fn build_context(
    resource: &serde_json::Value,
    namespace: Option<&str>,
    operation: &str,
    user_info: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "request": {
            "object": resource,
            "namespace": namespace,
            "operation": operation,
            "userInfo": user_info.cloned().unwrap_or(serde_json::json!({})),
        }
    })
}
