---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/services/managers/RealmManager.java
upstream_fn: checkPermission
status: draft
tier: 1
created_at: 2026-04-24T17:17:19.104242+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/services/managers/RealmManager.java` → `checkPermission`

## Failing test

```rust
#[tokio::test]
async fn test_checkpermission_grants_access_when_user_has_required_role() {
    use cave_auth::{checkpermission, Permission, PermissionResult, User, Role};
    use std::collections::HashSet;

    // Setup: user with role "admin" in realm "acme"
    let user = User {
        id: "user-123".to_string(),
        username: "alice".to_string(),
        realm: "acme".to_string(),
        roles: HashSet::from([Role::new("admin", "acme")]),
    };

    // Permission request: user needs "admin" role in "acme" realm
    let permission = Permission {
        resource: "realm".to_string(),
        action: "manage".to_string(),
        realm: "acme".to_string(),
        required_roles: HashSet::from(["admin".to_string()]),
    };

    let result = checkpermission(&user, &permission).await;

    assert!(matches!(result, PermissionResult::Granted));
}

## Implementation
```

## Implementation skeleton

```rust
pub async fn checkpermission(user: &User, permission: &Permission) -> PermissionResult {
    todo!("Tier 2")
}
```
