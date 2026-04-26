---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/generic/webhook.go
upstream_fn: Validate
status: draft
tier: 1
created_at: 2026-04-24T17:17:47.837897+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/generic/webhook.go` → `Validate`

## Failing test

```rust
#[tokio::test]
async fn validate_webhook_request_returns_error_for_invalid_webhook() {
    use cave_kernel::validate;
    use cave_kernel::webhook::{Webhook, WebhookMatchPolicy, WebhookNamespaceSelector, WebhookObjectSelector};
    use cave_kernel::admission::{AdmissionRequest, AdmissionReview};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
    use std::collections::HashMap;

    // Create a webhook with invalid configuration (nil match policy)
    let webhook = Webhook {
        name: "invalid-webhook".to_string(),
        client_config: None,
        rules: vec![],
        failure_policy: None,
        match_policy: None, // Invalid: must be Some
        namespace_selector: None,
        object_selector: None,
        reinvocation_policy: None,
        side_effects: None,
        timeout: None,
        admission_review_versions: vec!["v1".to_string()],
        match_conditions: vec![],
    };

    let request = AdmissionRequest {
        uid: "test-uid".to_string(),
        kind: k8s_openapi::apimachinery::pkg::apis::meta::v1::GroupVersionKind {
            group: "apps".to_string(),
            version: "v1".to_string(),
            kind: "Deployment".to_string(),
        },
        resource: k8s_openapi::apimachinery::pkg::apis::meta::v1::GroupVersionResource {
            group: "apps".to_string(),
            version: "v1".to_string(),
            resource: "deployments".to_string(),
        },
        sub_resource: None,
        operation: "CREATE".to_string(),
        user_info: None,
        object: None,
        old_object: None,
        dry_run: Some(false),
        options: None,
    };

    let review = AdmissionReview {
        api_version: "admission.k8s.io/v1".to_string(),
        kind: "AdmissionReview".to_string(),
        request: Some(request),
        response: None,
    };

    let result = validate(&webhook, &review).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("match_policy"));
}
```

## Implementation skeleton

```rust
pub async fn validate(webhook: &Webhook, review: &AdmissionReview) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2");
}
```
