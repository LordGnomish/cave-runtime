// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/brokers` — Kafka admin "Brokers" tab. cave-streams
//! runs a single-node broker today (per ADR-RUNTIME-STREAMING-
//! CONSOLIDATION-001 — Kafka + Pulsar wire on one process), so
//! this view synthesises a one-row cluster picture from the
//! tenant's topic load. The operator sees the broker the topics
//! are pointing at and the aggregate partition count it serves.
//!
//! Upstream: <https://kafka.apache.org/documentation/#basic_ops_cluster_id>

use super::StreamsViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRow {
    pub broker_id: i32,
    pub host: String,
    pub port: u16,
    /// Sum of all partitions across topics the tenant owns.
    pub partition_load: u64,
    /// `Active` for the live broker; future multi-node
    /// installations will surface `Fenced`/`Offline` here.
    pub status: &'static str,
}

pub fn list_brokers(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<BrokerRow>, StreamsViewError> {
    let topics = super::topics::list_topics_sorted(state, ctx)?;
    let load: u64 = topics.iter().map(|t| t.partitions as u64).sum();
    // cave-streams ships one broker today — broker_id=1 on
    // 0.0.0.0:9092 (default kafka port).
    Ok(vec![BrokerRow {
        broker_id: 1,
        host: "0.0.0.0".to_string(),
        port: 9092,
        partition_load: load,
        status: "Active",
    }])
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_brokers(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.broker_id.to_string(),
                escape(&r.host),
                r.port.to_string(),
                r.partition_load.to_string(),
                r.status.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Brokers ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Live broker roster. cave-streams single-process today. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#basic_ops_cluster_id">Kafka cluster ops</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["broker_id", "host", "port", "partition_load", "status"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/brokers",
        &format!("streams/brokers · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_returns_single_active_broker() {
        let rows = list_brokers(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "Active");
    }

    #[test]
    fn broker_load_matches_topic_partition_sum() {
        let topics = super::super::topics::list_topics_sorted(
            &AdminState::seeded(),
            &ctx(&[Permission::StreamsRead]),
        )
        .unwrap();
        let expected: u64 = topics.iter().map(|t| t.partitions as u64).sum();
        let brokers =
            list_brokers(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert_eq!(brokers[0].partition_load, expected);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_brokers(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn broker_id_defaults_to_one() {
        let rows = list_brokers(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert_eq!(rows[0].broker_id, 1);
    }

    #[test]
    fn render_includes_broker_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Brokers ("));
        assert!(html.contains("Active"));
    }
}
