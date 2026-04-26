//! Permission models — port from @backstage/permission-common
//!
//! Upstream: backstage/plugins/permission-common/src/types.ts

use serde::{Deserialize, Serialize};

/// Upstream: PermissionAction in permission-common/src/types.ts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionAction {
    Create,
    Read,
    Update,
    Delete,
}

/// Upstream: PermissionAttributes in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAttributes {
    pub action: Option<PermissionAction>,
}

/// Upstream: Permission in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub name: String,
    pub attributes: PermissionAttributes,
}

/// Upstream: ResourcePermission in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePermission {
    pub name: String,
    pub attributes: PermissionAttributes,
    pub resource_type: String,
}

/// Upstream: AuthorizeResult enum (ALLOW | DENY) in permission-common/src/types.ts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthorizeResult {
    Allow,
    Deny,
}

/// Upstream: PolicyDecision in permission-common/src/types.ts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PolicyDecision {
    Allow,
    Deny,
}

/// Upstream: EvaluatePermissionRequest (single request in batch)
/// in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatePermissionRequest {
    /// UUID string, used to correlate response
    pub id: String,
    pub permission: Permission,
    /// Optional resource reference
    pub resource_ref: Option<String>,
}

/// Upstream: EvaluatePermissionResponse (single response in batch)
/// in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatePermissionResponse {
    pub id: String,
    pub result: AuthorizeResult,
}

/// Upstream: AuthorizeRequest (batch) in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    pub items: Vec<EvaluatePermissionRequest>,
}

/// Upstream: AuthorizeResponse (batch) in permission-common/src/types.ts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizeResponse {
    pub items: Vec<EvaluatePermissionResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_result_serializes_as_allow() {
        let result = AuthorizeResult::Allow;
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(json, "\"ALLOW\"");
    }

    #[test]
    fn authorize_result_serializes_as_deny() {
        let result = AuthorizeResult::Deny;
        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(json, "\"DENY\"");
    }

    #[test]
    fn evaluate_permission_request_roundtrip() {
        let req = EvaluatePermissionRequest {
            id: "test-id-123".to_string(),
            permission: Permission {
                name: "catalog.entity.read".to_string(),
                attributes: PermissionAttributes {
                    action: Some(PermissionAction::Read),
                },
            },
            resource_ref: Some("component:default/my-service".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: EvaluatePermissionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, req.id);
        assert_eq!(decoded.permission.name, req.permission.name);
        assert_eq!(decoded.resource_ref, req.resource_ref);
        assert_eq!(
            decoded.permission.attributes.action,
            req.permission.attributes.action
        );
    }

    #[test]
    fn authorize_request_deserialization() {
        let json = r#"{"items":[{"id":"abc","permission":{"name":"catalog.entity.read","attributes":{"action":"read"}},"resource_ref":null},{"id":"def","permission":{"name":"catalog.entity.delete","attributes":{"action":"delete"}},"resource_ref":"component:default/svc"}]}"#;
        let req: AuthorizeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.items.len(), 2);
        assert_eq!(req.items[0].id, "abc");
        assert_eq!(req.items[0].permission.name, "catalog.entity.read");
        assert_eq!(
            req.items[0].permission.attributes.action,
            Some(PermissionAction::Read)
        );
        assert_eq!(req.items[1].id, "def");
        assert_eq!(
            req.items[1].resource_ref,
            Some("component:default/svc".to_string())
        );
    }

    #[test]
    fn authorize_response_serialization() {
        let resp = AuthorizeResponse {
            items: vec![
                EvaluatePermissionResponse {
                    id: "abc".to_string(),
                    result: AuthorizeResult::Allow,
                },
                EvaluatePermissionResponse {
                    id: "def".to_string(),
                    result: AuthorizeResult::Deny,
                },
            ],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["items"][0]["id"], "abc");
        assert_eq!(json["items"][0]["result"], "ALLOW");
        assert_eq!(json["items"][1]["id"], "def");
        assert_eq!(json["items"][1]["result"], "DENY");
    }
}
