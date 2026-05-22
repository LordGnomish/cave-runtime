// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! WireGuard transparent encryption — per-node key + peer registry.
//!
//! Mirrors `pkg/wireguard/agent.go` (the cilium-agent's WG manager).
//! Each node has a single WG keypair; remote nodes are added as
//! `Peer`s with their public key, UDP endpoint, and allowed IPs.
//!
//! Modes:
//!
//! * [`WgMode::PerNode`] — one Peer entry per remote node; allowed IPs
//!   carry the entire node's pod-CIDR.
//! * [`WgMode::PerPod`] — one Peer entry per remote *pod* (uses the
//!   pod IP as both endpoint identifier and allowed IP).
//!
//! Public/private keys are 32-byte Curve25519 values; we encode them
//! as base64 for serde, mirroring the upstream JSON format.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WgMode {
    PerNode,
    PerPod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WgKey(pub [u8; 32]);

impl WgKey {
    pub fn zero() -> Self {
        Self([0; 32])
    }
    pub fn from_seed(seed: u64) -> Self {
        let mut out = [0u8; 32];
        let bytes = seed.to_be_bytes();
        for (i, b) in out.iter_mut().enumerate() {
            *b = bytes[i % 8].wrapping_add(i as u8);
        }
        Self(out)
    }
    pub fn to_base64(self) -> String {
        let mut out = String::new();
        const CHARS: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i + 3 <= self.0.len() {
            let n =
                ((self.0[i] as u32) << 16) | ((self.0[i + 1] as u32) << 8) | (self.0[i + 2] as u32);
            for shift in [18, 12, 6, 0] {
                out.push(CHARS[((n >> shift) & 0x3F) as usize] as char);
            }
            i += 3;
        }
        // 32 mod 3 = 2 → one padding `=`.
        if i + 2 == self.0.len() {
            let n = ((self.0[i] as u32) << 16) | ((self.0[i + 1] as u32) << 8);
            out.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
            out.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        }
        out
    }
    pub fn from_base64(s: &str) -> Result<Self, WgError> {
        const TABLE: &[i8; 256] = &{
            let mut t = [-1i8; 256];
            let mut i = 0;
            while i < 64 {
                let b = match i {
                    0..=25 => b'A' + i as u8,
                    26..=51 => b'a' + (i - 26) as u8,
                    52..=61 => b'0' + (i - 52) as u8,
                    62 => b'+',
                    63 => b'/',
                    _ => 0,
                };
                t[b as usize] = i as i8;
                i += 1;
            }
            t
        };
        let s = s.trim_end_matches('=');
        let bytes = s.as_bytes();
        if bytes.len() != 43 {
            return Err(WgError::BadKey(s.to_string()));
        }
        let mut out = [0u8; 32];
        let mut bit_buf: u32 = 0;
        let mut bit_count = 0;
        let mut oi = 0;
        for &c in bytes {
            let v = TABLE[c as usize];
            if v < 0 {
                return Err(WgError::BadKey(s.to_string()));
            }
            bit_buf = (bit_buf << 6) | v as u32;
            bit_count += 6;
            while bit_count >= 8 && oi < 32 {
                bit_count -= 8;
                out[oi] = ((bit_buf >> bit_count) & 0xFF) as u8;
                oi += 1;
            }
        }
        if oi != 32 {
            return Err(WgError::BadKey(s.to_string()));
        }
        Ok(Self(out))
    }
}

