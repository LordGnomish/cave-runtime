//! CAVE Security — Falco runtime rule engine + Trivy vulnerability scanner.
//!
//! Compatible with: Falco + Trivy (full feature parity)
//! Upstream tracking: see cave-upstream for monitored features.

pub mod falco;
pub mod routes;
pub mod trivy;

use axum::Router;
use std::{
    collections::VecDeque,
    sync::Arc,
};
use tokio::sync::{broadcast, Mutex, RwLock};

// ---------------------------------------------------------------------------
// Module state
// ---------------------------------------------------------------------------

/// Maximum number of alerts kept in the in-memory ring buffer.
pub const ALERT_HISTORY_LEN: usize = 10_000;
/// Broadcast channel capacity.
const ALERT_CHANNEL_CAP: usize = 4_096;

pub struct SecurityState {
    // Falco
    pub rule_store: Arc<RwLock<falco::engine::RuleStore>>,
    pub alert_tx: broadcast::Sender<falco::engine::Alert>,
    pub alert_history: Arc<Mutex<VecDeque<falco::engine::Alert>>>,
    // Trivy
    pub vuln_db: Arc<RwLock<trivy::VulnDb>>,
}

impl Default for SecurityState {
    fn default() -> Self {
        let (alert_tx, _) = broadcast::channel(ALERT_CHANNEL_CAP);
        SecurityState {
            rule_store: Arc::new(RwLock::new(falco::engine::RuleStore::default())),
            alert_tx,
            alert_history: Arc::new(Mutex::new(VecDeque::with_capacity(ALERT_HISTORY_LEN))),
            vuln_db: Arc::new(RwLock::new(trivy::VulnDb::default())),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<SecurityState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "security";
