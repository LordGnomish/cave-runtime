// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Native TLS listener configuration.
//!
//! Ports the configuration + parse half of `src/tls.c`. Upstream's
//! tls.c wires OpenSSL-style cert/key loading into the listener
//! bring-up; this module owns the equivalent surface in Rust:
//!
//! * `TlsListenerConfig` — the operator-curated cert/key/CA paths
//!   plus client-auth and ALPN preferences (the same shape as the
//!   `tls-cert-file` / `tls-key-file` / `tls-ca-cert-file` /
//!   `tls-auth-clients` directives).
//! * `load_cert_chain` / `load_private_key` — read PEM blobs from
//!   disk into the typed forms a rustls server config expects.
//! * `validate` — verifies the cert + key parse cleanly, the chain
//!   is non-empty, the ALPN list contains only protocols we
//!   support, and the listen port doesn't clash with the plain
//!   port. Cryptographic key-match (does this key actually sign
//!   this cert?) is delegated to the rustls server-config build
//!   step the caller drives.
//!
//! The actual TLS handshake / connection accept loop lives in
//! [`crate::server`]; this module is the configuration surface only.

use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys, ec_private_keys};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum TlsConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("certificate file is empty: {0}")]
    EmptyCertChain(PathBuf),
    #[error("private key file has no key block: {0}")]
    NoPrivateKey(PathBuf),
    #[error("certificate file contains malformed PEM: {0}")]
    MalformedCert(PathBuf),
    #[error("private key file contains malformed PEM: {0}")]
    MalformedKey(PathBuf),
    #[error("ALPN protocol unsupported: {0}")]
    UnsupportedAlpn(String),
    #[error("TLS port {tls} clashes with plain port {plain}")]
    PortConflict { tls: u16, plain: u16 },
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// What we ask of clients during the TLS handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAuth {
    /// Plain TLS — no client cert.
    No,
    /// Optional client cert (mTLS), validation soft.
    Optional,
    /// Required client cert (mTLS), validation strict.
    Required,
}

impl ClientAuth {
    /// Parses upstream's `tls-auth-clients` directive (`yes`/`no`/`optional`).
    pub fn parse(s: &str) -> Result<Self, TlsConfigError> {
        match s.to_ascii_lowercase().as_str() {
            "yes" | "true" | "required" => Ok(ClientAuth::Required),
            "no" | "false" => Ok(ClientAuth::No),
            "optional" => Ok(ClientAuth::Optional),
            other => Err(TlsConfigError::Invalid(format!(
                "tls-auth-clients: unknown value {other:?}"
            ))),
        }
    }
}

/// Configuration for a TLS listener. Mirrors the redis.conf
/// `tls-*` directive surface.
#[derive(Debug, Clone)]
pub struct TlsListenerConfig {
    pub bind_addr: String,
    pub tls_port: u16,
    /// Plain RESP port — must be different from tls_port.
    pub plain_port: Option<u16>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    /// Optional CA bundle for client cert verification.
    pub ca_path: Option<PathBuf>,
    pub client_auth: ClientAuth,
    /// ALPN protocols offered in preference order. Empty disables ALPN.
    pub alpn_protocols: Vec<String>,
    /// Allowed TLS protocol versions (e.g. `["TLSv1.2", "TLSv1.3"]`).
    pub allowed_protocols: Vec<String>,
}

impl TlsListenerConfig {
    pub fn new(cert: impl Into<PathBuf>, key: impl Into<PathBuf>) -> Self {
        Self {
            bind_addr: "0.0.0.0".into(),
            tls_port: 6380,
            plain_port: Some(6379),
            cert_path: cert.into(),
            key_path: key.into(),
            ca_path: None,
            client_auth: ClientAuth::No,
            alpn_protocols: Vec::new(),
            allowed_protocols: vec!["TLSv1.2".into(), "TLSv1.3".into()],
        }
    }

    /// Verify configuration values without contacting the disk.
    pub fn validate(&self) -> Result<(), TlsConfigError> {
        if let Some(p) = self.plain_port {
            if p == self.tls_port {
                return Err(TlsConfigError::PortConflict {
                    tls: self.tls_port,
                    plain: p,
                });
            }
        }
        for a in &self.alpn_protocols {
            if !KNOWN_ALPN.iter().any(|k| k == a) {
                return Err(TlsConfigError::UnsupportedAlpn(a.clone()));
            }
        }
        for p in &self.allowed_protocols {
            if !KNOWN_TLS_PROTOCOLS.iter().any(|k| k == p) {
                return Err(TlsConfigError::Invalid(format!(
                    "allowed protocol unsupported: {p}"
                )));
            }
        }
        if self.client_auth != ClientAuth::No && self.ca_path.is_none() {
            return Err(TlsConfigError::Invalid(
                "client auth enabled but no CA bundle provided".into(),
            ));
        }
        Ok(())
    }