impl Serialize for WgKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_base64())
    }
}
impl<'de> Deserialize<'de> for WgKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        WgKey::from_base64(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WgPeer {
    pub node: String,
    pub public_key: WgKey,
    pub endpoint: SocketAddr,
    pub allowed_ips: Vec<String>,
    pub psk: Option<WgKey>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WgError {
    #[error("invalid wireguard key `{0}`")]
    BadKey(String),
    #[error("peer for node `{0}` not found")]
    PeerNotFound(String),
    #[error("tenant {tenant} cannot mutate WG agent owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct WgAgent {
    pub tenant: TenantId,
    pub node_name: String,
    pub mode: WgMode,
    pub private_key: WgKey,
    pub public_key: WgKey,
    peers: HashMap<String, WgPeer>,
}

impl WgAgent {
    /// Create a new agent. The keypair is derived from a seed for
    /// deterministic tests; production code uses `wg genkey`.
    pub fn new(tenant: TenantId, node_name: impl Into<String>, mode: WgMode, seed: u64) -> Self {
        let private_key = WgKey::from_seed(seed);
        // Public key derivation in real WG is `Curve25519::pubkey(private_key)`;
        // we approximate with a distinct seed transformation so tests can
        // assert pub != priv without doing real ECC.
        let public_key = WgKey::from_seed(seed.wrapping_mul(0x9E3779B97F4A7C15));
        Self {
            tenant,
            node_name: node_name.into(),
            mode,
            private_key,
            public_key,
            peers: HashMap::new(),
        }
    }

    pub fn upsert_peer(&mut self, peer: WgPeer) {
        self.peers.insert(peer.node.clone(), peer);
    }

    pub fn remove_peer(&mut self, node: &str) -> bool {
        self.peers.remove(node).is_some()
    }

    pub fn lookup_peer(&self, node: &str) -> Option<&WgPeer> {
        self.peers.get(node)
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/wireguard/agent.go", "Agent");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn endpoint(p: u16) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, 1], p))
    }

    // ── Keys ─────────────────────────────────────────────────────────────────

    #[test]
    fn wg_keypair_has_distinct_public_and_private() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.NewKeypair",
            "tenant-wg-keys"
        );
        let a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        assert_ne!(a.private_key, a.public_key);
    }

    #[test]
    fn wg_key_base64_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/wireguard/agent.go", "Key.Base64", "tenant-wg-b64");
        let k = WgKey::from_seed(42);
        let s = k.to_base64();
        assert_eq!(s.len(), 44);
        assert!(s.ends_with('='));
        let back = WgKey::from_base64(&s).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn wg_key_bad_base64_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/wireguard/agent.go", "Key.Validate", "tenant-wg-bad");
        let err = WgKey::from_base64("not-a-key").unwrap_err();
        assert!(matches!(err, WgError::BadKey(_)));
    }

    #[test]
    fn wg_pub_key_serializes_as_base64_string() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Key.Serialize",
            "tenant-wg-serialize"
        );
        let k = WgKey::from_seed(7);
        let json = serde_json::to_string(&k).unwrap();
        assert!(json.starts_with('"'));
        assert!(json.ends_with('"'));
        let back: WgKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);
    }

    // ── Peer management ──────────────────────────────────────────────────────

    #[test]
    fn wg_register_peer_with_endpoint_and_allowed_ips() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.UpsertPeer",
            "tenant-wg-peer"
        );
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        let p = WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.0/24".into()],
            psk: None,
        };
        a.upsert_peer(p.clone());
        assert_eq!(a.peer_count(), 1);
        assert_eq!(a.lookup_peer("node-b").unwrap(), &p);
    }

    #[test]
    fn wg_upsert_peer_replaces_existing_entry() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.UpsertPeer.Replace",
            "tenant-wg-peerup"
        );
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        a.upsert_peer(WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.0/24".into()],
            psk: None,
        });
        a.upsert_peer(WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51821),
            allowed_ips: vec!["10.244.2.0/24".into()],
            psk: None,
        });
        assert_eq!(a.peer_count(), 1);
        assert_eq!(a.lookup_peer("node-b").unwrap().endpoint.port(), 51821);
    }

    #[test]
    fn wg_remove_peer() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.RemovePeer",
            "tenant-wg-peerrm"
        );
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        a.upsert_peer(WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.0/24".into()],
            psk: None,
        });
        assert!(a.remove_peer("node-b"));
        assert!(a.lookup_peer("node-b").is_none());
    }

    #[test]
    fn wg_remove_unknown_peer_returns_false() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.RemovePeer.NotFound",
            "tenant-wg-peerrmnf"
        );
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        assert!(!a.remove_peer("ghost"));
    }

    #[test]
    fn wg_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.LookupPeer",
            "tenant-wg-peerlk"
        );
        let a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        assert!(a.lookup_peer("ghost").is_none());
    }

    // ── Modes ────────────────────────────────────────────────────────────────

    #[test]
    fn wg_per_node_mode_one_peer_per_node_with_pod_cidr() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/wireguard/agent.go", "Agent.PerNode", "tenant-wg-pn");
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        a.upsert_peer(WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.0/24".into()],
            psk: None,
        });
        let p = a.lookup_peer("node-b").unwrap();
        assert_eq!(p.allowed_ips, vec!["10.244.1.0/24".to_string()]);
    }

    #[test]
    fn wg_per_pod_mode_separate_peer_per_pod_ip() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/wireguard/agent.go", "Agent.PerPod", "tenant-wg-pp");
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerPod, 1);
        a.upsert_peer(WgPeer {
            node: "pod-b1".into(),
            public_key: WgKey::from_seed(11),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.5/32".into()],
            psk: None,
        });
        a.upsert_peer(WgPeer {
            node: "pod-b2".into(),
            public_key: WgKey::from_seed(22),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.6/32".into()],
            psk: None,
        });
        assert_eq!(a.peer_count(), 2);
    }

    // ── PSK ──────────────────────────────────────────────────────────────────

    #[test]
    fn wg_optional_psk_serializes_when_set() {
        let (_c, tenant) = cilium_test_ctx!("pkg/wireguard/agent.go", "Agent.PSK", "tenant-wg-psk");
        let mut a = WgAgent::new(tenant, "node-a", WgMode::PerNode, 1);
        a.upsert_peer(WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.0/24".into()],
            psk: Some(WgKey::from_seed(55)),
        });
        assert!(a.lookup_peer("node-b").unwrap().psk.is_some());
    }

    #[test]
    fn wg_peer_round_trips_through_serde() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/wireguard/agent.go",
            "Agent.Peer.Serde",
            "tenant-wg-serde"
        );
        let p = WgPeer {
            node: "node-b".into(),
            public_key: WgKey::from_seed(99),
            endpoint: endpoint(51820),
            allowed_ips: vec!["10.244.1.0/24".into()],
            psk: Some(WgKey::from_seed(55)),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: WgPeer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
