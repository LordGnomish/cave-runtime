//! Conversation resolution for `cavectl chat --conversation new|last|<id>`.
//!
//! `new`  — always create a fresh conversation.
//! `last` — reuse the most recently active conversation for the tenant; create
//!          one if none exists.
//! `<id>` — load by exact id; error if missing.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use super::output::ChatMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationKind {
    New,
    Last,
    ById,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub conversation_id: String,
    pub tenant_id: String,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub messages: Vec<ChatMessage>,
}

impl Conversation {
    pub fn new(tenant_id: &str) -> Self {
        let now = Utc::now();
        Self {
            conversation_id: Uuid::new_v4().to_string(),
            tenant_id: tenant_id.to_string(),
            created_at: now,
            last_active_at: now,
            messages: vec![],
        }
    }
}

/// Parse the `--conversation` flag value into a tagged kind.
/// Returns `(kind, optional_id)`.
pub fn parse_selector(raw: &str) -> Result<(ConversationKind, Option<String>)> {
    match raw.trim() {
        "" => Err(anyhow!("conversation selector cannot be empty")),
        "new" => Ok((ConversationKind::New, None)),
        "last" => Ok((ConversationKind::Last, None)),
        other => Ok((ConversationKind::ById, Some(other.to_string()))),
    }
}

#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn create(&self, tenant_id: &str) -> Result<Conversation>;
    async fn last(&self, tenant_id: &str) -> Result<Option<Conversation>>;
    async fn get(&self, tenant_id: &str, conversation_id: &str) -> Result<Conversation>;
    async fn append(
        &self,
        tenant_id: &str,
        conversation_id: &str,
        msg: ChatMessage,
    ) -> Result<Conversation>;
    /// Resolve a `--conversation` selector into a concrete conversation, creating
    /// when needed. `last` with no prior conversation falls through to `new`.
    async fn resolve(&self, tenant_id: &str, selector: &str) -> Result<Conversation> {
        let (kind, id) = parse_selector(selector)?;
        match (kind, id) {
            (ConversationKind::New, _) => self.create(tenant_id).await,
            (ConversationKind::Last, _) => match self.last(tenant_id).await? {
                Some(c) => Ok(c),
                None => self.create(tenant_id).await,
            },
            (ConversationKind::ById, Some(id)) => self.get(tenant_id, &id).await,
            (ConversationKind::ById, None) => Err(anyhow!("missing conversation id")),
        }
    }
}

#[derive(Default)]
pub struct InMemoryConversationStore {
    inner: Arc<RwLock<HashMap<(String, String), Conversation>>>,
}

impl InMemoryConversationStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ConversationStore for InMemoryConversationStore {
    async fn create(&self, tenant_id: &str) -> Result<Conversation> {
        let c = Conversation::new(tenant_id);
        self.inner
            .write()
            .insert((tenant_id.to_string(), c.conversation_id.clone()), c.clone());
        Ok(c)
    }

    async fn last(&self, tenant_id: &str) -> Result<Option<Conversation>> {
        let s = self.inner.read();
        Ok(s.values()
            .filter(|c| c.tenant_id == tenant_id)
            .max_by_key(|c| c.last_active_at)
            .cloned())
    }

    async fn get(&self, tenant_id: &str, conversation_id: &str) -> Result<Conversation> {
        self.inner
            .read()
            .get(&(tenant_id.to_string(), conversation_id.to_string()))
            .cloned()
            .ok_or_else(|| {
                anyhow!("conversation not found: {tenant_id}/{conversation_id}")
            })
    }

