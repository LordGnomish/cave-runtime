// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Signing configuration — which Fulcio + Rekor + OIDC issuer the signer
//! should target.
//!
//! Maps to:
//!   * cmd/cosign/cli/signingconfig → `cosign signing-config`
//!   * pkg/cosign/sign.go::Options  → SignerOpts
//!
//! Sovereign cave installs override `fulcio_url`/`rekor_url` to point at
//! cave-fulcio + cave-rekor; defaults remain the public-good endpoints.

use crate::fulcio::{CAVE_FULCIO_DEFAULT_URL, PUBLIC_GOOD_FULCIO_URL};
use crate::rekor::{CAVE_REKOR_DEFAULT_URL, PUBLIC_GOOD_REKOR_URL};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SigningConfig {
    pub fulcio_url: String,
    pub rekor_url: String,
    pub oidc_issuer: String,
    pub oidc_client_id: String,
    /// `public-good` → talk to `sigstore.dev`; `sovereign` → talk to in-cluster cave.
    pub deployment: Deployment,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Deployment {
    PublicGood,
    Sovereign,
}

impl SigningConfig {
    pub fn public_good() -> Self {
        Self {
            fulcio_url: PUBLIC_GOOD_FULCIO_URL.into(),
            rekor_url: PUBLIC_GOOD_REKOR_URL.into(),
            oidc_issuer: "https://oauth2.sigstore.dev/auth".into(),
            oidc_client_id: "sigstore".into(),
            deployment: Deployment::PublicGood,
        }
    }

    pub fn sovereign() -> Self {
        Self {
            fulcio_url: CAVE_FULCIO_DEFAULT_URL.into(),
            rekor_url: CAVE_REKOR_DEFAULT_URL.into(),
            oidc_issuer: "https://oidc.cave.svc.cluster.local".into(),
            oidc_client_id: "cave-sign".into(),
            deployment: Deployment::Sovereign,
        }
    }

    pub fn merge_env(&mut self) {
        if let Ok(v) = std::env::var("CAVE_SIGN_FULCIO_URL") {
            self.fulcio_url = v;
        }
        if let Ok(v) = std::env::var("CAVE_SIGN_REKOR_URL") {
            self.rekor_url = v;
        }
        if let Ok(v) = std::env::var("CAVE_SIGN_OIDC_ISSUER") {
            self.oidc_issuer = v;
        }
        if let Ok(v) = std::env::var("CAVE_SIGN_OIDC_CLIENT_ID") {
            self.oidc_client_id = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_good_targets_sigstore_dev() {
        let c = SigningConfig::public_good();
        assert!(c.fulcio_url.contains("sigstore.dev"));
        assert!(c.rekor_url.contains("sigstore.dev"));
        assert_eq!(c.deployment, Deployment::PublicGood);
    }

    #[test]
    fn sovereign_targets_cluster_local() {
        let c = SigningConfig::sovereign();
        assert!(c.fulcio_url.contains("cave-fulcio"));
        assert!(c.rekor_url.contains("cave-rekor"));
        assert_eq!(c.deployment, Deployment::Sovereign);
    }

    #[test]
    fn config_serde_roundtrip() {
        let c = SigningConfig::sovereign();
        let s = serde_json::to_string(&c).unwrap();
        let back: SigningConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn deployment_kind_is_kebab_case() {
        let s = serde_json::to_string(&Deployment::PublicGood).unwrap();
        assert_eq!(s, "\"public-good\"");
    }
}
