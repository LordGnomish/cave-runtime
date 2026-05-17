// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/jobservice/job/impl/gc/garbage_collection.go
//! Garbage collection for unreferenced blobs.
//!
//! GC is run manually via POST /api/v2.0/system/gc or on a schedule.
//! The algorithm is mark-and-sweep: collect all digests reachable from
//! any live tag, then delete everything else.

use crate::harbor::storage::{GcStats, RegistryStorage};
use std::sync::Arc;
use tracing::info;

/// Run a full garbage-collection sweep and return statistics.
pub async fn run_gc(storage: Arc<RegistryStorage>) -> GcStats {
    info!("garbage collection: starting");
    let stats = storage.gc().await;
    info!(
        blobs_removed = stats.blobs_removed,
        blobs_retained = stats.blobs_retained,
        "garbage collection: complete"
    );
    stats
}
