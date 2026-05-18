// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::{
    api::{
        records::{batch_records, create_record, delete_record, list_records, RecordState},
        zones::{create_zone, delete_zone, export_zone, get_zone, list_zones, ZoneState},
    },
    zone::ZoneManager,
};

/// Build the HTTP management router.
pub fn create_router(zones: Arc<ZoneManager>) -> Router {
    // Both states wrap the same Arc<ZoneManager>; build two separate sub-routers
    // and merge them.
    let zone_state = ZoneState {
        zones: Arc::clone(&zones),
    };
    let record_state = RecordState {
        zones: Arc::clone(&zones),
    };

    let zone_router = Router::new()
        .route("/api/v1/zones", get(list_zones).post(create_zone))
        .route("/api/v1/zones/{zone}", get(get_zone).delete(delete_zone))
        .route("/api/v1/zones/{zone}/export", get(export_zone))
        .with_state(zone_state);

    let record_router = Router::new()
        .route(
            "/api/v1/zones/{zone}/records",
            get(list_records).post(create_record),
        )
        .route("/api/v1/zones/{zone}/records/batch", post(batch_records))
        .route(
            "/api/v1/zones/{zone}/records/{name}/{rtype}",
            delete(delete_record),
        )
        .with_state(record_state);

    zone_router.merge(record_router)
}
