// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CAVE Trivy — vulnerability + misconfig + secret scanner.
//!
//! Reimplements aquasecurity/trivy v0.70.0 across the seven scan targets
//! image / fs / repo / k8s / sbom / secret / config with an offline-first
//! vulnerability DB (OSV + CVE + GHSA), OS and language package detection,
//! Terraform/Kubernetes/Dockerfile/Helm IaC misconfig checks, CycloneDX +
//! SPDX SBOM emission, VEX (OpenVEX) suppression and the report formats
//! `table` / `json` / `sarif` / `template`.

pub mod cache;
pub mod engine;
pub mod error;
pub mod filter;
pub mod ignore;
pub mod k8s_operator;
pub mod misconf;
pub mod models;
pub mod osv;
pub mod pkg_lang;
pub mod pkg_os;
pub mod purl;
pub mod report_json;
pub mod report_sarif;
pub mod report_table;
pub mod report_template;
pub mod routes;
pub mod sbom_cyclonedx;
pub mod sbom_spdx;
pub mod scan_fs;
pub mod scan_iac;
pub mod scan_image;
pub mod scan_k8s;
pub mod scan_license;
pub mod scan_repo;
pub mod scan_sbom;
pub mod scan_secret;
pub mod server;
pub mod severity;
pub mod store;
pub mod vex;
pub mod vulndb;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

pub use error::TrivyError;
pub use models::*;

/// Module identity. Mirrors the convention used by other cave crates.
pub const MODULE_NAME: &str = "trivy";

/// Pinned upstream tag/commit. Used by the parity self-audit to assert that
/// the version baked into the crate matches the manifest pin.
pub const UPSTREAM_VERSION: &str = "v0.70.0";
pub const UPSTREAM_SHA: &str = "8a3177aedf7ee0864920eb1852eef031cd3742b8";

pub struct State {
    pub pool: Arc<CavePool>,
    pub store: store::ScanStore,
    pub vulndb: vulndb::VulnDb,
    pub cache: cache::ScanCache,
}

impl State {
    pub fn new(pool: Arc<CavePool>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            store: store::ScanStore::new(),
            vulndb: vulndb::VulnDb::cave_default(),
            cache: cache::ScanCache::new(),
        })
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            pool: Arc::new(cave_db::CavePool::mock()),
            store: store::ScanStore::new(),
            vulndb: vulndb::VulnDb::cave_default(),
            cache: cache::ScanCache::new(),
        }
    }
}

pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}
