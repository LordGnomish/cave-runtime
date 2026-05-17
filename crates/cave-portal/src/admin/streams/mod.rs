// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams` view — Kafka/Pulsar topic + consumer-group browser.
//!
//! Mirrors the Kafka admin dashboard pattern: per-tenant topics with
//! their partition / replication / retention / compaction settings,
//! plus consumer-group lag (current_offset vs log_end_offset). The
//! single mutator is `reset_consumer_offset`, which exposes the
//! `kafka-consumer-groups.sh --reset-offsets` semantics.
//!
//! Tab layout — mirrors Kafka admin tooling:
//!
//! * [`topics`]          — topic registry + per-topic configs
//! * [`brokers`]         — live broker roster
//! * [`consumer_groups`] — group state + lag
//! * [`partitions`]      — per-partition leader + replica view
//! * [`acls`]            — tenant-scoped allow rules

pub mod acls;
pub mod brokers;
pub mod connect;
pub mod consumer_groups;
pub mod partitions;
pub mod topics;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, StreamsConsumerGroup, StreamsTopic};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StreamsViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("topic {0} not found in this tenant")]
    TopicNotFound(String),
    #[error("consumer group {0} not found in this tenant")]
    GroupNotFound(String),
    #[error("offset {given} exceeds log end {end}")]
    OffsetOutOfRange { given: u64, end: u64 },
    #[error("compaction must be Delete, Compact or DeleteAndCompact")]
    InvalidCompaction,
}

