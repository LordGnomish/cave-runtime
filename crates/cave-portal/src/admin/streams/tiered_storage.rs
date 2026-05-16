// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/streams/tiered_storage` — KIP-405 tiered-storage status
//! per topic-partition. Surfaces:
//!
//!   * `remote_log_manager_state` — RLM lifecycle phase
//!   * `segments_uploaded` — number of segments copied to remote
//!   * `copy_lag_bytes` — local-log-end - remote-log-end
//!   * `last_upload_error` — Optional error string from the last
//!     `RemoteLogManager.copyLogSegment` failure
//!
//! Persona gate — tenant-scoped READ-ONLY:
//!   * `PlatformAdmin` → all rows
//!   * `TenantAdmin` → caller's tenant
//!   * `Anonymous` (dev) → tenant-scoped
//!
//! Source — synthesised from each tenant's [`StreamsTopic`] rows.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::StreamsViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierRow {
    pub topic: String,
    pub partition: u32,
    pub rlm_state: &'static str,
    pub segments_uploaded: u64,
    pub copy_lag_bytes: u64,
    pub last_upload_error: Option<String>,
}

pub fn list_status(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<TierRow>, StreamsViewError> {
    let topics = super::topics::list_topics_sorted(state, ctx)?;
    let mut rows = Vec::new();
    // Each topic: one row per partition; first partition Carries
    // a synthetic upload error if the topic is `events` (the seeded
    // high-throughput one) — deterministic so tests are stable.
    for t in &topics {
        for p in 0..t.partitions {
            let state_str: &'static str = if t.compaction == "Compact" {
                "CopyAhead" // log compaction streams ahead of remote
            } else if p % 5 == 0 {
                "CopyInProgress"
            } else {
                "Active"
            };
            let segments = (t.partitions as u64) * 32 + p as u64;
            let lag = if state_str == "CopyInProgress" { 4 * 1024 * 1024 } else { 0 };
            let err = if t.name == "events" && p == 0 {
                Some("s3: Slow Down (503)".to_string())
            } else {
                None
            };
            rows.push(TierRow {
                topic: t.name.clone(),
                partition: p,
                rlm_state: state_str,
                segments_uploaded: segments,
                copy_lag_bytes: lag,
                last_upload_error: err,
            });
        }
    }
    Ok(rows)
}

pub fn total_copy_lag(rows: &[TierRow]) -> u64 {
    rows.iter().map(|r| r.copy_lag_bytes).sum()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, StreamsViewError> {
    let rows = list_status(state, ctx)?;
    let total = total_copy_lag(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let err = match &r.last_upload_error {
                Some(e) => escape(e),
                None => "—".into(),
            };
            vec![
                escape(&r.topic),
                r.partition.to_string(),
                r.rlm_state.to_string(),
                r.segments_uploaded.to_string(),
                r.copy_lag_bytes.to_string(),
                err,
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Tiered Storage ({n}) · total copy lag {total} B</h2>
  <p class="text-sm text-gray-600 mb-3">
    KIP-405 RemoteLogManager status. Upstream:
    <a class="text-blue-700 underline" href="https://cwiki.apache.org/confluence/display/KAFKA/KIP-405%3A+Kafka+Tiered+Storage">KIP-405</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        total = total,
        tbl = table(
            &["topic", "partition", "rlm_state", "segments_uploaded", "copy_lag_bytes", "last_error"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/streams/tiered_storage",
        &format!("streams/tiered_storage · {}", escape(ctx.tenant.as_str())),
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
    fn list_status_one_row_per_partition() {
        let rows = list_status(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let topics = super::super::topics::list_topics_sorted(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let expected: u32 = topics.iter().map(|t| t.partitions).sum();
        assert_eq!(rows.len() as u32, expected);
    }

    #[test]
    fn list_status_compacted_topics_in_copy_ahead() {
        let rows = list_status(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        // 'events' is the Compact topic in the seeded state.
        let events_rows: Vec<_> = rows.iter().filter(|r| r.topic == "events").collect();
        assert!(!events_rows.is_empty());
        // Most events rows should be in CopyAhead (compacted), modulo
        // the synthetic upload-error row.
        let copy_ahead = events_rows.iter().filter(|r| r.rlm_state == "CopyAhead").count();
        assert_eq!(copy_ahead, events_rows.len());
    }

    #[test]
    fn synthetic_upload_error_lands_on_events_partition_zero() {
        let rows = list_status(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        let first = rows.iter().find(|r| r.topic == "events" && r.partition == 0).unwrap();
        assert!(first.last_upload_error.is_some());
    }

    #[test]
    fn list_status_accepts_tenant_admin_persona_read_only() {
        assert!(list_status(
            &AdminState::seeded(),
            &ctx_as(Persona::TenantAdmin, &[Permission::StreamsRead]),
        )
        .is_ok());
    }

    #[test]
    fn list_status_rejects_without_permission() {
        assert!(list_status(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_kip_link_and_total_lag() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::StreamsRead])).unwrap();
        assert!(html.contains("Tiered Storage ("));
        assert!(html.contains("KIP-405"));
        assert!(html.contains("total copy lag"));
    }
}
