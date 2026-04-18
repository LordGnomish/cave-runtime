//! Merkle hash chain — append-only log with integrity verification.

use crate::entry::{LedgerEntry, LedgerEntryKind};
use sha2::{Digest, Sha256};

/// Append-only Merkle hash chain.
pub struct MerkleChain {
    entries: Vec<LedgerEntry>,
}

/// Result of chain verification.
pub struct VerifyResult {
    pub is_valid: bool,
    pub entries_checked: u64,
    pub error: Option<String>,
}

impl MerkleChain {
    /// Create a new empty chain.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Append a new entry to the chain. Returns a reference to the created entry.
    pub fn append(
        &mut self,
        kind: LedgerEntryKind,
        actor: &str,
        action: &str,
        metadata: serde_json::Value,
    ) -> &LedgerEntry {
        let sequence = self.entries.len() as u64;
        let previous_hash = self
            .entries
            .last()
            .map(|e| e.hash.as_str())
            .unwrap_or("");

        let entry = LedgerEntry::new(sequence, previous_hash, kind, actor, action, metadata);
        self.entries.push(entry);
        self.entries.last().unwrap()
    }

    /// Get all entries in the chain.
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// Get the total number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Verify the entire chain integrity.
    /// Checks:
    /// 1. Each entry's hash matches its content
    /// 2. Each entry's previous_hash matches the prior entry's hash
    /// 3. Sequence numbers are monotonically increasing
    pub fn verify(&self) -> VerifyResult {
        if self.entries.is_empty() {
            return VerifyResult {
                is_valid: true,
                entries_checked: 0,
                error: None,
            };
        }

        for (i, entry) in self.entries.iter().enumerate() {
            // Check sequence
            if entry.sequence != i as u64 {
                return VerifyResult {
                    is_valid: false,
                    entries_checked: i as u64,
                    error: Some(format!(
                        "Sequence mismatch at index {i}: expected {i}, got {}",
                        entry.sequence
                    )),
                };
            }

            // Check hash integrity
            if !entry.verify_hash() {
                return VerifyResult {
                    is_valid: false,
                    entries_checked: i as u64,
                    error: Some(format!(
                        "Hash verification failed at sequence {}",
                        entry.sequence
                    )),
                };
            }

            // Check chain linkage
            if i == 0 {
                if !entry.previous_hash.is_empty() {
                    return VerifyResult {
                        is_valid: false,
                        entries_checked: 0,
                        error: Some("Genesis entry has non-empty previous_hash".to_string()),
                    };
                }
            } else {
                let prev = &self.entries[i - 1];
                if entry.previous_hash != prev.hash {
                    return VerifyResult {
                        is_valid: false,
                        entries_checked: i as u64,
                        error: Some(format!(
                            "Chain broken at sequence {}: previous_hash doesn't match entry {}",
                            entry.sequence,
                            prev.sequence
                        )),
                    };
                }
            }
        }

        VerifyResult {
            is_valid: true,
            entries_checked: self.entries.len() as u64,
            error: None,
        }
    }

    /// Compute the Merkle root of all entry hashes.
    /// Uses a binary Merkle tree over the entry hashes.
    pub fn merkle_root(&self) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut hashes: Vec<String> = self.entries.iter().map(|e| e.hash.clone()).collect();

        // Build Merkle tree bottom-up
        while hashes.len() > 1 {
            let mut next_level = Vec::new();
            for chunk in hashes.chunks(2) {
                let combined = if chunk.len() == 2 {
                    format!("{}{}", chunk[0], chunk[1])
                } else {
                    // Odd leaf: hash with itself
                    format!("{}{}", chunk[0], chunk[0])
                };
                let mut hasher = Sha256::new();
                hasher.update(combined.as_bytes());
                next_level.push(hex::encode(hasher.finalize()));
            }
            hashes = next_level;
        }

        hashes.into_iter().next().unwrap_or_default()
    }

    /// Find entries by kind.
    pub fn find_by_kind(&self, kind: &LedgerEntryKind) -> Vec<&LedgerEntry> {
        let kind_str = format!("{kind:?}");
        self.entries
            .iter()
            .filter(|e| format!("{:?}", e.kind) == kind_str)
            .collect()
    }

    /// Find entries by actor.
    pub fn find_by_actor(&self, actor: &str) -> Vec<&LedgerEntry> {
        self.entries
            .iter()
            .filter(|e| e.actor == actor)
            .collect()
    }

    /// Find entries by tenant.
    pub fn find_by_tenant(&self, tenant_id: &str) -> Vec<&LedgerEntry> {
        self.entries
            .iter()
            .filter(|e| e.tenant_id.as_deref() == Some(tenant_id))
            .collect()
    }
}

impl Default for MerkleChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_chain_is_valid() {
        let chain = MerkleChain::new();
        let result = chain.verify();
        assert!(result.is_valid);
        assert_eq!(result.entries_checked, 0);
    }

    #[test]
    fn test_single_entry_chain() {
        let mut chain = MerkleChain::new();
        chain.append(
            LedgerEntryKind::Deployment,
            "system",
            "genesis",
            serde_json::json!({}),
        );

        assert_eq!(chain.len(), 1);
        let result = chain.verify();
        assert!(result.is_valid);
        assert_eq!(result.entries_checked, 1);
    }

    #[test]
    fn test_multi_entry_chain() {
        let mut chain = MerkleChain::new();

        chain.append(
            LedgerEntryKind::Deployment,
            "system",
            "platform bootstrap",
            serde_json::json!({"profile": "hetzner-prod"}),
        );

        chain.append(
            LedgerEntryKind::TenantLifecycle,
            "admin-1",
            "created tenant acme",
            serde_json::json!({"tenant_id": "acme", "tier": "hard"}),
        );

        chain.append(
            LedgerEntryKind::Security,
            "cave-admission",
            "rejected unsigned image nginx:latest",
            serde_json::json!({"image": "nginx:latest", "reason": "no cosign signature"}),
        );

        assert_eq!(chain.len(), 3);
        let result = chain.verify();
        assert!(result.is_valid);
        assert_eq!(result.entries_checked, 3);
    }

    #[test]
    fn test_merkle_root_deterministic() {
        let mut chain = MerkleChain::new();
        chain.append(LedgerEntryKind::Deployment, "a", "x", serde_json::json!({}));
        let root1 = chain.merkle_root();

        // Root should be the single entry's hash
        assert!(!root1.is_empty());
        assert_eq!(root1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn test_find_by_kind() {
        let mut chain = MerkleChain::new();
        chain.append(LedgerEntryKind::Deployment, "a", "deploy 1", serde_json::json!({}));
        chain.append(LedgerEntryKind::Security, "b", "block 1", serde_json::json!({}));
        chain.append(LedgerEntryKind::Deployment, "a", "deploy 2", serde_json::json!({}));

        let deployments = chain.find_by_kind(&LedgerEntryKind::Deployment);
        assert_eq!(deployments.len(), 2);
    }

    #[test]
    fn test_find_by_actor() {
        let mut chain = MerkleChain::new();
        chain.append(LedgerEntryKind::Deployment, "alice", "deploy", serde_json::json!({}));
        chain.append(LedgerEntryKind::Deployment, "bob", "deploy", serde_json::json!({}));
        chain.append(LedgerEntryKind::Security, "alice", "block", serde_json::json!({}));

        let alice_entries = chain.find_by_actor("alice");
        assert_eq!(alice_entries.len(), 2);
    }

    #[test]
    fn test_empty_merkle_root() {
        let chain = MerkleChain::new();
        assert!(chain.merkle_root().is_empty());
    }
}
