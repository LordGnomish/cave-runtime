// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Transparent encryption — WireGuard, IPsec, and a PQC-ready hybrid.
//!
//! Ports cilium's transparent-encryption control plane:
//!   * **WireGuard** (`pkg/wireguard/agent`): the per-node device with its
//!     keypair, peers, and `allowed-ips`. The peer/allowed-ip bookkeeping
//!     and the longest-prefix "which tunnel does this dest route through"
//!     decision are ported faithfully — cilium delegates the Noise crypto
//!     itself to the kernel/`wgctrl`, and so do we.
//!   * **IPsec** (`pkg/ipsec`): the SPI key-id rotation state machine
//!     (key ids live in 1..=15, SPI 0 reserved), keeping current + previous
//!     keys so in-flight packets still decrypt across a rotation.
//!   * **PQC-ready hybrid** (`encryption: { type: wireguard }` future work):
//!     the [`pqc`] submodule carries the FIPS-203 ML-KEM-768 / FIPS-204
//!     ML-DSA-65 parameter sizes, a [`pqc::Kem`] trait, and a hybrid
//!     [`pqc::combine`] that mixes the classical X25519 secret with the
//!     post-quantum KEM secret into one session key.
//!
//! Honesty note: the ML-KEM/ML-DSA lattice primitives are modelled behind a
//! trait (faithful interface + parameter sizes), NOT reimplemented — vetted
//! lattice crypto belongs to an audited library, not this control-plane
//! port. The hybrid *combiner* and all WireGuard/IPsec bookkeeping are real.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};

use ipnet::Ipv4Net;
use thiserror::Error;

// ---------------------------------------------------------------------------
// WireGuard
// ---------------------------------------------------------------------------

/// A 32-byte WireGuard (Curve25519) key, rendered as standard base64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WireguardKey([u8; 32]);

