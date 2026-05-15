// SPDX-License-Identifier: AGPL-3.0-or-later
//! Persistence tab — RDB last-save, AOF size, rewrite status.

use super::super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistenceStatus {
    pub rdb_last_save_unix: i64,
    pub rdb_last_save_status: &'static str, // "ok" | "in_progress" | "failed"
    pub rdb_last_save_duration_sec: u32,
    pub aof_enabled: bool,
    pub aof_size_bytes: u64,
    pub aof_pending_bio_fsync: u32,
    pub aof_rewrite_in_progress: bool,
    pub aof_last_rewrite_time_sec: u32,
    pub aof_fsync_policy: &'static str, // "always" | "everysec" | "no"
}

pub fn status(_state: &AdminState, ctx: &RequestCtx) -> Result<PersistenceStatus, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(PersistenceStatus {
        rdb_last_save_unix: 1_715_749_500,
        rdb_last_save_status: "ok",
        rdb_last_save_duration_sec: 2,
        aof_enabled: true,
        aof_size_bytes: 67_108_864, // 64 MiB
        aof_pending_bio_fsync: 0,
        aof_rewrite_in_progress: false,
        aof_last_rewrite_time_sec: 47,
        aof_fsync_policy: "everysec",
    })
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let s = status(state, ctx)?;
    Ok(format!(
        r#"<section id="cache-cluster-persistence" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Persistence (INFO persistence)</h2>
  <dl class="grid grid-cols-[16rem_1fr] gap-x-4 gap-y-1 text-sm">
    <dt class="text-gray-500">rdb_last_save_unix</dt><dd>{rls}</dd>
    <dt class="text-gray-500">rdb_last_save_status</dt><dd><strong>{rss}</strong></dd>
    <dt class="text-gray-500">rdb_last_save_duration</dt><dd>{rsd}s</dd>
    <dt class="text-gray-500">aof_enabled</dt><dd>{ae}</dd>
    <dt class="text-gray-500">aof_current_size</dt><dd>{asb} B</dd>
    <dt class="text-gray-500">aof_pending_bio_fsync</dt><dd>{apf}</dd>
    <dt class="text-gray-500">aof_rewrite_in_progress</dt><dd>{aip}</dd>
    <dt class="text-gray-500">aof_last_rewrite_time</dt><dd>{alrt}s</dd>
    <dt class="text-gray-500">aof_fsync_policy</dt><dd><strong>{afp}</strong></dd>
  </dl>
</section>"#,
        rls = s.rdb_last_save_unix,
        rss = s.rdb_last_save_status,
        rsd = s.rdb_last_save_duration_sec,
        ae = s.aof_enabled,
        asb = s.aof_size_bytes,
        apf = s.aof_pending_bio_fsync,
        aip = s.aof_rewrite_in_progress,
        alrt = s.aof_last_rewrite_time_sec,
        afp = s.aof_fsync_policy,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn status_reports_aof_enabled_with_everysec_default() {
        let s = status(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(s.aof_enabled);
        assert_eq!(s.aof_fsync_policy, "everysec");
    }

    #[test]
    fn status_reports_last_rdb_save_status_ok() {
        let s = status(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert_eq!(s.rdb_last_save_status, "ok");
    }

    #[test]
    fn render_section_includes_all_fields() {
        let h = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        for k in [
            "rdb_last_save_unix",
            "rdb_last_save_status",
            "aof_enabled",
            "aof_current_size",
            "aof_fsync_policy",
        ] {
            assert!(h.contains(k), "missing {k}");
        }
    }

    #[test]
    fn refuses_status_without_permission() {
        assert!(status(&AdminState::seeded(), &ctx(&[])).is_err());
    }
}
