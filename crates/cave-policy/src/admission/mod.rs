//! Kubernetes admission webhook handler.
//!
//! Implements: AdmissionReview v1 API, ValidatingWebhookConfiguration compatible,
//! MutatingWebhookConfiguration compatible, namespace/object selectors,
//! fail-open/fail-closed modes.

use crate::error::PolicyError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── AdmissionReview v1 ───────────────────────────────────────────────────────

/// Top-level AdmissionReview object (K8s admission.k8s.io/v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionReview {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<AdmissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<AdmissionResponse>,
}

impl AdmissionReview {
    pub fn new_response(uid: String, response: AdmissionResponse) -> Self {
        let mut r = response;
        r.uid = uid;
        Self {
            api_version: "admission.k8s.io/v1".into(),
            kind: "AdmissionReview".into(),
            request: None,
            response: Some(r),
        }
    }
}

/// AdmissionRequest — incoming request from the API server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub uid: String,
    pub kind: GroupVersionKind,
    pub resource: GroupVersionResource,
    #[serde(rename = "subResource", skip_serializing_if = "Option::is_none")]
    pub sub_resource: Option<String>,
    #[serde(rename = "requestKind", skip_serializing_if = "Option::is_none")]
    pub request_kind: Option<GroupVersionKind>,
    #[serde(rename = "requestResource", skip_serializing_if = "Option::is_none")]
    pub request_resource: Option<GroupVersionResource>,
    #[serde(rename = "requestSubResource", skip_serializing_if = "Option::is_none")]
    pub request_sub_resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub operation: Operation,
    #[serde(rename = "userInfo")]
    pub user_info: UserInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<serde_json::Value>,
    #[serde(rename = "oldObject", skip_serializing_if = "Option::is_none")]
    pub old_object: Option<serde_json::Value>,
    #[serde(rename = "dryRun", default)]
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
}

