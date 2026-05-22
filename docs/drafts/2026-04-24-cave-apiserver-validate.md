---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/plugin.go
upstream_fn: Validate
status: draft
tier: 1
created_at: 2026-04-24T17:28:01.598985+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/admission/plugin/policy/validating/plugin.go` → `Validate`

## Failing test

```rust
#[tokio::test]
async fn validate_validates_admission_review_correctly() {
    use cave_apiserver::admission::{AdmissionReview, AdmissionRequest, AdmissionResponse, ValidatingAdmissionPolicy};
    use cave_apiserver::error::Error;
    use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1 as apiexts;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1 as meta;
    use std::collections::HashMap;

    // Create a mock policy with a simple validation rule
    let policy = ValidatingAdmissionPolicy {
        metadata: meta::ObjectMeta {
            name: Some("test-policy".to_string()),
            ..Default::default()
        },
        spec: apiexts::ValidatingAdmissionPolicySpec {
            param_kind: Some(apiexts::ParamKind {
                api_version: Some("example.com/v1".to_string()),
                kind: Some("TestParam".to_string()),
            }),
            validations: vec![
                apiexts::Validation {
                    expression: "object.metadata.name != null".to_string(),
                    message: "name is required".to_string(),
                    ..Default::default()
                }
            ],
            ..Default::default()
        },
    };

    // Create a request with a valid object (has name)
    let request = AdmissionRequest {
        uid: "test-uid".to_string(),
        kind: meta::GroupVersionKind {
            group: "apps".to_string(),
            version: "v1".to_string(),
            kind: "Deployment".to_string(),
        },
        resource: meta::GroupVersionResource {
            group: "apps".to_string(),
            version: "v1".to_string(),
            resource: "deployments".to_string(),
        },
        operation: "CREATE".to_string(),
        user_info: None,
        object: serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "my-deployment"
            }
        }),
        old_object: None,
        dry_run: Some(false),
        options: None,
    };

    // Validate should succeed
    let result = validate(&policy, &request).await;
    assert!(matches!(result, Ok(AdmissionResponse {
        allowed: true,
        ..
    })));

    // Now test with invalid object (missing name)
    let invalid_request = AdmissionRequest {
        object: serde_json::json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {}
        }),
        ..request.clone()
    };

    let result = validate(&policy, &invalid_request).await;
    assert!(matches!(result, Ok(AdmissionResponse {
        allowed: false,
        status: Some(ref status),
        ..
    }) if status.message == "name is required"));
}
```

## Implementation skeleton

```rust
pub async fn validate(
    _policy: &ValidatingAdmissionPolicy,
    _request: &AdmissionRequest,
) -> Result<AdmissionResponse, Error> {
    todo!("Tier 2")
}
```
