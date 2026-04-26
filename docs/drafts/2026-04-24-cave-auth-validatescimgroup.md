---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/protocol/scim/SCIMProviderFactory.java
upstream_fn: validateScimGroup
status: draft
tier: 1
created_at: 2026-04-24T16:28:04.131710+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/protocol/scim/SCIMProviderFactory.java` → `validateScimGroup`

## Failing test

```rust
#[tokio::test]
async fn test_validatescimgroup_validates_group_correctly() {
    use cave_auth::validatescimgroup;
    use serde_json::json;

    // Valid group with required fields
    let valid_group = json!({
        "id": "123e4567-e89b-12d3-a456-426614174000",
        "displayName": "Admins",
        "members": [
            {"value": "user1", "display": "Alice"},
            {"value": "user2", "display": "Bob"}
        ]
    });

    let result = validatescimgroup(&valid_group).await;
    assert!(result.is_ok(), "Valid group should pass validation");

    // Invalid group: missing displayName
    let invalid_group = json!({
        "id": "123e4567-e89b-12d3-a456-426614174000",
        "members": []
    });

    let result = validatescimgroup(&invalid_group).await;
    assert!(result.is_err(), "Group without displayName should fail");
    let err = result.unwrap_err();
    assert!(err.to_string().contains("displayName"), "Error should mention missing displayName");

    // Invalid group: members with invalid structure
    let invalid_members = json!({
        "id": "123e4567-e89b-12d3-a456-426614174000",
        "displayName": "Testers",
        "members": [
            {"value": "user1"}, // missing display
            {"display": "Charlie"} // missing value
        ]
    });

    let result = validatescimgroup(&invalid_members).await;
    assert!(result.is_err(), "Group with malformed members should fail");
}
```

## Implementation skeleton

```rust
use serde_json::Value;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ValidateScimGroupError {
    #[error("Group must have a non-empty 'displayName'")]
    MissingDisplayName,
    #[error("Group members must contain both 'value' and 'display' fields")]
    InvalidMemberStructure,
}

pub async fn validatescimgroup(group: &Value) -> Result<(), ValidateScimGroupError> {
    // Tier 2: Validate SCIM group structure per SCIM 2.0 spec and Keycloak expectations
    todo!("Tier 2")
}
```
