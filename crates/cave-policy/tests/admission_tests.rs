//! Admission webhook integration tests.
//!
//! Tests AdmissionReview v1 API, OPA + Kyverno integration,
//! fail-open/fail-closed, patch encoding, GVK/GVR types.

use cave_policy::admission::{
    AdmissionRequest, AdmissionResponse, AdmissionReview,
    AdmissionWebhook, GroupVersionKind, GroupVersionResource, Operation, UserInfo,
};
use cave_policy::kyverno::KyvernoEngine;
use cave_policy::rego::PolicyEngine;
use serde_json::{json, Value};

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn make_review(uid: &str, kind: &str, namespace: &str, name: &str, object: Value) -> AdmissionReview {
    AdmissionReview {
        api_version: "admission.k8s.io/v1".to_string(),
        kind: "AdmissionReview".to_string(),
        request: Some(AdmissionRequest {
            uid: uid.to_string(),
            kind: GroupVersionKind {
                group: "".to_string(),
                version: "v1".to_string(),
                kind: kind.to_string(),
            },
            resource: GroupVersionResource {
                group: "".to_string(),
                version: "v1".to_string(),
                resource: format!("{}s", kind.to_lowercase()),
            },
            sub_resource: None,
            request_kind: None,
            request_resource: None,
            request_sub_resource: None,
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            operation: Operation::Create,
            user_info: UserInfo {
                username: "test-user".to_string(),
                uid: Some("user-uid".to_string()),
                groups: vec!["system:authenticated".to_string()],
                extra: Default::default(),
            },
            object: Some(object),
            old_object: None,
            dry_run: false,
            options: None,
        }),
        response: None,
    }
}

fn pod_object(name: &str, namespace: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {"app": "web"}
        },
        "spec": {
            "containers": [{"name": "main", "image": "nginx:1.25"}]
        }
    })
}

fn handle(webhook: &AdmissionWebhook, review: &AdmissionReview) -> AdmissionResponse {
    let opa = PolicyEngine::new();
    let kyverno = KyvernoEngine::new();
    webhook.handle(review, &opa, &kyverno)
        .unwrap()
        .response
        .unwrap()
}

// ─── AdmissionResponse Tests ──────────────────────────────────────────────────

#[test]
fn test_response_allow() {
    let resp = AdmissionResponse::allow("test-uid-001".to_string());
    assert!(resp.allowed);
    assert_eq!(resp.uid, "test-uid-001");
    assert!(resp.status.is_none());
    assert!(resp.patch.is_none());
}

#[test]
fn test_response_deny() {
    let resp = AdmissionResponse::deny(
        "test-uid-002".to_string(),
        "image tag 'latest' is not allowed".to_string(),
        403,
    );
    assert!(!resp.allowed);
    assert_eq!(resp.uid, "test-uid-002");
    assert!(resp.status.is_some());
    let status = resp.status.unwrap();
    assert!(status.message.contains("latest"));
    assert_eq!(status.code, 403);
}

#[test]
fn test_response_allow_with_patch() {
    let patches = vec![json!({
        "op": "add",
        "path": "/metadata/labels/managed-by",
        "value": "admission-webhook"
    })];
    let resp = AdmissionResponse::allow_with_patch("test-uid-003".to_string(), patches);
    assert!(resp.allowed);
    assert!(resp.patch.is_some());
    assert_eq!(resp.patch_type, Some("JSONPatch".to_string()));

    // Verify patch is valid base64-encoded JSON
    let patch_b64 = resp.patch.unwrap();
    let decoded = base64_decode(&patch_b64);
    let decoded_json: Value = serde_json::from_slice(&decoded).unwrap();
    assert!(decoded_json.is_array());
    assert_eq!(decoded_json[0]["op"], json!("add"));
}

fn base64_decode(s: &str) -> Vec<u8> {
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = Vec::new();
    let chars: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    for chunk in chars.chunks(4) {
        let decode = |c: u8| -> u32 { alphabet.find(c as char).unwrap_or(0) as u32 };
        let b0 = decode(chunk[0]);
        let b1 = decode(chunk.get(1).copied().unwrap_or(b'A'));
        let b2 = decode(chunk.get(2).copied().unwrap_or(b'A'));
        let b3 = decode(chunk.get(3).copied().unwrap_or(b'A'));
        result.push(((b0 << 2) | (b1 >> 4)) as u8);
        if chunk.len() > 2 { result.push(((b1 << 4) | (b2 >> 2)) as u8); }
        if chunk.len() > 3 { result.push(((b2 << 6) | b3) as u8); }
    }
    result
}

