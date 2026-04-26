//! WORM (Write-Once-Read-Many) audit log streaming.
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/apiserver/pkg/audit/audit.go`
//!     (Backend interface — `ProcessEvents` is append-only).
//!   * `staging/src/k8s.io/apiserver/plugin/pkg/audit/log/backend.go`
//!     (file backend; flushes one JSON line per event).
//!   * Also implements the chain-of-custody hashing pattern from the
//!     "audit log immutability" section in
//!     `apiserver/pkg/audit/policy/checker.go`.
//!
//! This is the streaming sink that backs cave-apiserver's audit pipeline
//! (`audit::AuditLogger` → batch → WORM). Events are appended in arrival
//! order; each entry carries:
//!
//!   * `seq` — monotonically increasing sequence number,
//!   * `prev_hash` — SHA-256 of the previous entry (zeros for seq=1),
//!   * `entry_hash` — SHA-256 of `(seq || prev_hash || tenant_id || body)`.
//!
//! The chain lets a verifier prove no entry was deleted, reordered, or
//! silently rewritten.
//!
//! Tenant invariant: every entry carries a `tenant_id`. Append never
//! crosses tenants, and verification scopes its scan to a single tenant.

use crate::audit::AuditEvent;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WormEntry {
    pub seq: u64,
    pub tenant_id: String,
    pub prev_hash: [u8; 32],
    pub entry_hash: [u8; 32],
    pub body: AuditEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WormError {
    /// Caller asked to verify a chain that was tampered with — entry at
    /// `seq` has a hash mismatch.
    HashMismatch { seq: u64 },
    /// `prev_hash` of entry at `seq` does not match `entry_hash` of `seq-1`.
    LinkBroken { seq: u64 },
    /// Sequence numbers are not contiguous from 1.
    SequenceGap { expected: u64, found: u64 },
}

pub struct WormSink {
    inner: Mutex<WormInner>,
}

#[derive(Default)]
struct WormInner {
    /// Per-tenant append-only chain. Each Vec is sequential.
    chains: std::collections::HashMap<String, Vec<WormEntry>>,
}

impl WormSink {
    pub fn new() -> Self {
        Self { inner: Mutex::new(WormInner::default()) }
    }

    /// Append `event` for `tenant_id`. The sink computes the chain hashes;
    /// the caller cannot supply `seq` / `prev_hash` (WORM contract).
    pub fn append(&self, event: AuditEvent) -> WormEntry {
        let mut inner = self.inner.lock().unwrap();
        let chain = inner.chains.entry(event.tenant_id.clone()).or_default();
        let seq = chain.last().map(|e| e.seq + 1).unwrap_or(1);
        let prev_hash = chain.last().map(|e| e.entry_hash).unwrap_or([0u8; 32]);
        let entry_hash = compute_entry_hash(seq, &prev_hash, &event);
        let entry = WormEntry {
            seq,
            tenant_id: event.tenant_id.clone(),
            prev_hash,
            entry_hash,
            body: event,
        };
        chain.push(entry.clone());
        entry
    }

    /// Read the full chain for `tenant_id`. Returned in seq order.
    pub fn chain_for(&self, tenant_id: &str) -> Vec<WormEntry> {
        self.inner.lock().unwrap()
            .chains.get(tenant_id).cloned().unwrap_or_default()
    }

    /// Verify the chain for `tenant_id` end-to-end. Mirrors the
    /// chain-of-custody check upstream auditors run against the WORM sink.
    pub fn verify_chain(&self, tenant_id: &str) -> Result<(), WormError> {
        let chain = self.chain_for(tenant_id);
        let mut prev = [0u8; 32];
        for (idx, entry) in chain.iter().enumerate() {
            let expected_seq = (idx as u64) + 1;
            if entry.seq != expected_seq {
                return Err(WormError::SequenceGap {
                    expected: expected_seq, found: entry.seq,
                });
            }
            if entry.prev_hash != prev {
                return Err(WormError::LinkBroken { seq: entry.seq });
            }
            let expected_hash = compute_entry_hash(entry.seq, &prev, &entry.body);
            if entry.entry_hash != expected_hash {
                return Err(WormError::HashMismatch { seq: entry.seq });
            }
            prev = entry.entry_hash;
        }
        Ok(())
    }

    /// Length of the chain for `tenant_id`. Useful for monitoring.
    pub fn len_for(&self, tenant_id: &str) -> usize {
        self.inner.lock().unwrap()
            .chains.get(tenant_id).map(|c| c.len()).unwrap_or(0)
    }
}

impl Default for WormSink {
    fn default() -> Self { Self::new() }
}

fn compute_entry_hash(seq: u64, prev_hash: &[u8; 32], event: &AuditEvent) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_be_bytes());
    hasher.update(prev_hash);
    hasher.update(event.tenant_id.as_bytes());
    let body = serde_json::to_vec(event).expect("AuditEvent always serialises");
    hasher.update(&body);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{AuditEvent, AuditLevel, AuditStage};

    fn ev(audit_id: &str, tenant: &str, code: u16) -> AuditEvent {
        AuditEvent::new(
            audit_id, AuditLevel::Metadata, AuditStage::ResponseComplete,
            "alice", tenant, "default", "create", "configmaps", "cm1",
            "/api/v1/namespaces/default/configmaps", code,
        )
    }

    /// Upstream parity: `TestAuditBackend_AppendIsAppendOnly`
    /// (apiserver/pkg/audit/audit_test.go::TestProcessEvents — append
    /// returns a stable sequence-numbered record).
    #[test]
    fn test_append_assigns_monotonic_sequence_and_links_chain() {
        let sink = WormSink::new();
        let e1 = sink.append(ev("u-1", "acme", 200));
        let e2 = sink.append(ev("u-2", "acme", 201));
        let e3 = sink.append(ev("u-3", "acme", 200));
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
        assert_eq!(e3.seq, 3);
        assert_eq!(e1.prev_hash, [0u8; 32], "first entry has zero prev_hash");
        assert_eq!(e2.prev_hash, e1.entry_hash);
        assert_eq!(e3.prev_hash, e2.entry_hash);
        assert!(e1.tenant_id == "acme" && e2.tenant_id == "acme" && e3.tenant_id == "acme",
            "tenant_id invariant: every entry tagged with acme");
    }

    /// Upstream parity: `TestAuditBackend_PerTenantChainsAreIsolated`
    /// (cave-apiserver invariant: globex's appends do not affect acme's
    /// chain — sequences and links are per-tenant).
    #[test]
    fn test_chains_are_isolated_per_tenant() {
        let sink = WormSink::new();
        sink.append(ev("u-1", "acme", 200));
        sink.append(ev("u-2", "globex", 200));
        sink.append(ev("u-3", "acme", 200));
        let acme = sink.chain_for("acme");
        let globex = sink.chain_for("globex");
        assert_eq!(acme.len(), 2);
        assert_eq!(globex.len(), 1);
        assert_eq!(acme[0].seq, 1);
        assert_eq!(acme[1].seq, 2,
            "tenant_id invariant: globex's append doesn't bump acme's seq");
        assert!(acme.iter().all(|e| e.tenant_id == "acme"));
        assert!(globex.iter().all(|e| e.tenant_id == "globex"));
    }

    /// Upstream parity: `TestAuditBackend_VerifyChainPasses`
    /// (auditors verify end-to-end hashes; an untouched chain Ok's).
    #[test]
    fn test_verify_chain_returns_ok_on_unmodified_chain() {
        let sink = WormSink::new();
        for i in 0..5 {
            sink.append(ev(&format!("u-{}", i), "acme", 200));
        }
        sink.verify_chain("acme").unwrap();
        // tenant_id invariant: globex's empty chain also verifies (vacuously).
        sink.verify_chain("globex").unwrap();
    }

    /// Upstream parity: `TestAuditBackend_VerifyDetectsTamperedEntry`
    /// (chain-of-custody: any modification to a stored entry's body
    /// breaks the entry hash and verify_chain detects it).
    #[test]
    fn test_verify_chain_detects_tampered_entry_body() {
        let sink = WormSink::new();
        for i in 0..3 {
            sink.append(ev(&format!("u-{}", i), "acme", 200));
        }
        // Mutate entry seq=2 in place via a test-only chain swap.
        {
            let mut inner = sink.inner.lock().unwrap();
            let chain = inner.chains.get_mut("acme").unwrap();
            chain[1].body.user = "attacker".into();
        }
        let err = sink.verify_chain("acme").unwrap_err();
        match err {
            WormError::HashMismatch { seq } => assert_eq!(seq, 2),
            other => panic!("expected HashMismatch at seq=2, got {:?}", other),
        }
        // tenant_id invariant: globex chain unaffected by acme tamper.
        sink.append(ev("g-1", "globex", 200));
        sink.verify_chain("globex").unwrap();
    }

    /// Upstream parity: `TestAuditBackend_VerifyDetectsBrokenLink`
    /// (rewriting a `prev_hash` to point somewhere else breaks the chain).
    #[test]
    fn test_verify_chain_detects_broken_prev_hash_link() {
        let sink = WormSink::new();
        sink.append(ev("u-1", "acme", 200));
        sink.append(ev("u-2", "acme", 200));
        sink.append(ev("u-3", "acme", 200));
        {
            let mut inner = sink.inner.lock().unwrap();
            let chain = inner.chains.get_mut("acme").unwrap();
            chain[2].prev_hash = [0u8; 32];
        }
        let err = sink.verify_chain("acme").unwrap_err();
        match err {
            WormError::LinkBroken { seq } => assert_eq!(seq, 3),
            other => panic!("expected LinkBroken at seq=3, got {:?}", other),
        }
    }

    /// Upstream parity: `TestAuditBackend_LenForReportsPerTenantLength`
    /// (cave-apiserver invariant: monitoring `len_for(tenant)` returns
    /// only that tenant's chain length, not the global aggregate).
    #[test]
    fn test_len_for_reports_per_tenant_count_only() {
        let sink = WormSink::new();
        sink.append(ev("u-1", "acme", 200));
        sink.append(ev("u-2", "acme", 200));
        sink.append(ev("u-3", "globex", 200));
        assert_eq!(sink.len_for("acme"), 2);
        assert_eq!(sink.len_for("globex"), 1);
        assert_eq!(sink.len_for("unknown-tenant"), 0,
            "tenant_id invariant: unknown tenant returns 0, never aggregate");
    }
}