    /// Validate config and load every PEM blob from disk. Returns
    /// the parsed material packed into [`TlsLoadedMaterial`] so the
    /// caller can hand it to a rustls server-config builder.
    pub fn load(&self) -> Result<TlsLoadedMaterial, TlsConfigError> {
        self.validate()?;
        let cert_chain = load_cert_chain(&self.cert_path)?;
        let private_key = load_private_key(&self.key_path)?;
        let ca_certs = match &self.ca_path {
            Some(p) => Some(load_cert_chain(p)?),
            None => None,
        };
        Ok(TlsLoadedMaterial {
            cert_chain,
            private_key,
            ca_certs,
            alpn_protocols: self.alpn_protocols.clone(),
            client_auth: self.client_auth,
        })
    }
}

/// Whitelist of ALPN protocol identifiers we permit.
pub const KNOWN_ALPN: &[&str] = &["resp3", "resp2", "http/1.1", "h2"];

pub const KNOWN_TLS_PROTOCOLS: &[&str] = &["TLSv1.2", "TLSv1.3"];

/// Result of loading a `TlsListenerConfig` from disk.
#[derive(Debug, Clone)]
pub struct TlsLoadedMaterial {
    pub cert_chain: Vec<Vec<u8>>,
    pub private_key: Vec<u8>,
    pub ca_certs: Option<Vec<Vec<u8>>>,
    pub alpn_protocols: Vec<String>,
    pub client_auth: ClientAuth,
}

impl TlsLoadedMaterial {
    pub fn leaf_certificate(&self) -> &[u8] {
        &self.cert_chain[0]
    }
    pub fn chain_len(&self) -> usize {
        self.cert_chain.len()
    }
}

pub fn load_cert_chain(path: &Path) -> Result<Vec<Vec<u8>>, TlsConfigError> {
    let f = File::open(path)?;
    let mut rdr = BufReader::new(f);
    let chain: Vec<_> = certs(&mut rdr)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| TlsConfigError::MalformedCert(path.to_owned()))?;
    if chain.is_empty() {
        return Err(TlsConfigError::EmptyCertChain(path.to_owned()));
    }
    Ok(chain.into_iter().map(|c| c.as_ref().to_vec()).collect())
}