pub fn list_topics(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsTopic>, StreamsViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    Ok(scope(&state.streams_topics.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_consumer_groups(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<StreamsConsumerGroup>, StreamsViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    Ok(scope(&state.streams_consumer_groups.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn inspect_topic(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<StreamsTopic, StreamsViewError> {
    list_topics(state, ctx)?
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| StreamsViewError::TopicNotFound(name.into()))
}

/// Highest-lag consumer groups first — the key dashboard signal.
pub fn lag_leaders(
    state: &AdminState,
    ctx: &RequestCtx,
    limit: usize,
) -> Result<Vec<StreamsConsumerGroup>, StreamsViewError> {
    let mut groups = list_consumer_groups(state, ctx)?;
    groups.sort_by(|a, b| b.lag().cmp(&a.lag()));
    groups.truncate(limit);
    Ok(groups)
}

/// Reset a consumer group's current offset. Refuses to set an offset
/// past the log end (Kafka rejects this with `OFFSET_OUT_OF_RANGE`).
pub fn reset_consumer_offset(
    state: &AdminState,
    ctx: &RequestCtx,
    group_id: &str,
    new_offset: u64,
) -> Result<(), StreamsViewError> {
    ctx.authorise(Permission::StreamsAdmin)?;
    let mut groups = state.streams_consumer_groups.write().unwrap();
    let target = groups
        .iter_mut()
        .find(|g| g.tenant == ctx.tenant && g.group_id == group_id)
        .ok_or_else(|| StreamsViewError::GroupNotFound(group_id.into()))?;
    if new_offset > target.log_end_offset {
        return Err(StreamsViewError::OffsetOutOfRange {
            given: new_offset,
            end: target.log_end_offset,
        });
    }
    target.current_offset = new_offset;
    Ok(())
}

/// Update the compaction strategy for a topic.
pub fn set_topic_compaction(
    state: &AdminState,
    ctx: &RequestCtx,
    topic: &str,
    compaction: &str,
) -> Result<(), StreamsViewError> {
    ctx.authorise(Permission::StreamsAdmin)?;
    let normalised: &'static str = match compaction {
        "Delete" => "Delete",
        "Compact" => "Compact",
        "DeleteAndCompact" => "DeleteAndCompact",
        _ => return Err(StreamsViewError::InvalidCompaction),
    };
    let mut topics = state.streams_topics.write().unwrap();
    let target = topics
        .iter_mut()
        .find(|t| t.tenant == ctx.tenant && t.name == topic)
        .ok_or_else(|| StreamsViewError::TopicNotFound(topic.into()))?;
    target.compaction = normalised;
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let topics = list_topics(state, ctx)?;
    let groups = list_consumer_groups(state, ctx)?;
    let t_rows: Vec<Vec<String>> = topics
        .iter()
        .map(|t| {
            vec![
                t.name.clone(),
                t.partitions.to_string(),
                format!("rf={}", t.replication_factor),
                format!("{} ms", t.retention_ms),
                t.compaction.into(),
            ]
        })
        .collect();
    let g_rows: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            vec![
                g.group_id.clone(),
                g.topic.clone(),
                g.members.to_string(),
                g.current_offset.to_string(),
                g.log_end_offset.to_string(),
                g.lag().to_string(),
                g.state.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Topics ({n_t})</h2>{t_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Consumer groups ({n_g})</h2>{g_tbl}</section>"#,
        n_t = topics.len(),
        n_g = groups.len(),
        t_tbl = table(
            &["name", "partitions", "rf", "retention", "compaction"],
            &t_rows,
        ),
        g_tbl = table(
            &["group_id", "topic", "members", "current", "end", "lag", "state"],
            &g_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams",
        &format!("streams · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/streams/src/components/Topics/TopicsPage.tsx",
    "TopicsPage",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_topics_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/streams/src/components/Topics/TopicsList.tsx",
            "TopicsList",
            "acme"
        );
        let s = AdminState::seeded();
        let t = list_topics(&s, &ctx(&[Permission::StreamsRead])).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn lag_leaders_orders_by_lag_desc() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/streams/src/components/Consumers/LagBoard.tsx",
            "LagBoard",
            "acme"
        );
        let s = AdminState::seeded();
        let leaders = lag_leaders(&s, &ctx(&[Permission::StreamsRead]), 10).unwrap();
        // events-consumer (lag 45000) > orders-consumer (lag 500)
        assert_eq!(leaders[0].group_id, "events-consumer");
        assert_eq!(leaders[0].lag(), 45_000);
        assert_eq!(leaders[1].group_id, "orders-consumer");
        assert_eq!(leaders[1].lag(), 500);
    }

    #[test]
    fn reset_consumer_offset_updates_current_and_validates_range() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/streams/src/components/Consumers/OffsetResetDialog.tsx",
            "OffsetReset",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::StreamsRead, Permission::StreamsAdmin]);
        reset_consumer_offset(&s, &c, "orders-consumer", 9_900).unwrap();
        let groups = list_consumer_groups(&s, &c).unwrap();
        let orders = groups.iter().find(|g| g.group_id == "orders-consumer").unwrap();
        assert_eq!(orders.current_offset, 9_900);
        assert_eq!(orders.lag(), 100);
        // Past log_end_offset → rejected.
        assert!(matches!(
            reset_consumer_offset(&s, &c, "orders-consumer", 10_001).unwrap_err(),
            StreamsViewError::OffsetOutOfRange { .. }
        ));
    }

    #[test]
    fn reset_consumer_offset_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "tenantScopeGuard",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::StreamsRead, Permission::StreamsAdmin]);
        assert!(matches!(
            reset_consumer_offset(&s, &c, "evil-consumer", 0).unwrap_err(),
            StreamsViewError::GroupNotFound(_)
        ));
    }

    #[test]
    fn set_topic_compaction_normalises_and_validates() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/streams/src/components/Topics/CompactionEditor.tsx",
            "CompactionEditor",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::StreamsRead, Permission::StreamsAdmin]);
        set_topic_compaction(&s, &c, "orders", "Compact").unwrap();
        let topic = inspect_topic(&s, &c, "orders").unwrap();
        assert_eq!(topic.compaction, "Compact");
        assert!(matches!(
            set_topic_compaction(&s, &c, "orders", "Squish").unwrap_err(),
            StreamsViewError::InvalidCompaction
        ));
    }

    #[test]
    fn list_topics_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(list_topics(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_excludes_evil_topic_and_evil_consumer() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/streams/src/components/Topics/TopicsPage.tsx",
            "TopicsPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Topics (2)"));
        assert!(html.contains("Consumer groups (2)"));
        assert!(html.contains("orders-consumer"));
        assert!(!html.contains("evil-topic"));
        assert!(!html.contains("evil-consumer"));
    }
}
