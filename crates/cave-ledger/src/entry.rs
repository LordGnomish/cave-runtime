// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ledger entry types — the fundamental unit of the audit trail.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Classification of ledger events. Maps to CAVE platform operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerEntryKind {
    /// Deployment, promotion, rollback
    Deployment,
    /// Configuration change (profile, module, tenant)
    ConfigChange,
    /// Automated remediation (Reflex Engine, Crossplane Ops)
    SelfHealed,
    /// Compliance event (policy violation, export, attestation)
    Compliance,
    /// Security event (admission reject, signature verify, alert)
    Security,
    /// Identity event (login, role change, JIT grant, dormant disable)
    Identity,
    /// Tenant lifecycle (create, suspend, decommission)
    TenantLifecycle,
    /// FinOps event (budget alert, kill switch, egress quarantine)
    FinOps,
    /// Chaos experiment (started, completed, aborted)
    Chaos,
    /// APOL decision (AI reasoning trace)
    ApolDecision,
    /// Emergency action (mesh permissive, force-sync)
    Emergency,
    /// Backup/restore event
    BackupRestore,
    /// Certificate event (issue, renew, revoke)
    Certificate,
    /// Resurrection drill
    ResurrectionDrill,
    /// Platform upgrade
    PlatformUpgrade,
    /// Custom event type
    Custom(String),
}

/// A single immutable ledger entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Monotonically increasing sequence number
    pub sequence: u64,
    /// SHA-256 hash of this entry's content
    pub hash: String,
    /// Hash of the previous entry (empty string for genesis)
    pub previous_hash: String,
    /// Event classification
    pub kind: LedgerEntryKind,
    /// Who performed the action (cave_uid, service account, or APOL agent)
    pub actor: String,
    /// Human-readable action description
    pub action: String,
    /// Structured metadata (JSON)
    pub metadata: serde_json::Value,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Sigstore signature (optional, populated by CI/CD pipeline)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Tenant scope (None = platform-level event)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl LedgerEntry {
    /// Create a new entry, computing its hash from content + previous hash.
    pub fn new(
        sequence: u64,
        previous_hash: &str,
        kind: LedgerEntryKind,
        actor: &str,
        action: &str,
        metadata: serde_json::Value,
    ) -> Self {
        let timestamp = Utc::now();

        // Hash computation: SHA-256(sequence | previous_hash | kind | actor | action | metadata | timestamp)
        let content = format!(
            "{}|{}|{:?}|{}|{}|{}|{}",
            sequence,
            previous_hash,
            kind,
            actor,
            action,
            metadata,
            timestamp.to_rfc3339()
        );

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hex::encode(hasher.finalize());

        Self {
            sequence,
            hash,
            previous_hash: previous_hash.to_string(),
            kind,
            actor: actor.to_string(),
            action: action.to_string(),
            metadata,
            timestamp,
            signature: None,
            tenant_id: None,
        }
    }

    /// Verify this entry's hash matches its content.
    pub fn verify_hash(&self) -> bool {
        let content = format!(
            "{}|{}|{:?}|{}|{}|{}|{}",
            self.sequence,
            self.previous_hash,
            self.kind,
            self.actor,
            self.action,
            self.metadata,
            self.timestamp.to_rfc3339()
        );

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let computed = hex::encode(hasher.finalize());

        computed == self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_hash_verification() {
        let entry = LedgerEntry::new(
            1,
            "",
            LedgerEntryKind::Deployment,
            "user-123",
            "deployed cave-portal v0.2.0",
            serde_json::json!({"version": "0.2.0"}),
        );

        assert!(entry.verify_hash());
        assert_eq!(entry.sequence, 1);
        assert!(entry.previous_hash.is_empty());
        assert!(!entry.hash.is_empty());
    }

    #[test]
    fn test_tampered_entry_fails_verification() {
        let mut entry = LedgerEntry::new(
            1,
            "",
            LedgerEntryKind::Security,
            "admin",
            "rejected unsigned image",
            serde_json::json!({}),
        );

        // Tamper with the action
        entry.action = "approved unsigned image".to_string();
        assert!(!entry.verify_hash());
    }

    #[test]
    fn test_chain_links() {
        let e1 = LedgerEntry::new(
            0,
            "",
            LedgerEntryKind::Deployment,
            "system",
            "genesis",
            serde_json::json!({}),
        );

        let e2 = LedgerEntry::new(
            1,
            &e1.hash,
            LedgerEntryKind::ConfigChange,
            "admin",
            "updated profile",
            serde_json::json!({}),
        );

        assert_eq!(e2.previous_hash, e1.hash);
        assert_ne!(e1.hash, e2.hash);
    }
}
