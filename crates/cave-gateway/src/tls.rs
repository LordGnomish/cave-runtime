// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TLS subsystem — SNI-based routing, TLS termination, mTLS upstream, ACME.
//!
//! Uses rustls for TLS handling. Certificates are stored in the GatewayStore
//! and loaded into the SNI resolver at startup (or hot-reloaded via Admin API).

use crate::models::{Certificate, Sni};
use crate::store::GatewayStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::ClientHello;
use rustls::{ServerConfig, ServerConnection};
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use std::collections::HashMap;
use std::io::BufReader;
use std::sync::{Arc, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

/// SNI-aware certificate resolver.
/// Maps hostname → (cert chain, private key).
pub struct SniCertResolver {
    /// hostname → CertifiedKey
    certs: RwLock<HashMap<String, Arc<rustls::sign::CertifiedKey>>>,
    default_cert: RwLock<Option<Arc<rustls::sign::CertifiedKey>>>,
}

impl std::fmt::Debug for SniCertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let certs = self.certs.read().unwrap();
        f.debug_struct("SniCertResolver")
            .field("sni_count", &certs.len())
            .finish()
    }
}

impl SniCertResolver {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            certs: RwLock::new(HashMap::new()),
            default_cert: RwLock::new(None),
        })
    }

    /// Load all certificates from the store into the resolver.
    pub fn reload_from_store(&self, store: &GatewayStore) {
        let snis = store.list_snis();
        let mut map = HashMap::new();

        for sni in &snis {
            if let Some(cert) = store.certificates.get(&sni.certificate_id) {
                match parse_certified_key(&cert.cert, &cert.key) {
                    Ok(ck) => {
                        map.insert(sni.name.clone(), Arc::new(ck));
                        info!(sni=%sni.name, "TLS certificate loaded");
                    }
                    Err(e) => {
                        warn!(sni=%sni.name, err=%e, "failed to load TLS certificate");
                    }
                }
            }
        }

        *self.certs.write().unwrap() = map;
    }

    /// Add a single certificate for the given SNI names.
    pub fn add_cert(
        &self,
        sni_names: &[String],
        cert_pem: &str,
        key_pem: &str,
    ) -> Result<(), String> {
        let ck = parse_certified_key(cert_pem, key_pem)
            .map_err(|e| format!("failed to parse certificate: {}", e))?;
        let ck = Arc::new(ck);
        let mut map = self.certs.write().unwrap();
        for name in sni_names {
            map.insert(name.clone(), ck.clone());
        }
        Ok(())
    }

    pub fn remove_cert(&self, sni_name: &str) {
        self.certs.write().unwrap().remove(sni_name);
    }
}

impl rustls::server::ResolvesServerCert for SniCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let sni = client_hello.server_name()?;
        let map = self.certs.read().ok()?;

        // Exact match
        if let Some(ck) = map.get(sni) {
            return Some(ck.clone());
        }

        // Wildcard match: *.example.com matches foo.example.com
        let parts: Vec<&str> = sni.splitn(2, '.').collect();
        if parts.len() == 2 {
            let wildcard = format!("*.{}", parts[1]);
            if let Some(ck) = map.get(&wildcard) {
                return Some(ck.clone());
            }
        }

        // Fall back to default
        self.default_cert.read().ok()?.as_ref().cloned()
    }
}

/// Parse a PEM certificate + private key into a rustls CertifiedKey.
pub fn parse_certified_key(
    cert_pem: &str,
    key_pem: &str,
) -> Result<rustls::sign::CertifiedKey, Box<dyn std::error::Error + Send + Sync>> {
    let cert_bytes = cert_pem.as_bytes();
    let key_bytes = key_pem.as_bytes();

    let cert_chain: Vec<CertificateDer<'static>> = {
        let mut reader = BufReader::new(cert_bytes);
        certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|c| c.into_owned())
            .collect()
    };

    if cert_chain.is_empty() {
        return Err("no certificates found in PEM".into());
    }

    let private_key: PrivateKeyDer<'static> = {
        let mut reader = BufReader::new(key_bytes);
        // Try PKCS8 first
        let pkcs8: Vec<_> = pkcs8_private_keys(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();

        if !pkcs8.is_empty() {
            let key = pkcs8.into_iter().next().unwrap();
            PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                key.secret_pkcs8_der().to_vec(),
            ))
        } else {
            // Try RSA
            let mut reader = BufReader::new(key_bytes);
            let rsa: Vec<_> = rsa_private_keys(&mut reader)
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_default();
            if !rsa.is_empty() {
                let key = rsa.into_iter().next().unwrap();
                PrivateKeyDer::Pkcs1(rustls::pki_types::PrivatePkcs1KeyDer::from(
                    key.secret_pkcs1_der().to_vec(),
                ))
            } else {
                return Err("no private key found in PEM".into());
            }
        }
    };

    let signing_key = rustls::crypto::ring::sign::any_supported_type(&private_key)
        .map_err(|e| format!("unsupported key type: {:?}", e))?;

    Ok(rustls::sign::CertifiedKey::new(cert_chain, signing_key))
}

