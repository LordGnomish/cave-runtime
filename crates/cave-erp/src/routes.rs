use crate::store::ErpStore;
use axum::{response::IntoResponse, routing::get, Json, Router};
use serde_json::json;
use std::sync::Arc;

async fn health() -> impl IntoResponse {
    Json(json!({
        "module": "cave-erp",
        "status": "ok",
        "upstream": "Odoo Community Edition",
        "submodules": [
            "hr",
            "recruitment",
            "crm",
            "sales",
            "purchase",
            "inventory",
            "accounting",
            "manufacturing",
            "projects"
        ]
    }))
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    Router::new()
        .route("/api/erp/health", get(health))
        .merge(crate::modules::hr::create_router(state.clone()))
        .merge(crate::modules::recruitment::create_router(state.clone()))
        .merge(crate::modules::crm::create_router(state.clone()))
        .merge(crate::modules::sales::create_router(state.clone()))
        .merge(crate::modules::purchase::create_router(state.clone()))
        .merge(crate::modules::inventory::create_router(state.clone()))
        .merge(crate::modules::accounting::create_router(state.clone()))
        .merge(crate::modules::manufacturing::create_router(state.clone()))
        .merge(crate::modules::projects::create_router(state))
}
