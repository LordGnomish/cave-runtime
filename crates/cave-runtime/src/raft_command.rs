// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! State-machine commands carried in Raft log entries.
//!
//! The Raft core in [`crate::raft_core`] replicates opaque
//! `Vec<u8>` payloads; this module is the typed schema the host
//! layers on top so committed entries can be deterministically
//! applied to cave-etcd's `KvStore` and cave-apiserver's
//! `ResourceStore`.
//!
//! ## Wire format
//!
//! Commands are serialised with `serde_json`. JSON (rather than
//! bincode) trades a few bytes per entry for human-readable WAL
//! contents — operators debugging a divergent replica can `jq` the
//! log instead of reaching for a hex-dump. The cost is negligible
//! for the per-entry sizes Raft is replicating (KV puts of a few
//! KB, Resource upserts of <100 KB).
//!
//! ## NoOp entries
//!
//! Per Raft Section 8 ("Client interaction"), a fresh leader
//! commits a no-op entry at the start of its term so its read-index
//! reflects the new term. The apply daemon ignores `NoOp` rather
//! than rejecting it — it is a valid Raft event with no
//! state-machine side effect.

use serde::{Deserialize, Serialize};

/// The set of mutations that flow through Raft. Adding a new variant
/// must be backwards-compatible: older replicas that don't recognise
/// the variant will reject the entry as malformed and re-enter
/// recovery, which is exactly what we want on a version-skew between
/// peers. New leaders proposing new variants therefore wait for the
/// follower roll to complete before they can rely on them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RaftCommand {
    /// `cave-etcd` PUT. cave-etcd's `KvStore::put` keys keys as `String`
    /// (utf-8); the RaftCommand surface follows the same contract.
    /// When `lease` is Some, the put is associated with that lease ID.
    EtcdPut { key: String, value: String, lease: Option<i64> },
    /// `cave-etcd` DELETE — single-key removal. range_end is optional
    /// to match `DeleteRangeRequest` (None = single-key).
    EtcdDelete { key: String, range_end: Option<String> },
    /// `cave-apiserver` upsert — JSON-encoded `Resource` (the
    /// `#[serde(tag = "kind")]` discriminator is preserved).
    /// Apiserver-side wiring takes care of the conflict semantics —
    /// the command itself is pure last-writer-wins so concurrent
    /// proposers don't deadlock on a CAS at the Raft layer.
    ApiserverUpsert { resource: serde_json::Value },
    /// `cave-apiserver` delete — addresses a resource by its
    /// `(kind, namespace, name)` triple as the store keys it.
    ApiserverDelete { kind: String, namespace: String, name: String },
    /// New-leader marker. Per Raft §8 a fresh leader commits one to
    /// re-anchor the read-index without changing state-machine
    /// state. The apply daemon recognises and ignores it.
    NoOp,
}

/// Error path for round-trip encode/decode. Distinct from
/// `ApplyError` so callers can tell deserialisation problems apart
/// from state-machine rejections.
#[derive(Debug, thiserror::Error)]
pub enum RaftCommandError {
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
}

impl RaftCommand {
    /// Encode to a `Vec<u8>` suitable for `RaftCore::propose`.
    pub fn encode(&self) -> Result<Vec<u8>, RaftCommandError> {
        serde_json::to_vec(self).map_err(RaftCommandError::Encode)
    }

    /// Decode from a committed log entry's payload.
    pub fn decode(bytes: &[u8]) -> Result<Self, RaftCommandError> {
        // Empty payloads are how earlier sessions encoded the leader
        // no-op (literally `propose(vec![])`). Treat them as NoOp so
        // a mixed-encoding log still applies cleanly.
        if bytes.is_empty() {
            return Ok(Self::NoOp);
        }
        serde_json::from_slice(bytes).map_err(RaftCommandError::Decode)
    }

