// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Permission policy trait and built-in implementations.
//!
//! Upstream: @backstage/permission-node — PermissionPolicy interface,
//! DefaultPermissionPolicy / AllowAllPermissionPolicy examples.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::{Permission, PolicyDecision};

/// Upstream: BackstagePrincipal — the authenticated caller identity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackstagePrincipal {
    pub user_entity_ref: Option<String>,
}

/// Upstream: PolicyQuery — the request handed to the policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyQuery {
    pub permission: Permission,
    pub principal: Option<BackstagePrincipal>,
}

/// Upstream: PermissionPolicy interface in @backstage/permission-node
#[async_trait]
pub trait PermissionPolicy: Send + Sync {
    async fn handle(
        &self,
        request: &PolicyQuery,
        user: Option<&BackstagePrincipal>,
    ) -> PolicyDecision;
}

/// Upstream: AllowAllPermissionPolicy — the "allow everything" policy used
/// as the default in Backstage examples and integration tests.
pub struct AllowAllPermissionPolicy;

#[async_trait]
impl PermissionPolicy for AllowAllPermissionPolicy {
    async fn handle(
        &self,
        _request: &PolicyQuery,
        _user: Option<&BackstagePrincipal>,
    ) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        catalog_entity_delete_permission, catalog_entity_read_permission,
    };

    #[tokio::test]
    async fn allow_all_policy_allows_any_permission() {
        let policy = AllowAllPermissionPolicy;
        let query = PolicyQuery {
            permission: catalog_entity_read_permission(),
            principal: None,
        };
        let decision = policy.handle(&query, None).await;
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[tokio::test]
    async fn allow_all_policy_allows_delete() {
        let policy = AllowAllPermissionPolicy;
        let query = PolicyQuery {
            permission: catalog_entity_delete_permission(),
            principal: None,
        };
        let decision = policy.handle(&query, None).await;
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[tokio::test]
    async fn allow_all_policy_allows_without_user() {
        let policy = AllowAllPermissionPolicy;
        let query = PolicyQuery {
            permission: catalog_entity_read_permission(),
            principal: None,
        };
        // Explicitly pass None principal — upstream guarantee
        let decision = policy.handle(&query, None).await;
        assert_eq!(decision, PolicyDecision::Allow);
    }
}
