// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/groups` — Kafka consumer-group view (the new
//! single-word tab name for the 9-tab streams console). Per-row:
//! state, members, assignor, current/end offsets, lag.
//!
//! Persona gate — tenant-scoped READ-ONLY:
//!   * `PlatformAdmin` → all rows
//!   * `TenantAdmin` → rows in caller's tenant (existing tenant-
//!     scoping via [`crate::admin::state::scope`])
//!   * `Anonymous` (dev shortcut) → tenant-scoped view
//!
//! Mutation (offset reset / delete) is admin-only — exposed through
//! the legacy `consumer_groups.rs` mod-level helpers + cavectl.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, StreamsConsumerGroup};
use super::StreamsViewError;

/// Synth-enriched row: adds `assignor` derived from the group id
/// (round-robin for `*-consumer`, sticky for `*-events*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub group_id: String,
    pub topic: String,
    pub members: u32,
    pub assignor: &'static str,
    pub current_offset: u64,
    pub log_end_offset: u64,
    pub lag: u64,
    pub state: &'static str,
}

impl GroupRow {
    fn from_state(g: &StreamsConsumerGroup) -> Self {
        let assignor: &'static str = if g.group_id.contains("events") {
            "sticky"
        } else {
            "round-robin"
        };
        Self {
            group_id: g.group_id.clone(),
            topic: g.topic.clone(),
            members: g.members,
            assignor,
            current_offset: g.current_offset,
            log_end_offset: g.log_end_offset,
            lag: g.lag(),
            state: g.state,
        }
    }
}

pub fn list_groups(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<GroupRow>, StreamsViewError> {
    ctx.authorise(Permission::StreamsRead)?;
    let mut rows: Vec<GroupRow> = scope(
        &state.streams_consumer_groups.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .map(GroupRow::from_state)
    .collect();
    rows.sort_by(|a, b| b.lag.cmp(&a.lag).then(a.group_id.cmp(&b.group_id)));
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_groups(state, ctx)?;
    let total_lag: u64 = rows.iter().map(|g| g.lag).sum();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|g| {
            vec![
                escape(&g.group_id),
                escape(&g.topic),
                g.members.to_string(),
                g.assignor.to_string(),
                g.current_offset.to_string(),
                g.log_end_offset.to_string(),
                g.lag.to_string(),
                g.state.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Consumer Groups ({n}) · total lag {total}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Sorted by lag desc. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#basic_ops_consumer_lag">Kafka consumer lag</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        total = total_lag,
        tbl = table(
            &["group_id", "topic", "members", "assignor", "current", "end", "lag", "state"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/groups",
        &format!("streams/groups · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Persona;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }
    fn ctx_as(persona: Persona, perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as("acme", perms, persona)
    }

    #[test]
    fn list_returns_seeded_groups_for_tenant() {
        let rows = list_groups(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(rows.iter().any(|g| g.group_id == "orders-consumer"));
        assert!(rows.iter().any(|g| g.group_id == "events-consumer"));
    }

    #[test]
    fn list_sorted_by_lag_desc() {
        let rows = list_groups(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].lag >= w[1].lag);
        }
    }

    #[test]
    fn assignor_set_by_topic() {
        let rows = list_groups(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for g in &rows {
            if g.group_id.contains("events") {
                assert_eq!(g.assignor, "sticky");
            } else {
                assert_eq!(g.assignor, "round-robin");
            }
        }
    }

    #[test]
    fn list_accepts_tenant_admin_persona_read_only() {
        // TenantAdmin should be able to READ groups in their tenant.
        assert!(list_groups(
            &AdminState::seeded(),
            &ctx_as(Persona::TenantAdmin, &[Permission::StreamsRead]),
        )
        .is_ok());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_groups(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_excludes_evil_consumer() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Consumer Groups ("));
        assert!(html.contains("orders-consumer"));
        assert!(!html.contains("evil-consumer"));
    }
}