impl WireguardKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        WireguardKey(b)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_base64(&self) -> String {
        base64_encode(&self.0)
    }

    pub fn from_base64(s: &str) -> Result<Self, EncryptionError> {
        let v = base64_decode(s).ok_or(EncryptionError::BadKey)?;
        if v.len() != 32 {
            return Err(EncryptionError::BadKey);
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(&v);
        Ok(WireguardKey(b))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncryptionError {
    #[error("invalid WireGuard key encoding")]
    BadKey,
}

/// A WireGuard peer (a remote cilium node).
#[derive(Debug, Clone)]
pub struct Peer {
    pub public_key: WireguardKey,
    pub endpoint: Option<SocketAddr>,
    pub allowed_ips: Vec<Ipv4Net>,
    pub node_name: String,
}

/// The local WireGuard device (`cilium_wg0`).
#[derive(Debug)]
pub struct WireguardDevice {
    private_key: WireguardKey,
    pub listen_port: u16,
    peers: HashMap<[u8; 32], Peer>,
}

impl WireguardDevice {
    pub fn new(private_key: WireguardKey, listen_port: u16) -> Self {
        WireguardDevice {
            private_key,
            listen_port,
            peers: HashMap::new(),
        }
    }

    pub fn private_key(&self) -> &WireguardKey {
        &self.private_key
    }

    /// Insert or update a peer, merging allowed-ips (cilium's
    /// `updatePeerByConfig` replaces the peer but unions the prefixes).
    pub fn upsert_peer(&mut self, peer: Peer) {
        let k = *peer.public_key.as_bytes();
        match self.peers.get_mut(&k) {
            Some(existing) => {
                existing.endpoint = peer.endpoint.or(existing.endpoint);
                existing.node_name = peer.node_name;
                for net in peer.allowed_ips {
                    if !existing.allowed_ips.contains(&net) {
                        existing.allowed_ips.push(net);
                    }
                }
            }
            None => {
                self.peers.insert(k, peer);
            }
        }
    }

    pub fn remove_peer(&mut self, public_key: &WireguardKey) -> bool {
        self.peers.remove(public_key.as_bytes()).is_some()
    }

    pub fn peer(&self, public_key: &WireguardKey) -> Option<&Peer> {
        self.peers.get(public_key.as_bytes())
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Pick the peer (tunnel) a destination IP routes through, by
    /// longest-prefix match across all peers' allowed-ips.
    pub fn route(&self, ip: Ipv4Addr) -> Option<&Peer> {
        let mut best: Option<(&Peer, u8)> = None;
        for p in self.peers.values() {
            for net in &p.allowed_ips {
                if net.contains(&ip) {
                    let len = net.prefix_len();
                    if best.map(|(_, bl)| len > bl).unwrap_or(true) {
                        best = Some((p, len));
                    }
                }
            }
        }
        best.map(|(p, _)| p)
    }
}

// ---------------------------------------------------------------------------
// IPsec SPI rotation
// ---------------------------------------------------------------------------

/// IPsec key-id (SPI) rotation state. Key ids live in 1..=15; SPI 0 is
/// reserved. cilium keeps the previous key alive across a rotation so
/// packets encrypted under the old key still decrypt.
#[derive(Debug, Clone)]
pub struct IpsecState {
    spi: u8,
    prev: Option<u8>,
}

impl Default for IpsecState {
    fn default() -> Self {
        IpsecState { spi: 1, prev: None }
    }
}

impl IpsecState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_spi(&self) -> u8 {
        self.spi
    }

    pub fn prev_spi(&self) -> Option<u8> {
        self.prev
    }

    /// Rotate to the next key id, wrapping 15 → 1 (never 0).
    pub fn rotate(&mut self) -> u8 {
        self.prev = Some(self.spi);
        self.spi = if self.spi >= 15 { 1 } else { self.spi + 1 };
        self.spi
    }

    /// The decrypt path accepts the current and previous SPIs.
    pub fn accepts(&self, spi: u8) -> bool {
        spi == self.spi || Some(spi) == self.prev
    }
}

// ---------------------------------------------------------------------------
// Cipher suites + PQC hybrid
// ---------------------------------------------------------------------------

/// The negotiated key-agreement suite for a tunnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherSuite {
    /// Classical WireGuard (Curve25519 / X25519).
    X25519,
    /// Hybrid PQC: ML-KEM-768 encapsulation combined with X25519.
    MlKem768X25519,
}

impl CipherSuite {
    pub fn is_post_quantum(&self) -> bool {
        matches!(self, CipherSuite::MlKem768X25519)
    }
}

/// Post-quantum key-agreement modelling.
pub mod pqc {
    // FIPS-203 ML-KEM-768 sizes (bytes).
    pub const ML_KEM_768_PUBLIC_KEY: usize = 1184;
    pub const ML_KEM_768_SECRET_KEY: usize = 2400;
    pub const ML_KEM_768_CIPHERTEXT: usize = 1088;
    pub const ML_KEM_768_SHARED: usize = 32;

    // FIPS-204 ML-DSA-65 sizes (bytes).
    pub const ML_DSA_65_PUBLIC_KEY: usize = 1952;
    pub const ML_DSA_65_SECRET_KEY: usize = 4032;
    pub const ML_DSA_65_SIGNATURE: usize = 3309;

    /// A Key-Encapsulation Mechanism (ML-KEM and friends). Implemented by an
    /// audited lattice library in production; modelled here so the hybrid
    /// handshake structure is testable end-to-end.
    pub trait Kem {
        fn suite(&self) -> &'static str;
        /// Encapsulate against `public_key` → (ciphertext, shared secret).
        fn encapsulate(&self, public_key: &[u8]) -> (Vec<u8>, [u8; 32]);
        /// Decapsulate `ciphertext` with `secret_key` → shared secret.
        fn decapsulate(&self, secret_key: &[u8], ciphertext: &[u8]) -> [u8; 32];
    }

    /// Hybrid combiner: mix the classical (X25519) and post-quantum (KEM)
    /// shared secrets into one 32-byte session key. The construction is
    /// length-prefixed and domain-separated (`classical || pq`) so the two
    /// inputs are not interchangeable — the standard hybrid KDF shape. The
    /// mixing function below is a deterministic 256-bit absorbing state and
    /// is a structural stand-in for HKDF-SHA256, not a security primitive.
    pub fn combine(classical: &[u8], pq: &[u8]) -> [u8; 32] {
        // Eight 32-bit FNV-1a lanes, domain-separated per lane index.
        let mut lanes = [0u32; 8];
        for (i, lane) in lanes.iter_mut().enumerate() {
            *lane = 0x811c_9dc5 ^ (i as u32).wrapping_mul(0x9e37_79b1);
        }
        let mut absorb = |label: u8, data: &[u8], lanes: &mut [u32; 8]| {
            // Length-prefix + label for domain separation.
            let header = [label, (data.len() & 0xff) as u8, (data.len() >> 8) as u8];
            for &byte in header.iter().chain(data.iter()) {
                for (j, lane) in lanes.iter_mut().enumerate() {
                    *lane ^= byte as u32 ^ (j as u32);
                    *lane = lane.wrapping_mul(0x0100_0193); // FNV prime
                    *lane = lane.rotate_left(((byte as u32) + j as u32) % 31 + 1);
                }
            }
        };
        absorb(0x01, classical, &mut lanes);
        absorb(0x02, pq, &mut lanes);

        let mut out = [0u8; 32];
        for (i, lane) in lanes.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&lane.to_le_bytes());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// base64 (standard alphabet, with padding) — WireGuard key encoding
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    if input.contains('=') && input.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        let mut bits = 0;
        for &c in chunk {
            n = (n << 6) | val(c)?;
            bits += 6;
        }
        // Left-align the accumulated bits.
        n <<= 24 - bits;
        out.push((n >> 16) as u8);
        // bytes produced = bits / 8: 4 chars→3, 3 chars→2, 2 chars→1.
        if bits >= 16 {
            out.push((n >> 8) as u8);
        }
        if bits >= 24 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::pqc::*;
    use super::*;
    use std::net::Ipv4Addr;

    fn key(seed: u8) -> WireguardKey {
        WireguardKey::from_bytes([seed; 32])
    }

    fn cidr(s: &str) -> ipnet::Ipv4Net {
        s.parse().unwrap()
    }

    #[test]
    fn wireguard_key_base64_roundtrips() {
        let k = WireguardKey::from_bytes([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ]);
        let b64 = k.to_base64();
        // 32 bytes → 44 base64 chars (with one '=' pad).
        assert_eq!(b64.len(), 44);
        let back = WireguardKey::from_base64(&b64).unwrap();
        assert_eq!(back.as_bytes(), k.as_bytes());
    }

    #[test]
    fn device_upsert_merges_allowed_ips() {
        let mut dev = WireguardDevice::new(key(0), 51871);
        dev.upsert_peer(Peer {
            public_key: key(1),
            endpoint: None,
            allowed_ips: vec![cidr("10.0.1.0/24")],
            node_name: "node-1".into(),
        });
        // Upserting the same peer merges new allowed-ips, not duplicates.
        dev.upsert_peer(Peer {
            public_key: key(1),
            endpoint: None,
            allowed_ips: vec![cidr("10.0.1.0/24"), cidr("10.0.2.0/24")],
            node_name: "node-1".into(),
        });
        assert_eq!(dev.peer_count(), 1);
        let p = dev.peer(&key(1)).unwrap();
        assert_eq!(p.allowed_ips.len(), 2);
    }

    #[test]
    fn device_routes_by_longest_prefix() {
        let mut dev = WireguardDevice::new(key(0), 51871);
        dev.upsert_peer(Peer {
            public_key: key(1),
            endpoint: None,
            allowed_ips: vec![cidr("10.0.0.0/16")],
            node_name: "node-1".into(),
        });
        dev.upsert_peer(Peer {
            public_key: key(2),
            endpoint: None,
            allowed_ips: vec![cidr("10.0.5.0/24")],
            node_name: "node-2".into(),
        });
        // .5.x is covered by both /16 and /24 → the /24 peer wins.
        let p = dev.route(Ipv4Addr::new(10, 0, 5, 9)).unwrap();
        assert_eq!(p.node_name, "node-2");
        // .9.x only by the /16 peer.
        let p = dev.route(Ipv4Addr::new(10, 0, 9, 9)).unwrap();
        assert_eq!(p.node_name, "node-1");
        // Outside any allowed-ip → no tunnel.
        assert!(dev.route(Ipv4Addr::new(192, 168, 0, 1)).is_none());
    }

    #[test]
    fn device_remove_peer() {
        let mut dev = WireguardDevice::new(key(0), 51871);
        dev.upsert_peer(Peer {
            public_key: key(1),
            endpoint: None,
            allowed_ips: vec![cidr("10.0.1.0/24")],
            node_name: "node-1".into(),
        });
        assert!(dev.remove_peer(&key(1)));
        assert!(!dev.remove_peer(&key(1)));
        assert_eq!(dev.peer_count(), 0);
    }

    #[test]
    fn ipsec_spi_rotation_skips_zero_and_wraps() {
        let mut st = IpsecState::new();
        assert_eq!(st.current_spi(), 1);
        assert_eq!(st.prev_spi(), None);
        assert_eq!(st.rotate(), 2);
        assert_eq!(st.prev_spi(), Some(1));
        // Both current and previous are accepted on the decrypt path.
        assert!(st.accepts(2));
        assert!(st.accepts(1));
        assert!(!st.accepts(3));
        // Walk up to 15, then wrap to 1 (SPI 0 is reserved/skipped).
        for _ in 0..13 {
            st.rotate();
        }
        assert_eq!(st.current_spi(), 15);
        assert_eq!(st.rotate(), 1);
        assert_ne!(st.current_spi(), 0);
    }

    #[test]
    fn pqc_param_sizes_match_fips() {
        // FIPS-203 ML-KEM-768.
        assert_eq!(ML_KEM_768_PUBLIC_KEY, 1184);
        assert_eq!(ML_KEM_768_CIPHERTEXT, 1088);
        assert_eq!(ML_KEM_768_SHARED, 32);
        // FIPS-204 ML-DSA-65.
        assert_eq!(ML_DSA_65_PUBLIC_KEY, 1952);
        assert_eq!(ML_DSA_65_SIGNATURE, 3309);
    }

    #[test]
    fn hybrid_combiner_is_deterministic_and_input_sensitive() {
        let classical = [7u8; 32];
        let pq = [9u8; 32];
        let k1 = combine(&classical, &pq);
        let k2 = combine(&classical, &pq);
        assert_eq!(k1, k2, "same inputs → same session key");
        assert_eq!(k1.len(), 32);
        // Changing either secret changes the derived key.
        assert_ne!(k1, combine(&classical, &[10u8; 32]));
        assert_ne!(k1, combine(&[8u8; 32], &pq));
        // Domain separation: classical||pq differs from pq||classical.
        assert_ne!(combine(&classical, &pq), combine(&pq, &classical));
    }

    /// A deterministic test double for the KEM trait — exercises the
    /// handshake's structure without standing in real lattice crypto.
    struct StubKem;
    impl Kem for StubKem {
        fn suite(&self) -> &'static str {
            "stub"
        }
        fn encapsulate(&self, pk: &[u8]) -> (Vec<u8>, [u8; 32]) {
            // ct echoes pk; ss is a fixed transform both sides can recompute.
            let mut ss = [0u8; 32];
            for (i, s) in ss.iter_mut().enumerate() {
                *s = pk.get(i).copied().unwrap_or(0) ^ 0x5a;
            }
            (pk.to_vec(), ss)
        }
        fn decapsulate(&self, _sk: &[u8], ct: &[u8]) -> [u8; 32] {
            let mut ss = [0u8; 32];
            for (i, s) in ss.iter_mut().enumerate() {
                *s = ct.get(i).copied().unwrap_or(0) ^ 0x5a;
            }
            ss
        }
    }

    #[test]
    fn hybrid_handshake_both_sides_agree() {
        let kem = StubKem;
        let initiator_pk = [3u8; 32];
        let classical_shared = [1u8; 32];

        // Initiator encapsulates against responder's PQ pubkey.
        let (ct, pq_ss_init) = kem.encapsulate(&initiator_pk);
        // Responder decapsulates → same PQ shared secret.
        let pq_ss_resp = kem.decapsulate(&[0u8; 32], &ct);
        assert_eq!(pq_ss_init, pq_ss_resp);

        // Both derive the hybrid session key identically.
        let session_init = combine(&classical_shared, &pq_ss_init);
        let session_resp = combine(&classical_shared, &pq_ss_resp);
        assert_eq!(session_init, session_resp);
    }

    #[test]
    fn cipher_suite_reports_pqc() {
        assert!(CipherSuite::MlKem768X25519.is_post_quantum());
        assert!(!CipherSuite::X25519.is_post_quantum());
    }
}
