// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus text-exposition for cave-streams.
//!
//! The workspace aggregates most series through the central `cave-metrics`
//! crate, but cave-streams also exposes a focused `/api/streams/metrics`
//! endpoint reporting live broker/Pulsar gauges plus counters for the
//! streaming-ray-2 transaction-coordinator and share-group preview
//! surfaces. All values are real: gauges read from [`crate::StreamsState`]
//! at scrape time and the `*_preview_total` series are process-wide
//! monotonic counters bumped by the REST handlers.

use crate::StreamsState;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of Pulsar transaction previews served.
pub static PULSAR_TXN_PREVIEW_TOTAL: AtomicU64 = AtomicU64::new(0);
/// Number of Kafka share-group previews served.
pub static SHARE_GROUP_PREVIEW_TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn inc_txn_preview() {
    PULSAR_TXN_PREVIEW_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn inc_share_preview() {
    SHARE_GROUP_PREVIEW_TOTAL.fetch_add(1, Ordering::Relaxed);
}

/// Render the Prometheus text exposition (version 0.0.4).
pub fn render_prometheus(state: &StreamsState) -> String {
    let kafka_topics = state.broker.list_topics().len();
    let kafka_groups = state.broker.groups.list_groups().len();
    let pulsar_tenants = state.pulsar_admin.list_tenants().len();
    let txn = PULSAR_TXN_PREVIEW_TOTAL.load(Ordering::Relaxed);
    let share = SHARE_GROUP_PREVIEW_TOTAL.load(Ordering::Relaxed);

    let mut out = String::new();
    out.push_str("# HELP cave_streams_kafka_topics Number of Kafka topics on the broker.\n");
    out.push_str("# TYPE cave_streams_kafka_topics gauge\n");
    out.push_str(&format!("cave_streams_kafka_topics {kafka_topics}\n"));

    out.push_str("# HELP cave_streams_kafka_consumer_groups Number of Kafka consumer groups.\n");
    out.push_str("# TYPE cave_streams_kafka_consumer_groups gauge\n");
    out.push_str(&format!("cave_streams_kafka_consumer_groups {kafka_groups}\n"));

    out.push_str("# HELP cave_streams_pulsar_tenants Number of Pulsar tenants.\n");
    out.push_str("# TYPE cave_streams_pulsar_tenants gauge\n");
    out.push_str(&format!("cave_streams_pulsar_tenants {pulsar_tenants}\n"));

    out.push_str(
        "# HELP cave_streams_pulsar_txn_preview_total Pulsar transaction previews served.\n",
    );
    out.push_str("# TYPE cave_streams_pulsar_txn_preview_total counter\n");
    out.push_str(&format!("cave_streams_pulsar_txn_preview_total {txn}\n"));

    out.push_str(
        "# HELP cave_streams_share_group_preview_total Kafka share-group previews served.\n",
    );
    out.push_str("# TYPE cave_streams_share_group_preview_total counter\n");
    out.push_str(&format!("cave_streams_share_group_preview_total {share}\n"));

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposition_has_help_type_and_real_gauges() {
        let state = StreamsState::default();
        let text = render_prometheus(&state);
        // Prometheus 0.0.4 framing.
        assert!(text.contains("# HELP cave_streams_kafka_topics"));
        assert!(text.contains("# TYPE cave_streams_kafka_topics gauge"));
        assert!(text.contains("cave_streams_kafka_topics 0"));
        // Counter series present.
        assert!(text.contains("# TYPE cave_streams_pulsar_txn_preview_total counter"));
        assert!(text.contains("# TYPE cave_streams_share_group_preview_total counter"));
    }

    #[test]
    fn preview_counters_increment() {
        let before = PULSAR_TXN_PREVIEW_TOTAL.load(Ordering::Relaxed);
        inc_txn_preview();
        assert_eq!(
            PULSAR_TXN_PREVIEW_TOTAL.load(Ordering::Relaxed),
            before + 1
        );
        let sb = SHARE_GROUP_PREVIEW_TOTAL.load(Ordering::Relaxed);
        inc_share_preview();
        assert_eq!(SHARE_GROUP_PREVIEW_TOTAL.load(Ordering::Relaxed), sb + 1);
    }
}
