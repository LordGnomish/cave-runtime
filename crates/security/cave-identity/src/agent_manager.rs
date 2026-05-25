// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). Manager loop + SDS stub
// line-ported from pkg/agent/manager/manager.go +
// pkg/agent/endpoints/sdsv3/handler.go.
//
//! Agent-side SVID manager + SDS v3 envoy compatibility stub.
//!
//! Drives X.509-SVID rotation per registration entry, keeps a cache, and
//! exposes Envoy SDS v3 [`SecretFetch`] semantics. Real wire SDS goes over
//! gRPC and is intentionally a Charter-scope_cut — the stub returns the
//! data Envoy would otherwise stream.

use crate::error::{IdentityError, Result};
use crate::models::{RegistrationEntry, SpiffeId, X509Svid};
use crate::server_ca::ServerCa;
use crate::x509_svid;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Per-agent SVID cache entry.
#[derive(Debug, Clone)]
pub struct AgentSvidEntry {
    pub entry: RegistrationEntry,
    pub current_svid: X509Svid,
    pub last_rotated_at: DateTime<Utc>,
}

/// Manager state — collects per-entry SVIDs and exposes refresh ticks.
pub struct AgentManager {
    cache: Arc<DashMap<String, AgentSvidEntry>>,
    ca: Arc<ServerCa>,
}

impl AgentManager {
    pub fn new(ca: Arc<ServerCa>) -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            ca,
        }
    }

    /// Bootstrap: issue an initial SVID for every entry.
    pub fn bootstrap(&self, entries: &[RegistrationEntry]) -> Result<()> {
        for e in entries {
            let svid = x509_svid::issue(&self.ca, e)?;
            self.cache.insert(
                e.id.clone(),
                AgentSvidEntry {
                    entry: e.clone(),
                    current_svid: svid,
                    last_rotated_at: Utc::now(),
                },
            );
        }
        Ok(())
    }

    /// Run a single rotation pass: replace SVIDs that crossed half-life.
    pub fn tick(&self) -> Result<usize> {
        let mut rotated = 0usize;
        for mut kv in self.cache.iter_mut() {
            let cached = kv.value().clone();
            if let Some(new) = x509_svid::rotate_if_needed(&self.ca, &cached.entry, &cached.current_svid)? {
                kv.current_svid = new;
                kv.last_rotated_at = Utc::now();
                rotated += 1;
            }
        }
        Ok(rotated)
    }

    /// SDS-v3 compatibility: fetch a secret by spiffe id.
    pub fn sds_fetch(&self, spiffe_id: &SpiffeId) -> Result<SdsSecret> {
        let entry = self
            .cache
            .iter()
            .find(|kv| &kv.value().entry.spiffe_id == spiffe_id)
            .map(|kv| kv.value().clone())
            .ok_or_else(|| {
                IdentityError::EntryNotFound(format!("no svid for {}", spiffe_id))
            })?;
        Ok(SdsSecret {
            name: entry.entry.spiffe_id.as_str().to_string(),
            cert_chain_pem: pem_chain(&entry.current_svid),
            private_key_pem: pem_priv(&entry.current_svid),
            trust_bundle_pem: pem_trust_bundle(&self.ca),
        })
    }

    pub fn current(&self, entry_id: &str) -> Option<AgentSvidEntry> {
        self.cache.get(entry_id).map(|e| e.value().clone())
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

/// SDS-v3 Secret resource — `envoy.extensions.transport_sockets.tls.v3.Secret`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdsSecret {
    pub name: String,
    pub cert_chain_pem: String,
    pub private_key_pem: String,
    pub trust_bundle_pem: String,
}

fn pem_chain(svid: &X509Svid) -> String {
    let mut out = String::new();
    out.push_str(&pem_block("CERTIFICATE", &svid.leaf_der));
    for d in &svid.intermediates_der {
        out.push_str(&pem_block("CERTIFICATE", d));
    }
    out
}

fn pem_priv(svid: &X509Svid) -> String {
    pem_block("PRIVATE KEY", &svid.private_key_der)
}

fn pem_trust_bundle(ca: &ServerCa) -> String {
    let b = ca.trust_bundle();
    let mut out = String::new();
    for a in &b.x509_authorities {
        out.push_str(&pem_block("CERTIFICATE", &a.asn1_der));
    }
    out
}

fn pem_block(label: &str, der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut body = String::new();
    for chunk in b64.as_bytes().chunks(64) {
        body.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        body.push('\n');
    }
    format!(
        "-----BEGIN {label}-----\n{body}-----END {label}-----\n",
        label = label,
        body = body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TrustDomain;
    use crate::server_ca::RotationParams;
    use chrono::Duration;

    fn fresh_ca() -> Arc<ServerCa> {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        Arc::new(ca)
    }

    fn entry(id: &str) -> RegistrationEntry {
        RegistrationEntry {
            id: id.into(),
            spiffe_id: SpiffeId::new(format!("spiffe://example.org/svc/{}", id)),
            parent_id: SpiffeId::new("spiffe://example.org/spire/agent/k8s/n"),
            x509_svid_ttl_seconds: 3600,
            ..Default::default()
        }
    }

    #[test]
    fn bootstrap_caches_svids() {
        let mgr = AgentManager::new(fresh_ca());
        mgr.bootstrap(&[entry("a"), entry("b")]).unwrap();
        assert_eq!(mgr.len(), 2);
        assert!(mgr.current("a").is_some());
    }

    #[test]
    fn tick_rotates_expiring_svid() {
        let mgr = AgentManager::new(fresh_ca());
        mgr.bootstrap(&[entry("a")]).unwrap();
        // Force half-life expiry
        if let Some(mut v) = mgr.cache.get_mut("a") {
            v.current_svid.expires_at = Utc::now() + Duration::seconds(60);
        }
        let rotated = mgr.tick().unwrap();
        assert!(rotated >= 1);
    }

    #[test]
    fn sds_fetch_returns_pem_blocks() {
        let mgr = AgentManager::new(fresh_ca());
        mgr.bootstrap(&[entry("a")]).unwrap();
        let s = mgr
            .sds_fetch(&SpiffeId::new("spiffe://example.org/svc/a"))
            .unwrap();
        assert!(s.cert_chain_pem.contains("BEGIN CERTIFICATE"));
        assert!(s.private_key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(s.trust_bundle_pem.contains("BEGIN CERTIFICATE"));
        assert_eq!(s.name, "spiffe://example.org/svc/a");
    }

    #[test]
    fn sds_fetch_unknown_id() {
        let mgr = AgentManager::new(fresh_ca());
        mgr.bootstrap(&[entry("a")]).unwrap();
        assert!(mgr
            .sds_fetch(&SpiffeId::new("spiffe://example.org/missing"))
            .is_err());
    }
}
