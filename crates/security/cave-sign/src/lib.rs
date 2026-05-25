// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cave-sign` — Sigstore Cosign reimplementation.
//!
//! Upstream: sigstore/cosign v3.0.6 + sigstore/sigstore v1.10.6 (Apache-2.0).
//! Source commit pin: see `parity.manifest.toml`.
//!
//! Surface:
//!   * keypair + keyless signing
//!   * Fulcio + Rekor + SCT integration
//!   * SLSA in-toto + OpenVEX attestations
//!   * cert-identity / cert-issuer / require-rekor verification policy
//!   * cosign bundle format (.sig + .pem + bundle.json)
//!   * OCI artifact signature attachment

use std::sync::{Arc, Mutex};

pub mod attestation;
pub mod blob;
pub mod bundle;
pub mod engine;
pub mod error;
pub mod fulcio;
pub mod keyless;
pub mod keypair;
pub mod models;
pub mod oci;
pub mod oidc;
pub mod policy;
pub mod rekor;
pub mod routes;
pub mod sct;
pub mod signature;
pub mod signing_config;
pub mod store;
pub mod tlog;
pub mod trustedroot;
pub mod verify;

use axum::Router;
use fulcio::FulcioClient;
use policy::Policy;
use rekor::RekorClient;
use signing_config::SigningConfig;
use store::SignedArtifactStore;

pub const MODULE_NAME: &str = "sign";

/// Module state. Bundles the Fulcio + Rekor clients, the in-memory store,
/// and the active verification policy that the HTTP routes consult.
pub struct State {
    pub config: SigningConfig,
    pub fulcio: FulcioClient,
    pub rekor: RekorClient,
    pub store: SignedArtifactStore,
    pub policy: Mutex<Policy>,
}

impl Default for State {
    fn default() -> Self {
        let cfg = SigningConfig::sovereign();
        Self {
            fulcio: FulcioClient::new(cfg.fulcio_url.clone()),
            rekor: RekorClient::new(cfg.rekor_url.clone()),
            store: SignedArtifactStore::new(),
            policy: Mutex::new(Policy::default()),
            config: cfg,
        }
    }
}

impl State {
    pub fn with_config(cfg: SigningConfig) -> Self {
        Self {
            fulcio: FulcioClient::new(cfg.fulcio_url.clone()),
            rekor: RekorClient::new(cfg.rekor_url.clone()),
            store: SignedArtifactStore::new(),
            policy: Mutex::new(Policy::default()),
            config: cfg,
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_is_sign() {
        assert_eq!(MODULE_NAME, "sign");
    }

    #[test]
    fn default_state_is_sovereign() {
        let s = State::default();
        assert_eq!(s.config.deployment, signing_config::Deployment::Sovereign);
        assert!(s.fulcio.base_url.contains("cave-fulcio"));
    }

    #[test]
    fn public_good_state_swaps_endpoints() {
        let s = State::with_config(SigningConfig::public_good());
        assert!(s.fulcio.base_url.contains("sigstore.dev"));
        assert!(s.rekor.base_url.contains("sigstore.dev"));
    }
}
