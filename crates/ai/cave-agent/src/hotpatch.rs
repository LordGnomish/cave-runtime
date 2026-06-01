// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-improvement, step 4: hot-patch ingestion. A [`PatchRegistry`] holds the
//! live config map and accepts [`Patch`]es that carry a SHA-256 of their
//! payload. Patches are *staged* (checksum-verified), then *applied* (which
//! snapshots the previous value so it can be *rolled back*). Every transition
//! is appended to an audit trail.
//!
//! OpenJarvis upstream: `jarvis/improve/hotpatch.py`. Upstream also fetches and
//! signature-verifies remote patch bundles; the network/keyring path is
//! scope-cut to cave-vault / cave-sign. This is the in-process apply engine.

use crate::error::{AgentError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Canonical SHA-256 (hex) of a JSON value. `serde_json`'s default object map
/// is key-sorted, so the digest is independent of literal key order.
pub fn sha256_of(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    hex::encode(digest)
}

/// A proposed change to one config key, carrying a checksum of its payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    /// Unique patch id.
    pub id: String,
    /// The config key it targets.
    pub target_key: String,
    /// The new value.
    pub value: Value,
    /// SHA-256 of `value` at creation time. Re-checked at stage time.
    pub sha256: String,
}

impl Patch {
    /// Build a patch and stamp it with the checksum of `value`.
    pub fn create(id: impl Into<String>, target_key: impl Into<String>, value: Value) -> Self {
        let sha256 = sha256_of(&value);
        Self { id: id.into(), target_key: target_key.into(), value, sha256 }
    }

    /// Whether the recorded checksum still matches the payload.
    fn checksum_valid(&self) -> bool {
        sha256_of(&self.value) == self.sha256
    }
}

/// One audit-trail record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditEntry {
    /// `"stage"`, `"apply"`, or `"rollback"`.
    pub action: String,
    /// The config key involved.
    pub key: String,
    /// The payload checksum (or the rolled-back-from checksum).
    pub sha256: String,
}

/// The live config map plus staged patches, rollback snapshots, and audit log.
#[derive(Default)]
pub struct PatchRegistry {
    active: HashMap<String, Value>,
    staged: HashMap<String, Patch>,
    /// Previous value per key, captured at apply time. `None` payload means the
    /// key did not exist before (rollback should delete it).
    rollback: HashMap<String, Option<Value>>,
    audit: Vec<AuditEntry>,
}

impl PatchRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed (or overwrite) a config key without going through the patch flow.
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.active.insert(key.into(), value);
    }

    /// Read the active value for a key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.active.get(key)
    }

    /// Whether a patch id is currently staged.
    pub fn is_staged(&self, id: &str) -> bool {
        self.staged.contains_key(id)
    }

    /// The audit trail in chronological order.
    pub fn audit(&self) -> &[AuditEntry] {
        &self.audit
    }

    /// Validate a patch's checksum and stage it. Does not touch active config.
    pub fn stage(&mut self, patch: Patch) -> Result<()> {
        if !patch.checksum_valid() {
            return Err(AgentError::PatchRejected(format!(
                "checksum mismatch for patch `{}`",
                patch.id
            )));
        }
        self.audit.push(AuditEntry {
            action: "stage".into(),
            key: patch.target_key.clone(),
            sha256: patch.sha256.clone(),
        });
        self.staged.insert(patch.id.clone(), patch);
        Ok(())
    }

    /// Promote a staged patch into active config, snapshotting the prior value
    /// for rollback.
    pub fn apply(&mut self, id: &str) -> Result<()> {
        let patch = self
            .staged
            .remove(id)
            .ok_or_else(|| AgentError::PatchRejected(format!("no staged patch `{id}`")))?;
        let prev = self.active.get(&patch.target_key).cloned();
        self.rollback.insert(patch.target_key.clone(), prev);
        self.active.insert(patch.target_key.clone(), patch.value.clone());
        self.audit.push(AuditEntry {
            action: "apply".into(),
            key: patch.target_key.clone(),
            sha256: patch.sha256.clone(),
        });
        Ok(())
    }

    /// Restore the value a key held before its last apply. If the key did not
    /// exist before, it is removed.
    pub fn rollback(&mut self, key: &str) -> Result<()> {
        let prev = self
            .rollback
            .remove(key)
            .ok_or_else(|| AgentError::PatchRejected(format!("no rollback history for `{key}`")))?;
        match &prev {
            Some(v) => {
                self.active.insert(key.to_string(), v.clone());
            }
            None => {
                self.active.remove(key);
            }
        }
        let sha = prev.as_ref().map(sha256_of).unwrap_or_default();
        self.audit.push(AuditEntry {
            action: "rollback".into(),
            key: key.to_string(),
            sha256: sha,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checksum_valid_after_create() {
        assert!(Patch::create("a", "k", json!({"x": 1})).checksum_valid());
    }

    #[test]
    fn double_apply_requires_restage() {
        let mut r = PatchRegistry::new();
        r.set("k", json!(1));
        r.stage(Patch::create("p", "k", json!(2))).unwrap();
        r.apply("p").unwrap();
        assert!(r.apply("p").is_err());
    }
}