    async fn append(
        &self,
        tenant_id: &str,
        conversation_id: &str,
        msg: ChatMessage,
    ) -> Result<Conversation> {
        let mut s = self.inner.write();
        let c = s
            .get_mut(&(tenant_id.to_string(), conversation_id.to_string()))
            .ok_or_else(|| {
                anyhow!("conversation not found: {tenant_id}/{conversation_id}")
            })?;
        c.messages.push(msg);
        c.last_active_at = Utc::now();
        Ok(c.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// cite: conversation selector — `new` parses to New variant
    #[test]
    fn conversation_acme_selector_new_parses() {
        let _tenant_id = "acme";
        let (kind, id) = parse_selector("new").unwrap();
        assert_eq!(kind, ConversationKind::New);
        assert!(id.is_none());
    }

    /// cite: conversation selector — `last` parses to Last variant
    #[test]
    fn conversation_globex_selector_last_parses() {
        let _tenant_id = "globex";
        let (kind, id) = parse_selector("last").unwrap();
        assert_eq!(kind, ConversationKind::Last);
        assert!(id.is_none());
    }

    /// cite: conversation selector — arbitrary string treated as id
    #[test]
    fn conversation_initech_selector_id_parses() {
        let _tenant_id = "initech";
        let (kind, id) = parse_selector("abc-123").unwrap();
        assert_eq!(kind, ConversationKind::ById);
        assert_eq!(id.as_deref(), Some("abc-123"));
    }

    /// cite: conversation selector — empty rejected
    #[test]
    fn conversation_acme_selector_empty_rejected() {
        let _tenant_id = "acme";
        let err = parse_selector("   ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    /// cite: conversation resolve — `new` always creates fresh id
    #[tokio::test]
    async fn conversation_acme_resolve_new_creates_fresh() {
        let tenant_id = "acme";
        let s = InMemoryConversationStore::new();
        let a = s.resolve(tenant_id, "new").await.unwrap();
        let b = s.resolve(tenant_id, "new").await.unwrap();
        assert_ne!(a.conversation_id, b.conversation_id);
    }

    /// cite: conversation resolve — `last` returns most-recent on second call
    #[tokio::test]
    async fn conversation_acme_resolve_last_returns_most_recent() {
        let tenant_id = "acme";
        let s = InMemoryConversationStore::new();
        let first = s.resolve(tenant_id, "new").await.unwrap();
        // touch first
        s.append(
            tenant_id,
            &first.conversation_id,
            ChatMessage::user(tenant_id, "ping"),
        )
        .await
        .unwrap();
        let last = s.resolve(tenant_id, "last").await.unwrap();
        assert_eq!(last.conversation_id, first.conversation_id);
    }

    /// cite: conversation resolve — `last` with empty store falls back to create
    #[tokio::test]
    async fn conversation_globex_resolve_last_empty_creates() {
        let tenant_id = "globex";
        let s = InMemoryConversationStore::new();
        let c = s.resolve(tenant_id, "last").await.unwrap();
        assert!(!c.conversation_id.is_empty());
    }

    /// cite: conversation resolve — by id loads existing
    #[tokio::test]
    async fn conversation_initech_resolve_by_id_loads_existing() {
        let tenant_id = "initech";
        let s = InMemoryConversationStore::new();
        let created = s.create(tenant_id).await.unwrap();
        let loaded = s.resolve(tenant_id, &created.conversation_id).await.unwrap();
        assert_eq!(loaded.conversation_id, created.conversation_id);
    }

    /// cite: conversation resolve — unknown id errors
    #[tokio::test]
    async fn conversation_acme_resolve_unknown_id_errors() {
        let tenant_id = "acme";
        let s = InMemoryConversationStore::new();
        let err = s.resolve(tenant_id, "ghost-id").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    /// cite: conversation append — last_active_at advances
    #[tokio::test]
    async fn conversation_acme_append_advances_last_active() {
        let tenant_id = "acme";
        let s = InMemoryConversationStore::new();
        let c = s.create(tenant_id).await.unwrap();
        let original = c.last_active_at;
        // ensure clock moves forward at least one tick
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let after = s
            .append(
                tenant_id,
                &c.conversation_id,
                ChatMessage::user(tenant_id, "x"),
            )
            .await
            .unwrap();
        assert!(after.last_active_at > original);
        assert_eq!(after.messages.len(), 1);
    }

    /// cite: conversation last — scoped to tenant
    #[tokio::test]
    async fn conversation_last_acme_excludes_globex() {
        let s = InMemoryConversationStore::new();
        s.create("acme").await.unwrap();
        let g = s.create("globex").await.unwrap();
        let acme_last = s.last("acme").await.unwrap().unwrap();
        assert_ne!(acme_last.conversation_id, g.conversation_id);
        assert_eq!(acme_last.tenant_id, "acme");
    }
}
