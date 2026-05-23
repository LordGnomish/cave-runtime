// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TLS plugin — per-server TLS material declaration.
//!
//! Upstream `plugin/tls/tls.go` declares the cert/key/CA paths that DoT,
//! DoH, and DoQ listeners will pick up. It is a setup-only directive: it
//! does not touch request handling.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::{DnsError, DnsResult},
    plugins::{Next, Plugin, QueryContext},
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TlsConfig {
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub ca_path: Option<String>,
    /// When true, presented client certs are required + verified.
    pub require_client_cert: bool,
    /// ALPN protocols advertised by DoT/DoH/DoQ listeners.
    pub alpn: Vec<String>,
}

pub struct TlsPlugin {
    config: TlsConfig,
}

impl TlsPlugin {
    pub fn new(config: TlsConfig) -> DnsResult<Self> {
        if config.cert_path.is_some() ^ config.key_path.is_some() {
            return Err(DnsError::Config(
                "tls: cert_path and key_path must be set together".into(),
            ));
        }
        Ok(Self { config })
    }

    pub fn cert_path(&self) -> Option<&str> {
        self.config.cert_path.as_deref()
    }

    pub fn key_path(&self) -> Option<&str> {
        self.config.key_path.as_deref()
    }

    pub fn ca_path(&self) -> Option<&str> {
        self.config.ca_path.as_deref()
    }

    pub fn require_client_cert(&self) -> bool {
        self.config.require_client_cert
    }

    pub fn alpn(&self) -> &[String] {
        &self.config.alpn
    }

    /// Returns true when both cert+key are wired — listeners use this to
    /// decide whether to bind DoT/DoH ports.
    pub fn tls_ready(&self) -> bool {
        self.config.cert_path.is_some() && self.config.key_path.is_some()
    }
}

#[async_trait]
impl Plugin for TlsPlugin {
    fn name(&self) -> &str {
        "tls"
    }

    async fn handle<'a>(&'a self, ctx: &mut QueryContext, next: Next<'a>) -> DnsResult<()> {
        // Setup-only directive — pass through at request time.
        next.run(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_ready_only_when_both_paths_set() {
        let p = TlsPlugin::new(TlsConfig {
            cert_path: Some("/tls/cert.pem".into()),
            key_path: Some("/tls/key.pem".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(p.tls_ready());
        assert_eq!(p.cert_path(), Some("/tls/cert.pem"));
        assert_eq!(p.key_path(), Some("/tls/key.pem"));
    }

    #[test]
    fn tls_rejects_cert_without_key() {
        let err = TlsPlugin::new(TlsConfig {
            cert_path: Some("/cert.pem".into()),
            key_path: None,
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(err, DnsError::Config(_)));
    }

    #[test]
    fn tls_default_is_not_ready() {
        let p = TlsPlugin::new(TlsConfig::default()).unwrap();
        assert!(!p.tls_ready());
        assert_eq!(p.name(), "tls");
        assert!(p.alpn().is_empty());
        assert!(!p.require_client_cert());
    }

    #[test]
    fn tls_alpn_advertised() {
        let p = TlsPlugin::new(TlsConfig {
            alpn: vec!["dot".into(), "doq".into(), "h2".into()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(p.alpn(), &["dot", "doq", "h2"]);
    }
}