    /// Short human-readable summary for log lines + the
    /// /admin/cluster Raft view. Avoids dumping full payloads.
    pub fn summary(&self) -> String {
        match self {
            Self::EtcdPut { key, value, .. } => format!(
                "etcd_put(key={}, val={} bytes)",
                truncate_key(key.as_bytes()),
                value.len()
            ),
            Self::EtcdDelete { key, .. } => {
                format!("etcd_delete(key={})", truncate_key(key.as_bytes()))
            }
            Self::ApiserverUpsert { resource } => {
                let kind = resource.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let name = resource
                    .get("metadata")
                    .and_then(|m| m.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("?");
                format!("apiserver_upsert({kind}/{name})")
            }
            Self::ApiserverDelete { kind, namespace, name } => {
                format!("apiserver_delete({kind}/{namespace}/{name})")
            }
            Self::NoOp => "noop".into(),
        }
    }
}

fn truncate_key(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= 64 {
        s.into_owned()
    } else {
        format!("{}…({}B)", &s[..60], bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etcd_put_roundtrip() {
        let cmd = RaftCommand::EtcdPut { key: "/foo".into(), value: "bar".into(), lease: None };
        let bytes = cmd.encode().unwrap();
        let back = RaftCommand::decode(&bytes).unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn etcd_put_roundtrip_with_lease() {
        let cmd = RaftCommand::EtcdPut {
            key: "/lease-key".into(),
            value: "x".into(),
            lease: Some(7_777),
        };
        let bytes = cmd.encode().unwrap();
        assert_eq!(RaftCommand::decode(&bytes).unwrap(), cmd);
    }

    #[test]
    fn etcd_delete_roundtrip() {
        let cmd = RaftCommand::EtcdDelete { key: "/foo".into(), range_end: None };
        let bytes = cmd.encode().unwrap();
        assert_eq!(RaftCommand::decode(&bytes).unwrap(), cmd);
    }

    #[test]
    fn etcd_delete_with_range_end_roundtrip() {
        let cmd = RaftCommand::EtcdDelete {
            key: "/a".into(),
            range_end: Some("/z".into()),
        };
        let bytes = cmd.encode().unwrap();
        assert_eq!(RaftCommand::decode(&bytes).unwrap(), cmd);
    }

    #[test]
    fn apiserver_upsert_roundtrip_preserves_kind_tag() {
        let cmd = RaftCommand::ApiserverUpsert {
            resource: serde_json::json!({
                "kind": "ConfigMap",
                "metadata": {"name": "demo", "namespace": "default"},
                "data": {"answer": "42"},
            }),
        };
        let bytes = cmd.encode().unwrap();
        let back = RaftCommand::decode(&bytes).unwrap();
        assert_eq!(back, cmd);
        if let RaftCommand::ApiserverUpsert { resource } = back {
            assert_eq!(resource["kind"].as_str(), Some("ConfigMap"));
        } else {
            panic!("variant lost");
        }
    }

    #[test]
    fn apiserver_delete_roundtrip() {
        let cmd = RaftCommand::ApiserverDelete {
            kind: "ConfigMap".into(),
            namespace: "default".into(),
            name: "demo".into(),
        };
        let bytes = cmd.encode().unwrap();
        assert_eq!(RaftCommand::decode(&bytes).unwrap(), cmd);
    }

    #[test]
    fn empty_payload_decodes_to_noop() {
        // Earlier sessions used `propose(vec![])` for leader no-ops.
        // Make sure replay across a binary upgrade still applies them.
        assert_eq!(RaftCommand::decode(&[]).unwrap(), RaftCommand::NoOp);
    }

    #[test]
    fn noop_roundtrip() {
        let bytes = RaftCommand::NoOp.encode().unwrap();
        assert_eq!(RaftCommand::decode(&bytes).unwrap(), RaftCommand::NoOp);
    }

    #[test]
    fn decode_returns_error_on_garbage() {
        let err = RaftCommand::decode(b"\x00not-json\xff").unwrap_err();
        assert!(matches!(err, RaftCommandError::Decode(_)));
    }

    #[test]
    fn summary_strings_are_compact() {
        let put = RaftCommand::EtcdPut {
            key: "/x".into(),
            value: "x".repeat(1024),
            lease: None,
        };
        let s = put.summary();
        assert!(s.contains("etcd_put"));
        assert!(s.contains("val=1024 bytes"));
        let upsert = RaftCommand::ApiserverUpsert {
            resource: serde_json::json!({"kind":"Pod","metadata":{"name":"web"}}),
        };
        assert!(upsert.summary().contains("Pod/web"));
        assert_eq!(RaftCommand::NoOp.summary(), "noop");
    }

    #[test]
    fn summary_truncates_long_keys() {
        let key: String = (0..200).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let cmd = RaftCommand::EtcdDelete { key, range_end: None };
        let s = cmd.summary();
        assert!(s.contains("…(200B)"));
        // Ensure the human-readable head is included.
        assert!(s.contains("etcd_delete"));
    }
}