// ─── Webhook Handle Tests ─────────────────────────────────────────────────────

#[test]
fn test_webhook_no_policies_defaults_allow() {
    let webhook = AdmissionWebhook::default();
    let review = make_review("uid-1", "Pod", "default", "my-pod", pod_object("my-pod", "default"));
    let resp = handle(&webhook, &review);
    assert!(resp.allowed, "no policies should default to allow");
    assert_eq!(resp.uid, "uid-1");
}

#[test]
fn test_webhook_opa_allow_policy() {
    let webhook = AdmissionWebhook::default();
    let mut opa = PolicyEngine::new();
    let kyverno = KyvernoEngine::new();

    opa.load_module(
        "admission/allow",
        "package kubernetes.admission\ndefault allow = true\n",
    ).unwrap();

    let review = make_review("uid-2", "Pod", "default", "test-pod", pod_object("test-pod", "default"));
    let resp = webhook.handle(&review, &opa, &kyverno).unwrap().response.unwrap();
    assert!(resp.allowed);
}

#[test]
fn test_webhook_opa_deny_policy() {
    let webhook = AdmissionWebhook::default();
    let mut opa = PolicyEngine::new();
    let kyverno = KyvernoEngine::new();

    opa.load_module(
        "admission/deny_pods",
        r#"
package kubernetes.admission

default allow = false

deny[msg] {
    input.request.kind.kind == "Pod"
    msg := "Pods are not allowed"
}
"#,
    ).unwrap();

    let review = make_review("uid-3", "Pod", "default", "blocked-pod", pod_object("blocked-pod", "default"));
    let resp = webhook.handle(&review, &opa, &kyverno).unwrap().response.unwrap();
    assert!(!resp.allowed, "OPA deny should block");
}

#[test]
fn test_webhook_uid_propagated() {
    let webhook = AdmissionWebhook::default();
    let uid = "unique-uid-abc-123";
    let review = make_review(uid, "Pod", "default", "p", pod_object("p", "default"));
    let resp = handle(&webhook, &review);
    assert_eq!(resp.uid, uid, "UID must be propagated to response");
}

#[test]
fn test_webhook_missing_request_returns_error() {
    let webhook = AdmissionWebhook::default();
    let opa = PolicyEngine::new();
    let kyverno = KyvernoEngine::new();

    let review = AdmissionReview {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        request: None,
        response: None,
    };

    let result = webhook.handle(&review, &opa, &kyverno);
    assert!(result.is_err(), "missing request should return error");
}

// ─── Operation Type Tests ─────────────────────────────────────────────────────

#[test]
fn test_operation_display() {
    assert_eq!(Operation::Create.to_string(), "CREATE");
    assert_eq!(Operation::Update.to_string(), "UPDATE");
    assert_eq!(Operation::Delete.to_string(), "DELETE");
    assert_eq!(Operation::Connect.to_string(), "CONNECT");
}

#[test]
fn test_admission_review_structure() {
    let review = make_review("test-uid", "Deployment", "production", "my-deploy", json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": "my-deploy", "namespace": "production"},
        "spec": {"replicas": 3}
    }));

    assert!(review.request.is_some());
    let req = review.request.as_ref().unwrap();
    assert_eq!(req.uid, "test-uid");
    assert_eq!(req.kind.kind, "Deployment");
    assert_eq!(req.namespace.as_deref(), Some("production"));
    assert_eq!(req.name.as_deref(), Some("my-deploy"));
    assert!(matches!(req.operation, Operation::Create));
}

// ─── Dry Run Flag Test ────────────────────────────────────────────────────────

#[test]
fn test_dry_run_flag() {
    let review = make_review("uid-dr", "Pod", "default", "dry-pod", pod_object("dry-pod", "default"));
    let req = review.request.as_ref().unwrap();
    assert!(!req.dry_run);

    let mut dry = review.request.unwrap();
    dry.dry_run = true;
    assert!(dry.dry_run);
}

// ─── AdmissionReview JSON Serialization Tests ─────────────────────────────────