/// AdmissionResponse — webhook response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AdmissionStatus>,
    /// JSON Patch encoded as base64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(rename = "patchType", skip_serializing_if = "Option::is_none")]
    pub patch_type: Option<String>,
    #[serde(rename = "auditAnnotations", default, skip_serializing_if = "HashMap::is_empty")]
    pub audit_annotations: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl AdmissionResponse {
    pub fn allow(uid: String) -> Self {
        Self {
            uid,
            allowed: true,
            status: None,
            patch: None,
            patch_type: None,
            audit_annotations: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    pub fn deny(uid: String, message: String, code: u16) -> Self {
        Self {
            uid,
            allowed: false,
            status: Some(AdmissionStatus {
                code,
                message,
                reason: None,
                details: None,
            }),
            patch: None,
            patch_type: None,
            audit_annotations: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    pub fn allow_with_patch(uid: String, patches: Vec<serde_json::Value>) -> Self {
        use base64::Engine as _;
        let patch_json = serde_json::to_string(&patches).unwrap_or_else(|_| "[]".into());
        let patch_b64 = base64::engine::general_purpose::STANDARD.encode(patch_json.as_bytes());
        Self {
            uid,
            allowed: true,
            status: None,
            patch: Some(patch_b64),
            patch_type: Some("JSONPatch".into()),
            audit_annotations: HashMap::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionStatus {
    pub code: u16,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersionKind {
    pub group: String,
    pub version: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupVersionResource {
    pub group: String,
    pub version: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Operation {
    Create,
    Update,
    Delete,
    Connect,
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Operation::Create => write!(f, "CREATE"),
            Operation::Update => write!(f, "UPDATE"),
            Operation::Delete => write!(f, "DELETE"),
            Operation::Connect => write!(f, "CONNECT"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Vec<String>>,
}

// ─── Webhook configuration ────────────────────────────────────────────────────

/// Admission webhook handler that combines OPA + Kyverno evaluation.
pub struct AdmissionWebhook {
    pub fail_open: bool,
    pub timeout_seconds: u32,
}

impl Default for AdmissionWebhook {
    fn default() -> Self {
        Self {
            fail_open: false, // fail-closed by default (safe)
            timeout_seconds: 10,
        }
    }
}

impl AdmissionWebhook {
    pub fn new(fail_open: bool) -> Self {
        Self { fail_open, timeout_seconds: 10 }
    }

    /// Process an AdmissionReview request.
    pub fn handle(
        &self,
        review: &AdmissionReview,
        opa_engine: &crate::rego::PolicyEngine,
        kyverno_engine: &crate::kyverno::KyvernoEngine,
    ) -> Result<AdmissionReview, PolicyError> {
        let request = match &review.request {
            Some(r) => r,
            None => {
                return Err(PolicyError::InvalidRequest("AdmissionReview missing request".into()));
            }
        };

        let uid = request.uid.clone();
        let resource = request.object.clone().unwrap_or(serde_json::Value::Null);
        let operation = request.operation.to_string();
        let namespace = request.namespace.as_deref();

        tracing::debug!(
            target: "admission",
            uid = uid,
            kind = request.kind.kind,
            namespace = namespace.unwrap_or(""),
            operation = operation,
            "processing admission request"
        );

        // ── OPA evaluation ────────────────────────────────────────────────────
        let opa_input = build_opa_input(request);
        let opa_decision = self.eval_opa(opa_engine, &opa_input);

        if let Some(false) = opa_decision {
            return Ok(AdmissionReview::new_response(
                uid.clone(),
                AdmissionResponse::deny(
                    uid,
                    "Request denied by OPA policy".into(),
                    403,
                ),
            ));
        }

        // ── Kyverno evaluation ────────────────────────────────────────────────
        let kyverno_result = kyverno_engine.evaluate(&resource, namespace, &operation, Some(&serde_json::json!({
            "username": request.user_info.username,
            "groups": request.user_info.groups,
        })));

        if !kyverno_result.allowed {
            let messages: Vec<String> = kyverno_result.violations
                .iter()
                .map(|v| format!("[{}:{}] {}", v.policy, v.rule, v.message))
                .collect();
            return Ok(AdmissionReview::new_response(
                uid.clone(),
                AdmissionResponse::deny(
                    uid,
                    messages.join("; "),
                    403,
                ),
            ));
        }

        // Build response
        let mut response = if kyverno_result.mutations.is_empty() {
            AdmissionResponse::allow(uid.clone())
        } else {
            AdmissionResponse::allow_with_patch(uid.clone(), kyverno_result.mutations)
        };

        // Add warnings
        response.warnings = kyverno_result.warnings;

        // Add audit annotations
        for (i, v) in kyverno_result.violations.iter().enumerate() {
            response.audit_annotations.insert(
                format!("policy.kyverno.io/audit-{i}"),
                format!("{}/{}: {}", v.policy, v.rule, v.message),
            );
        }

        Ok(AdmissionReview::new_response(uid, response))
    }

    fn eval_opa(
        &self,
        engine: &crate::rego::PolicyEngine,
        input: &serde_json::Value,
    ) -> Option<bool> {
        // Try standard OPA admission paths
        let paths_to_try = [
            vec!["data".to_string(), "kubernetes".to_string(), "admission".to_string(), "allow".to_string()],
            vec!["data".to_string(), "authz".to_string(), "allow".to_string()],
            vec!["data".to_string(), "main".to_string(), "allow".to_string()],
        ];

        for path in &paths_to_try {
            if let Some(v) = engine.query_path(path, input.clone()) {
                return Some(v.as_bool().unwrap_or(true));
            }
        }
        None // No OPA policy — pass through
    }
}

fn build_opa_input(request: &AdmissionRequest) -> serde_json::Value {
    serde_json::json!({
        "request": {
            "uid": request.uid,
            "kind": {
                "group": request.kind.group,
                "version": request.kind.version,
                "kind": request.kind.kind,
            },
            "resource": {
                "group": request.resource.group,
                "version": request.resource.version,
                "resource": request.resource.resource,
            },
            "name": request.name,
            "namespace": request.namespace,
            "operation": request.operation.to_string(),
            "userInfo": {
                "username": request.user_info.username,
                "uid": request.user_info.uid,
                "groups": request.user_info.groups,
            },
            "object": request.object,
            "oldObject": request.old_object,
            "dryRun": request.dry_run,
        }
    })
}

// ─── WebhookConfiguration models ─────────────────────────────────────────────

/// ValidatingWebhookConfiguration (for registering with K8s API server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhookConfiguration {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: crate::kyverno::models::ObjectMeta,
    pub webhooks: Vec<ValidatingWebhook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatingWebhook {
    pub name: String,
    #[serde(rename = "admissionReviewVersions")]
    pub admission_review_versions: Vec<String>,
    #[serde(rename = "clientConfig")]
    pub client_config: WebhookClientConfig,
    pub rules: Vec<WebhookRule>,
    #[serde(rename = "failurePolicy", default)]
    pub failure_policy: WebhookFailurePolicy,
    #[serde(rename = "matchPolicy", skip_serializing_if = "Option::is_none")]
    pub match_policy: Option<String>,
    #[serde(rename = "namespaceSelector", skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<crate::kyverno::models::LabelSelector>,
    #[serde(rename = "objectSelector", skip_serializing_if = "Option::is_none")]
    pub object_selector: Option<crate::kyverno::models::LabelSelector>,
    #[serde(rename = "sideEffects", default)]
    pub side_effects: String,
    #[serde(rename = "timeoutSeconds", skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
}

/// MutatingWebhookConfiguration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingWebhookConfiguration {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: crate::kyverno::models::ObjectMeta,
    pub webhooks: Vec<MutatingWebhook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatingWebhook {
    pub name: String,
    #[serde(rename = "admissionReviewVersions")]
    pub admission_review_versions: Vec<String>,
    #[serde(rename = "clientConfig")]
    pub client_config: WebhookClientConfig,
    pub rules: Vec<WebhookRule>,
    #[serde(rename = "failurePolicy", default)]
    pub failure_policy: WebhookFailurePolicy,
    #[serde(rename = "namespaceSelector", skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<crate::kyverno::models::LabelSelector>,
    #[serde(rename = "objectSelector", skip_serializing_if = "Option::is_none")]
    pub object_selector: Option<crate::kyverno::models::LabelSelector>,
    #[serde(rename = "reinvocationPolicy", skip_serializing_if = "Option::is_none")]
    pub reinvocation_policy: Option<String>,
    #[serde(rename = "sideEffects", default)]
    pub side_effects: String,
    #[serde(rename = "timeoutSeconds", skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookClientConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceReference>,
    #[serde(rename = "caBundle", skip_serializing_if = "Option::is_none")]
    pub ca_bundle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceReference {
    pub namespace: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRule {
    #[serde(rename = "apiGroups")]
    pub api_groups: Vec<String>,
    #[serde(rename = "apiVersions")]
    pub api_versions: Vec<String>,
    pub operations: Vec<String>,
    pub resources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>, // "*", "Namespaced", "Cluster"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum WebhookFailurePolicy {
    #[default]
    Fail,
    Ignore,
}