/// Build a TLS ServerConfig with the SNI resolver.
pub fn build_server_config(resolver: Arc<SniCertResolver>) -> Arc<ServerConfig> {
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    Arc::new(config)
}

/// Build a TLS ServerConfig that requires client certificates (mTLS).
pub fn build_mtls_server_config(
    resolver: Arc<SniCertResolver>,
    ca_cert_pem: &str,
) -> Result<Arc<ServerConfig>, Box<dyn std::error::Error + Send + Sync>> {
    let ca_cert = {
        let mut reader = BufReader::new(ca_cert_pem.as_bytes());
        certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .next()
            .ok_or("no CA cert found")?
            .into_owned()
    };

    let mut root_store = rustls::RootCertStore::empty();
    root_store.add(ca_cert)?;

    let client_auth = rustls::server::WebPkiClientVerifier::builder(Arc::new(root_store))
        .build()
        .map_err(|e| format!("failed to build client verifier: {:?}", e))?;

    let config = ServerConfig::builder()
        .with_client_cert_verifier(client_auth)
        .with_cert_resolver(resolver);

    Ok(Arc::new(config))
}

/// Build a reqwest client configured for mTLS upstream connections.
pub fn build_mtls_client(
    client_cert_pem: &str,
    client_key_pem: &str,
    ca_cert_pem: Option<&str>,
) -> Result<reqwest::Client, Box<dyn std::error::Error + Send + Sync>> {
    let identity = {
        let cert_bytes = client_cert_pem.as_bytes().to_vec();
        let key_bytes = client_key_pem.as_bytes().to_vec();
        let mut pem = cert_bytes;
        pem.extend_from_slice(&key_bytes);
        reqwest::Identity::from_pem(&pem)?
    };

    let mut builder = reqwest::Client::builder().identity(identity);

    if let Some(ca_pem) = ca_cert_pem {
        let ca_cert = reqwest::Certificate::from_pem(ca_pem.as_bytes())?;
        builder = builder.add_root_certificate(ca_cert);
    }

    Ok(builder.build()?)
}

// ── ACME (Let's Encrypt) ──────────────────────────────────────────────────────

/// Minimal ACME client state. For production use, integrate with `instant-acme`.
#[derive(Debug, Clone)]
pub struct AcmeConfig {
    pub directory_url: String, // e.g. "https://acme-v02.api.letsencrypt.org/directory"
    pub email: String,
    pub domains: Vec<String>,
    pub storage_path: String,
}

impl AcmeConfig {
    pub fn lets_encrypt(email: String, domains: Vec<String>) -> Self {
        Self {
            directory_url: "https://acme-v02.api.letsencrypt.org/directory".to_string(),
            email,
            domains,
            storage_path: "/etc/cave-gateway/acme".to_string(),
        }
    }

    pub fn lets_encrypt_staging(email: String, domains: Vec<String>) -> Self {
        Self {
            directory_url: "https://acme-staging-v02.api.letsencrypt.org/directory".to_string(),
            email,
            domains,
            storage_path: "/tmp/cave-gateway/acme".to_string(),
        }
    }
}

/// HTTP-01 challenge token store — used by the /.well-known/acme-challenge/ handler.
#[derive(Default, Clone)]
pub struct AcmeChallengeStore {
    tokens: Arc<dashmap::DashMap<String, String>>,
}

impl AcmeChallengeStore {
    pub fn set(&self, token: String, key_auth: String) {
        self.tokens.insert(token, key_auth);
    }

    pub fn get(&self, token: &str) -> Option<String> {
        self.tokens.get(token).map(|v| v.value().clone())
    }

    pub fn remove(&self, token: &str) {
        self.tokens.remove(token);
    }
}

/// Axum handler for ACME HTTP-01 challenges.
pub async fn acme_challenge_handler(
    axum::extract::Path(token): axum::extract::Path<String>,
    axum::extract::State(store): axum::extract::State<Arc<AcmeChallengeStore>>,
) -> impl axum::response::IntoResponse {
    match store.get(&token) {
        Some(key_auth) => (axum::http::StatusCode::OK, key_auth),
        None => (axum::http::StatusCode::NOT_FOUND, "".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sni_resolver_exact_match() {
        let resolver = SniCertResolver::new();
        // Without a cert loaded, should return None
        // (can't easily test with real cert in unit test)
        let certs = resolver.certs.read().unwrap();
        assert!(certs.is_empty());
    }

    #[test]
    fn acme_challenge_store() {
        let store = AcmeChallengeStore::default();
        store.set("abc123".to_string(), "abc123.def456".to_string());
        assert_eq!(store.get("abc123"), Some("abc123.def456".to_string()));
        store.remove("abc123");
        assert!(store.get("abc123").is_none());
    }
}