pub fn load_private_key(path: &Path) -> Result<Vec<u8>, TlsConfigError> {
    // Try PKCS#8 first (rustls preferred), fall back to PKCS#1 RSA,
    // then SEC1 EC. Match the order rustls-pemfile docs suggest.
    let key_bytes = std::fs::read(path)?;

    if let Some(k) = pkcs8_private_keys(&mut key_bytes.as_slice())
        .next()
        .transpose()
        .map_err(|_| TlsConfigError::MalformedKey(path.to_owned()))?
    {
        return Ok(k.secret_pkcs8_der().to_vec());
    }
    if let Some(k) = rsa_private_keys(&mut key_bytes.as_slice())
        .next()
        .transpose()
        .map_err(|_| TlsConfigError::MalformedKey(path.to_owned()))?
    {
        return Ok(k.secret_pkcs1_der().to_vec());
    }
    if let Some(k) = ec_private_keys(&mut key_bytes.as_slice())
        .next()
        .transpose()
        .map_err(|_| TlsConfigError::MalformedKey(path.to_owned()))?
    {
        return Ok(k.secret_sec1_der().to_vec());
    }
    Err(TlsConfigError::NoPrivateKey(path.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};
    use std::io::Write;
    use tempfile::TempDir;

    fn write_self_signed(dir: &TempDir) -> (PathBuf, PathBuf) {
        let key = KeyPair::generate().unwrap();
        let cert = CertificateParams::new(vec!["cave-cache.test".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let cert_path = dir.path().join("server.crt");
        let key_path = dir.path().join("server.key");
        std::fs::File::create(&cert_path)
            .unwrap()
            .write_all(cert.pem().as_bytes())
            .unwrap();
        std::fs::File::create(&key_path)
            .unwrap()
            .write_all(key.serialize_pem().as_bytes())
            .unwrap();
        (cert_path, key_path)
    }

    #[test]
    fn client_auth_parse_round_trips() {
        assert_eq!(ClientAuth::parse("yes").unwrap(), ClientAuth::Required);
        assert_eq!(ClientAuth::parse("no").unwrap(), ClientAuth::No);
        assert_eq!(ClientAuth::parse("optional").unwrap(), ClientAuth::Optional);
        assert!(ClientAuth::parse("garbage").is_err());
    }

    #[test]
    fn config_validate_passes_default() {
        let cfg = TlsListenerConfig::new("crt", "key");
        cfg.validate().unwrap();
    }

    #[test]
    fn config_rejects_port_clash() {
        let mut cfg = TlsListenerConfig::new("crt", "key");
        cfg.plain_port = Some(6380);
        cfg.tls_port = 6380;
        assert!(matches!(cfg.validate().unwrap_err(), TlsConfigError::PortConflict { .. }));
    }

    #[test]
    fn config_rejects_unknown_alpn() {
        let mut cfg = TlsListenerConfig::new("crt", "key");
        cfg.alpn_protocols = vec!["resp99".into()];
        assert!(matches!(cfg.validate().unwrap_err(), TlsConfigError::UnsupportedAlpn(_)));
    }

    #[test]
    fn config_rejects_unknown_tls_version() {
        let mut cfg = TlsListenerConfig::new("crt", "key");
        cfg.allowed_protocols = vec!["TLSv1.0".into()];
        assert!(matches!(cfg.validate().unwrap_err(), TlsConfigError::Invalid(_)));
    }

    #[test]
    fn config_requires_ca_when_client_auth_enabled() {
        let mut cfg = TlsListenerConfig::new("crt", "key");
        cfg.client_auth = ClientAuth::Required;
        assert!(matches!(cfg.validate().unwrap_err(), TlsConfigError::Invalid(_)));
    }

    #[test]
    fn load_cert_chain_round_trips_self_signed() {
        let dir = TempDir::new().unwrap();
        let (cert, _key) = write_self_signed(&dir);
        let chain = load_cert_chain(&cert).unwrap();
        assert_eq!(chain.len(), 1);
        assert!(!chain[0].is_empty());
    }

    #[test]
    fn load_private_key_round_trips_pkcs8() {
        let dir = TempDir::new().unwrap();
        let (_cert, key) = write_self_signed(&dir);
        let blob = load_private_key(&key).unwrap();
        assert!(!blob.is_empty());
    }

    #[test]
    fn load_cert_chain_rejects_empty_file() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("empty.pem");
        std::fs::write(&p, b"").unwrap();
        assert!(matches!(load_cert_chain(&p).unwrap_err(), TlsConfigError::EmptyCertChain(_)));
    }

    #[test]
    fn load_cert_chain_rejects_file_with_no_certificate_block() {
        // No BEGIN CERTIFICATE block anywhere — empty chain.
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("nocert.pem");
        std::fs::write(&p, b"# just a comment, no PEM blocks at all\n").unwrap();
        assert!(matches!(load_cert_chain(&p).unwrap_err(), TlsConfigError::EmptyCertChain(_)));
    }

    #[test]
    fn load_private_key_rejects_no_key_block() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("no-key.pem");
        std::fs::write(&p, b"-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n").unwrap();
        assert!(matches!(load_private_key(&p).unwrap_err(), TlsConfigError::NoPrivateKey(_)));
    }

    #[test]
    fn end_to_end_load_returns_material() {
        let dir = TempDir::new().unwrap();
        let (cert, key) = write_self_signed(&dir);
        let cfg = TlsListenerConfig::new(cert, key);
        let mat = cfg.load().unwrap();
        assert_eq!(mat.chain_len(), 1);
        assert!(!mat.leaf_certificate().is_empty());
        assert!(!mat.private_key.is_empty());
        assert_eq!(mat.client_auth, ClientAuth::No);
    }

    #[test]
    fn end_to_end_load_propagates_validation_errors() {
        let dir = TempDir::new().unwrap();
        let (cert, key) = write_self_signed(&dir);
        let mut cfg = TlsListenerConfig::new(cert, key);
        cfg.alpn_protocols = vec!["bogus".into()];
        assert!(matches!(cfg.load().unwrap_err(), TlsConfigError::UnsupportedAlpn(_)));
    }

    #[test]
    fn known_alpn_list_contains_resp_variants() {
        assert!(KNOWN_ALPN.contains(&"resp3"));
        assert!(KNOWN_ALPN.contains(&"resp2"));
    }
}
