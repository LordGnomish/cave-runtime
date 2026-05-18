// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE Docs Site — Developer documentation platform.
//!
//! Compatible with: GitBook / Docusaurus
//! Multi-version docs, full-text search, hierarchical nav, team collaboration.

pub mod models;
pub mod renderer;
pub mod routes;

use axum::Router;
use models::{DocPage, DocSite};
use std::sync::{Arc, Mutex};

pub struct DocsSiteState {
    pub sites: Arc<Mutex<Vec<DocSite>>>,
    pub pages: Arc<Mutex<Vec<DocPage>>>,
}

impl Default for DocsSiteState {
    fn default() -> Self {
        Self {
            sites: Arc::new(Mutex::new(Vec::new())),
            pages: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

pub fn router(state: Arc<DocsSiteState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "docs-site";
