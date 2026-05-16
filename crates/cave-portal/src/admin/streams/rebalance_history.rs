// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/rebalance_history` — last N rebalances per
//! consumer group. Each row carries the trigger reason, the
//! before/after member count, the duration, and a timestamp.
//!
//! Persona gate: PlatformAdmin only — rebalance history is a
//! diagnostic / operator surface (no per-tenant scoping).
//!
//! Source — synthesised deterministically from the seeded
//! [`crate::admin::state::StreamsConsumerGroup`] rows so future
//! runtime accessors can drop in without test churn.

use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};
use super::StreamsViewError;

/// Why a rebalance fired. Mirrors upstream Kafka's
/// `GroupCoordinator` log lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebalanceTrigger {
    MemberJoin,
    MemberLeave,
    HeartbeatTimeout,
    AssignmentChange,
}

impl RebalanceTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            RebalanceTrigger::MemberJoin => "MemberJoin",
            RebalanceTrigger::MemberLeave => "MemberLeave",
            RebalanceTrigger::HeartbeatTimeout => "HeartbeatTimeout",
            RebalanceTrigger::AssignmentChange => "AssignmentChange",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebalanceEvent {
    pub group_id: String,
    pub timestamp_unix: i64,
    pub trigger: RebalanceTrigger,
    pub members_before: u32,
    pub members_after: u32,
    pub duration_ms: u64,
}

/// Per-group window — last 5 rebalances synthesised from each
/// group's id + current_offset (deterministic).
pub fn list_history(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<RebalanceEvent>, StreamsViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    ctx.authorise(Permission::StreamsRead)?;
    let guard = state.streams_consumer_groups.read().unwrap();
    let groups = scope(&guard, &ctx.tenant, |r| &r.tenant);
    let triggers = [
        RebalanceTrigger::MemberJoin,
        RebalanceTrigger::MemberLeave,
        RebalanceTrigger::HeartbeatTimeout,
        RebalanceTrigger::AssignmentChange,
        RebalanceTrigger::MemberJoin,
    ];
    // 2026-05-16T00:00:00Z = 1747353600 unix.
    let base_ts: i64 = 1_747_353_600;
    let mut out = Vec::new();
    for g in groups {
        for (i, trig) in triggers.iter().enumerate() {
            // Walk backwards in time so the first event is newest.
            let ts = base_ts - (i as i64) * 600;
            let before = g.members.saturating_sub(i as u32 % 2);
            let after = before.saturating_add((i as u32) % 3);
            let dur = 250 + (i as u64) * 130 + (g.current_offset % 200);
            out.push(RebalanceEvent {
                group_id: g.group_id.clone(),
                timestamp_unix: ts,
                trigger: *trig,
                members_before: before,
                members_after: after,
                duration_ms: dur,
            });
        }
    }
    // Newest first across all groups.
    out.sort_by(|a, b| b.timestamp_unix.cmp(&a.timestamp_unix).then(a.group_id.cmp(&b.group_id)));
    Ok(out)
}

pub fn rebalance_count(rows: &[RebalanceEvent]) -> std::collections::BTreeMap<String, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.group_id.clone()).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_history(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.timestamp_unix.to_string(),
                escape(&r.group_id),
                r.trigger.as_str().to_string(),
                format!("{} → {}", r.members_before, r.members_after),
                format!("{} ms", r.duration_ms),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Rebalance History ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Last 5 rebalances per group. Upstream:
    <a class="text-blue-700 underline" href="https://kafka.apache.org/documentation/#impl_consumer_rebalance">Group rebalance protocol</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["timestamp", "group_id", "trigger", "members", "duration"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/rebalance_history",
        &format!("streams/rebalance_history · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }
    fn ctx_as(persona: Persona, perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as("acme", perms, persona)
    }

    #[test]
    fn list_history_returns_five_events_per_group() {
        let rows = list_history(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        // 2 acme groups × 5 events = 10.
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn list_history_sorted_newest_first() {
        let rows = list_history(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].timestamp_unix >= w[1].timestamp_unix);
        }
    }

    #[test]
    fn rebalance_count_per_group_is_five() {
        let rows = list_history(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let by_g = rebalance_count(&rows);
        for (_, n) in &by_g {
            assert_eq!(*n, 5);
        }
    }

    #[test]
    fn list_history_rejects_tenant_admin_persona() {
        assert!(list_history(
            &AdminState::seeded(),
            &ctx_as(Persona::TenantAdmin, &[Permission::StreamsRead]),
        )
        .is_err());
    }

    #[test]
    fn list_history_accepts_platform_admin() {
        assert!(list_history(
            &AdminState::seeded(),
            &ctx_as(Persona::PlatformAdmin, &[Permission::StreamsRead]),
        )
        .is_ok());
    }

    #[test]
    fn list_history_rejects_without_permission() {
        assert!(list_history(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_table_and_trigger_names() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Rebalance History ("));
        assert!(html.contains("MemberJoin"));
        assert!(!html.contains("evil-consumer"));
    }
}
