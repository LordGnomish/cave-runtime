//! Tag immutability policies and repository-level access control.

use crate::store::RegistryStore;
use crate::types::{AccessRule, Permission, TagPolicy};
use std::sync::Arc;

pub struct PolicyManager {
    store: Arc<RegistryStore>,
}

impl PolicyManager {
    pub fn new(store: Arc<RegistryStore>) -> Self {
        Self { store }
    }

    // ── Tag immutability ──────────────────────────────────────────────────────

    pub async fn set_tag_policy(&self, repo: &str, policy: TagPolicy) {
        self.store.set_tag_policy(repo, policy).await;
    }

    /// Returns true if `tag` in `repo` cannot be overwritten.
    pub async fn is_tag_immutable(&self, repo: &str, tag: &str) -> bool {
        // Never block digest-addressed references.
        if tag.starts_with("sha256:") {
            return false;
        }
        self.store.is_tag_immutable(repo, tag).await
    }

    // ── Access control ────────────────────────────────────────────────────────

    pub async fn set_access_rules(&self, repo: &str, rules: Vec<AccessRule>) {
        self.store.set_access_rules(repo, rules).await;
    }

    pub async fn check_permission(
        &self,
        repo: &str,
        subject: &str,
        perm: &Permission,
    ) -> bool {
        self.store.check_permission(repo, subject, perm).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_immutable_tag_blocks_push() {
        let store = Arc::new(RegistryStore::new());
        let pm = PolicyManager::new(Arc::clone(&store));

        pm.set_tag_policy(
            "prod",
            TagPolicy {
                immutable_tags: vec!["v1.0.0".to_string()],
                all_immutable: false,
            },
        )
        .await;

        assert!(pm.is_tag_immutable("prod", "v1.0.0").await);
        assert!(!pm.is_tag_immutable("prod", "latest").await);
    }

    #[tokio::test]
    async fn test_all_immutable_policy() {
        let store = Arc::new(RegistryStore::new());
        let pm = PolicyManager::new(Arc::clone(&store));

        pm.set_tag_policy("locked", TagPolicy { immutable_tags: vec![], all_immutable: true })
            .await;

        assert!(pm.is_tag_immutable("locked", "latest").await);
        assert!(pm.is_tag_immutable("locked", "v2.0.0").await);
        // Digests are never immutable via policy.
        assert!(!pm.is_tag_immutable("locked", "sha256:abcdef").await);
    }

    #[tokio::test]
    async fn test_access_control_pull_permission() {
        let store = Arc::new(RegistryStore::new());
        let pm = PolicyManager::new(Arc::clone(&store));

        pm.set_access_rules(
            "private",
            vec![AccessRule { subject: "alice".to_string(), permission: Permission::Pull }],
        )
        .await;

        assert!(pm.check_permission("private", "alice", &Permission::Pull).await);
        assert!(!pm.check_permission("private", "bob", &Permission::Pull).await);
    }

    #[tokio::test]
    async fn test_push_implies_pull() {
        let store = Arc::new(RegistryStore::new());
        let pm = PolicyManager::new(Arc::clone(&store));

        pm.set_access_rules(
            "shared",
            vec![AccessRule { subject: "ci".to_string(), permission: Permission::Push }],
        )
        .await;

        assert!(pm.check_permission("shared", "ci", &Permission::Push).await);
        assert!(pm.check_permission("shared", "ci", &Permission::Pull).await);
    }

    #[tokio::test]
    async fn test_no_rules_open_access() {
        let store = Arc::new(RegistryStore::new());
        let pm = PolicyManager::new(Arc::clone(&store));
        // No rules configured — any subject should be allowed.
        assert!(pm.check_permission("open", "anyone", &Permission::Pull).await);
    }
}