#[test]
fn test_review_serialization_roundtrip() {
    let review = make_review(
        "roundtrip-uid",
        "ConfigMap",
        "kube-system",
        "config",
        json!({"apiVersion": "v1", "kind": "ConfigMap", "data": {"key": "value"}}),
    );

    let serialized = serde_json::to_string(&review).unwrap();
    let deserialized: AdmissionReview = serde_json::from_str(&serialized).unwrap();

    assert!(deserialized.request.is_some());
    let req = deserialized.request.unwrap();
    assert_eq!(req.uid, "roundtrip-uid");
    assert_eq!(req.kind.kind, "ConfigMap");
    assert_eq!(req.namespace.as_deref(), Some("kube-system"));
}

#[test]
fn test_response_serialization() {
    let resp = AdmissionResponse::deny(
        "denied-uid".to_string(),
        "policy violation: no team label".to_string(),
        403,
    );

    let serialized = serde_json::to_value(&resp).unwrap();
    assert_eq!(serialized["allowed"], json!(false));
    assert_eq!(serialized["uid"], json!("denied-uid"));
    assert!(serialized["status"]["message"].as_str().unwrap().contains("team label"));
}

// ─── GVK / GVR Tests ─────────────────────────────────────────────────────────

#[test]
fn test_gvk_core_group() {
    let gvk = GroupVersionKind {
        group: "".to_string(),
        version: "v1".to_string(),
        kind: "Pod".to_string(),
    };
    assert_eq!(gvk.kind, "Pod");
    assert!(gvk.group.is_empty());
}

#[test]
fn test_gvk_named_group() {
    let gvk = GroupVersionKind {
        group: "apps".to_string(),
        version: "v1".to_string(),
        kind: "Deployment".to_string(),
    };
    assert_eq!(gvk.group, "apps");
    assert_eq!(gvk.kind, "Deployment");
}

#[test]
fn test_gvr_resource_name() {
    let gvr = GroupVersionResource {
        group: "".to_string(),
        version: "v1".to_string(),
        resource: "pods".to_string(),
    };
    assert_eq!(gvr.resource, "pods");
}

// ─── UserInfo Tests ───────────────────────────────────────────────────────────

#[test]
fn test_user_info_groups() {
    let user = UserInfo {
        username: "admin".to_string(),
        uid: Some("user-123".to_string()),
        groups: vec!["system:masters".to_string(), "system:authenticated".to_string()],
        extra: Default::default(),
    };

    assert_eq!(user.username, "admin");
    assert!(user.groups.contains(&"system:masters".to_string()));
}

// ─── Integration: OPA + Kyverno Combined ─────────────────────────────────────

#[test]
fn test_opa_allow_kyverno_deny() {
    use cave_policy::kyverno::models::{
        ClusterPolicy, KyvernoRule, MatchResources, ObjectMeta, PolicySpec, ResourceDescription,
        ValidationFailureAction, Validation,
    };

    let webhook = AdmissionWebhook::default();
    let mut opa = PolicyEngine::new();
    let mut kyverno = KyvernoEngine::new();

    // OPA says allow=true
    opa.load_module(
        "admission/allow",
        "package kubernetes.admission\ndefault allow = true\n",
    ).unwrap();

    // Kyverno denies pods without a specific label
    kyverno.add_cluster_policy(ClusterPolicy {
        api_version: "kyverno.io/v1".into(),
        kind: "ClusterPolicy".into(),
        metadata: ObjectMeta {
            name: "require-security-label".to_string(),
            ..Default::default()
        },
        spec: PolicySpec {
            validation_failure_action: ValidationFailureAction::Enforce,
            rules: vec![KyvernoRule {
                name: "check-security".to_string(),
                match_resources: MatchResources {
                    resources: Some(ResourceDescription {
                        kinds: vec!["Pod".to_string()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                exclude: None,
                context: vec![],
                preconditions: None,
                validate: Some(Validation {
                    message: Some("Pod must have security=restricted label".to_string()),
                    pattern: Some(json!({"metadata": {"labels": {"security": "?*"}}})),
                    ..Default::default()
                }),
                mutate: None,
                generate: None,
                verify_images: vec![],
            }],
            ..Default::default()
        },
        status: None,
    });

    // Pod without security label — OPA allows but Kyverno denies
    let review = make_review(
        "combined-uid",
        "Pod",
        "default",
        "insecure-pod",
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": "insecure-pod", "namespace": "default", "labels": {"app": "web"}},
            "spec": {"containers": [{"name": "c", "image": "nginx:1.25"}]}
        }),
    );

    let resp = webhook.handle(&review, &opa, &kyverno).unwrap().response.unwrap();
    assert!(!resp.allowed, "Kyverno deny should block even when OPA allows");
}
