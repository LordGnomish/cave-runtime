// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM ApiKey — `packages/twenty-server/src/engine/core-modules/api-key/api-key.entity.ts`
//!
//! Long-lived workspace-scoped credential. Twenty hashes the secret on
//! create and only returns the plaintext once. We keep the same wire
//! contract — the `secret_hash` field is the only persisted form.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiKey {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// SHA-256 hex of the issued token. Plain token is never stored.
    pub secret_hash: String,
    pub revoked: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl ApiKey {
    pub fn new(workspace_id: Uuid, name: impl Into<String>, secret_hash: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            secret_hash: secret_hash.into(),
            revoked: false,
            expires_at: None,
            last_used_at: None,
            created_at: Utc::now(),
        }
    }

    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if self.revoked {
            return false;
        }
        match self.expires_at {
            Some(exp) => exp > now,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn new_key_is_active() {
        let k = ApiKey::new(Uuid::nil(), "main", "deadbeef");
        assert!(k.is_active(Utc::now()));
        assert!(!k.revoked);
    }

    #[test]
    fn expired_key_is_inactive() {
        let mut k = ApiKey::new(Uuid::nil(), "main", "deadbeef");
        k.expires_at = Some(Utc::now() - Duration::hours(1));
        assert!(!k.is_active(Utc::now()));
    }

    #[test]
    fn revoked_key_is_inactive_even_if_not_expired() {
        let mut k = ApiKey::new(Uuid::nil(), "main", "deadbeef");
        k.revoked = true;
        assert!(!k.is_active(Utc::now()));
    }
}
