// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts crate root (consolidation, no single upstream)
//! cave-artifacts — consolidated artifact repository platform.
//!
//! Bundles best-of-breed upstream parities under one crate, mirroring the
//! cave-streams pattern of merging multiple upstream surfaces:
//! - `harbor` — container registry (Harbor v2 + Docker Registry V2 + OCI)
//! - `pulp`   — multi-format artifact repository (Pulp v3 — RPM/Deb/PyPI/etc.)
//! - `nexus`  — universal binary repository (Sonatype Nexus 3) — initial
//!              port: repository/component/asset/cleanup/routing + raw
//!              format end-to-end; remaining format adapters land later
//! - `cosign` — supply-chain signature module: ECDSA-P256 (real) +
//!              ML-DSA-65 hybrid composite (Ed25519 real + ML-DSA fixture
//!              today; backend swap is contained inside cave-certs)
//!
//! Surfaces:
//! - [`ArtifactsState`] — combined module state graph
//! - [`router`]         — combined axum Router merging all sub-modules

pub mod core;
pub mod cosign;
pub mod harbor;
pub mod integrations;
pub mod nexus;
pub mod pulp;

use axum::{routing::get, Json, Router};
use std::sync::Arc;

/// Combined state graph composing one Arc per upstream sub-module.
///
/// Each field is independently constructible and exposed publicly so callers
/// that only need one upstream (e.g. `cave-registry` alias re-export of harbor)
/// can build it in isolation without paying for the others.
pub struct ArtifactsState {
    pub harbor: Arc<harbor::RegistryState>,
    pub pulp: Arc<pulp::ArtifactsState>,
    pub nexus: Arc<nexus::NexusState>,
    pub cosign: Arc<cosign::CosignState>,
}

impl Default for ArtifactsState {
    fn default() -> Self {
        Self {
            harbor: Arc::new(harbor::RegistryState::default()),
            pulp: Arc::new(pulp::ArtifactsState::default()),
            nexus: Arc::new(nexus::NexusState::default()),
            cosign: Arc::new(cosign::CosignState::default()),
        }
    }
}

/// Build the combined axum router (harbor ∪ pulp ∪ nexus ∪ cosign ∪ /api/artifacts/health).
pub fn router(state: Arc<ArtifactsState>) -> Router {
    harbor::router(Arc::clone(&state.harbor))
        .merge(pulp::router(Arc::clone(&state.pulp)))
        .merge(nexus::router(Arc::clone(&state.nexus)))
        .merge(cosign::router(Arc::clone(&state.cosign)))
        .merge(health_router())
}

pub const MODULE_NAME: &str = "artifacts";

fn health_router() -> Router {
    Router::new().route(
        "/api/artifacts/health",
        get(|| async {
            Json(serde_json::json!({
                "status": "ok",
                "module": MODULE_NAME,
                "subsystems": {
                    "harbor": harbor::MODULE_NAME,
                    "pulp":   pulp::MODULE_NAME,
                    "nexus":  nexus::MODULE_NAME,
                    "cosign": cosign::MODULE_NAME,
                }
            }))
        }),
    )
}
