//! ztunnel — node-local L4 mTLS proxy.
//!
//! Mirrors `istio/ztunnel` (release-1.29) — the per-node Rust proxy that
//! terminates HBONE on the inbound side and originates it outbound. This
//! module models the inbound state machine so it can be unit-tested without
//! sockets:
//!
//! ```text
//! Idle ─accept─▶ TlsHandshake ─ok─▶ HboneRecv ─authz─▶ Established
//!                                                  └─deny─▶ Denied
//! ```
//!
//! `Established` exposes a byte-counting bidirectional pipe so tests can
//! drive both directions without networking.

use crate::ambient::hbone::{authorise, parse_request, HboneError, HboneRequest};
use crate::ambient::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};

/// State of a single inbound HBONE connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZtunnelState {
    Idle,
    TlsHandshake,
    HboneRecv,
    Established { tunnel: HboneRequest },
    Denied { reason: String },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ZtunnelError {
    #[error("invalid transition from {from:?} to {to}")]
    BadTransition { from: ZtunnelState, to: &'static str },
    #[error("hbone error: {0}")]
    Hbone(#[from] HboneError),
    #[error("peer mTLS identity {peer} did not match expected SPIFFE prefix {expected}")]
    PeerIdentityMismatch { peer: String, expected: String },
}

/// One inbound HBONE connection. Carries enough state to drive a step-wise
/// state machine and to count bytes through the established tunnel.
#[derive(Debug, Clone)]
pub struct ZtunnelConn {
    pub state: ZtunnelState,
    pub tenant: TenantId,
    /// SPIFFE prefix that the peer mTLS cert must start with — e.g.
    /// `spiffe://cluster.local/ns/acme/`.
    pub expected_peer_prefix: String,
    pub peer_identity: Option<String>,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl ZtunnelConn {
    pub fn new(tenant: TenantId, expected_peer_prefix: impl Into<String>) -> Self {
        Self {
            state: ZtunnelState::Idle,
            tenant,
            expected_peer_prefix: expected_peer_prefix.into(),
            peer_identity: None,
            bytes_in: 0,
            bytes_out: 0,
        }
    }

    pub fn accept(&mut self) -> Result<(), ZtunnelError> {
        if self.state != ZtunnelState::Idle {
            return Err(ZtunnelError::BadTransition {
                from: self.state.clone(),
                to: "TlsHandshake",
            });
        }
        self.state = ZtunnelState::TlsHandshake;
        Ok(())
    }

    /// Called when the underlying mTLS handshake reports the peer's SPIFFE id.
    /// The peer id must start with the expected prefix (tenant isolation).
    pub fn complete_handshake(&mut self, peer_identity: impl Into<String>) -> Result<(), ZtunnelError> {
        if self.state != ZtunnelState::TlsHandshake {
            return Err(ZtunnelError::BadTransition {
                from: self.state.clone(),
                to: "HboneRecv",
            });
        }
        let peer = peer_identity.into();
        if !peer.starts_with(&self.expected_peer_prefix) {
            self.state = ZtunnelState::Denied {
                reason: format!("peer {peer} not in {}", self.expected_peer_prefix),
            };
            return Err(ZtunnelError::PeerIdentityMismatch {
                peer,
                expected: self.expected_peer_prefix.clone(),
            });
        }
        self.peer_identity = Some(peer);
        self.state = ZtunnelState::HboneRecv;
        Ok(())
    }

    /// Receive HBONE headers, parse, and authorise against the connection's
    /// tenant.
    pub fn receive_hbone(&mut self, headers: &[(&str, &str)]) -> Result<(), ZtunnelError> {
        if self.state != ZtunnelState::HboneRecv {
            return Err(ZtunnelError::BadTransition {
                from: self.state.clone(),
                to: "Established",
            });
        }
        let req = parse_request(headers)?;
        authorise(&req, &self.tenant)?;
        self.state = ZtunnelState::Established { tunnel: req };
        Ok(())
    }

    /// Push a single inbound chunk through the tunnel.
    pub fn push_inbound(&mut self, n: u64) -> Result<(), ZtunnelError> {
        match &self.state {
            ZtunnelState::Established { .. } => {
                self.bytes_in = self.bytes_in.saturating_add(n);
                Ok(())
            }
            other => Err(ZtunnelError::BadTransition {
                from: other.clone(),
                to: "push_inbound",
            }),
        }
    }

    /// Push a single outbound chunk through the tunnel.
    pub fn push_outbound(&mut self, n: u64) -> Result<(), ZtunnelError> {
        match &self.state {
            ZtunnelState::Established { .. } => {
                self.bytes_out = self.bytes_out.saturating_add(n);
                Ok(())
            }
            other => Err(ZtunnelError::BadTransition {
                from: other.clone(),
                to: "push_outbound",
            }),
        }
    }

    pub fn is_established(&self) -> bool {
        matches!(self.state, ZtunnelState::Established { .. })
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::ext(
    "istio/ztunnel",
    "src/proxy/inbound.rs",
    "Inbound::handle",
    "release-1.29",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ambient_test_ctx;

    fn conn(tenant: &str) -> ZtunnelConn {
        ZtunnelConn::new(TenantId::new(tenant).expect("test fixture"), format!("spiffe://cluster.local/ns/{tenant}/"))
    }

    fn good_headers() -> Vec<(&'static str, &'static str)> {
        vec![
            (":method", "CONNECT"),
            (":authority", "10.0.0.42:8080"),
            (":path", "/"),
            ("baggage", "tenant=acme"),
        ]
    }

    #[test]
    fn happy_path_drives_idle_to_established() {
        let (_cite, _t) = ambient_test_ctx!(
            ext: "istio/ztunnel",
            "release-1.29",
            "src/proxy/inbound.rs",
            "Inbound::handle",
            "acme"
        );
        let mut c = conn("acme");
        c.accept().unwrap();
        c.complete_handshake("spiffe://cluster.local/ns/acme/sa/web").unwrap();
        c.receive_hbone(&good_headers()).unwrap();
        assert!(c.is_established());
        c.push_inbound(1024).unwrap();
        c.push_outbound(2048).unwrap();
        assert_eq!(c.bytes_in, 1024);
        assert_eq!(c.bytes_out, 2048);
    }

    #[test]
    fn handshake_outside_tenant_namespace_is_refused() {
        let (_cite, _t) = ambient_test_ctx!(
            ext: "istio/ztunnel",
            "release-1.29",
            "src/proxy/inbound.rs",
            "verify_peer",
            "tenant-zt-cross-ns"
        );
        let mut c = conn("acme");
        c.accept().unwrap();
        let err = c
            .complete_handshake("spiffe://cluster.local/ns/evil/sa/x")
            .unwrap_err();
        assert!(matches!(err, ZtunnelError::PeerIdentityMismatch { .. }));
        assert!(matches!(c.state, ZtunnelState::Denied { .. }));
    }

    #[test]
    fn cannot_receive_hbone_before_handshake() {
        let (_cite, _t) = ambient_test_ctx!(
            ext: "istio/ztunnel",
            "release-1.29",
            "src/proxy/inbound.rs",
            "Inbound::handle",
            "tenant-zt-order"
        );
        let mut c = conn("acme");
        c.accept().unwrap();
        // No handshake → receive_hbone in TlsHandshake state.
        let err = c.receive_hbone(&good_headers()).unwrap_err();
        assert!(matches!(err, ZtunnelError::BadTransition { .. }));
    }

    #[test]
    fn baggage_tenant_must_match_connection_tenant() {
        let (_cite, _t) = ambient_test_ctx!(
            ext: "istio/ztunnel",
            "release-1.29",
            "src/proxy/inbound.rs",
            "authorise",
            "tenant-zt-baggage"
        );
        let mut c = conn("acme");
        c.accept().unwrap();
        c.complete_handshake("spiffe://cluster.local/ns/acme/sa/web").unwrap();
        // Baggage header is for a different tenant.
        let bad = vec![
            (":method", "CONNECT"),
            (":authority", "10.0.0.42:8080"),
            (":path", "/"),
            ("baggage", "tenant=evil"),
        ];
        let err = c.receive_hbone(&bad).unwrap_err();
        assert!(matches!(err, ZtunnelError::Hbone(HboneError::TenantDenied { .. })));
    }

    #[test]
    fn pushes_only_succeed_when_established() {
        let (_cite, _t) = ambient_test_ctx!(
            ext: "istio/ztunnel",
            "release-1.29",
            "src/proxy/inbound.rs",
            "copy_bidirectional",
            "tenant-zt-pushes"
        );
        let mut c = conn("acme");
        let err = c.push_inbound(100).unwrap_err();
        assert!(matches!(err, ZtunnelError::BadTransition { .. }));
        c.accept().unwrap();
        c.complete_handshake("spiffe://cluster.local/ns/acme/sa/web").unwrap();
        c.receive_hbone(&good_headers()).unwrap();
        assert!(c.push_inbound(100).is_ok());
        assert!(c.push_outbound(50).is_ok());
    }
}
