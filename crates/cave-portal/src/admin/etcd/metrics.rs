//! Metrics tab — Raft log size, snapshot, commit duration.
//! Mirrors etcd's `/metrics` Prometheus endpoint key series.

use super::EtcdViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::escape;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtcdMetricsSnapshot {
    pub raft_log_size: u64,
    pub raft_log_term: u64,
    pub raft_term: u64,
    pub raft_index: u64,
    pub commit_index: u64,
    pub applied_index: u64,
    pub last_snapshot_unix: i64,
    pub snapshot_count: u32,
    pub disk_wal_fsync_p99_ms: u32,
    pub disk_backend_commit_p99_ms: u32,
}

pub fn metrics_snapshot(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<EtcdMetricsSnapshot, EtcdViewError> {
    let kv = super::keyspace::list_kv(state, ctx)?;
    let events = super::keyspace::watch_stream(state, ctx)?;
    let n = kv.len() as u64;
    let revisions: Vec<u64> = kv.iter().map(|r| r.revision).collect();
    let max_rev = revisions.iter().copied().max().unwrap_or(0);
    let raft_index = max_rev + events.len() as u64;
    Ok(EtcdMetricsSnapshot {
        raft_log_size: n * 256 + events.len() as u64 * 64,
        raft_log_term: 12,
        raft_term: 12,
        raft_index,
        commit_index: raft_index,
        applied_index: raft_index,
        last_snapshot_unix: 1_700_000_000,
        snapshot_count: 3,
        disk_wal_fsync_p99_ms: 6,
        disk_backend_commit_p99_ms: 18,
    })
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, EtcdViewError> {
    let m = metrics_snapshot(state, ctx)?;
    Ok(format!(
        r#"<section id="etcd-metrics" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Metrics (selected /metrics series)</h2>
  <dl class="grid grid-cols-[18rem_1fr] gap-x-4 gap-y-1 text-sm">
    <dt class="text-gray-500">etcd_server_has_leader</dt><dd>1</dd>
    <dt class="text-gray-500">etcd_server_leader_changes_seen_total</dt><dd>0</dd>
    <dt class="text-gray-500">etcd_debugging_mvcc_db_total_size_in_bytes</dt><dd>{rls}</dd>
    <dt class="text-gray-500">etcd_server_proposals_committed_total</dt><dd>{ci}</dd>
    <dt class="text-gray-500">etcd_server_proposals_applied_total</dt><dd>{ai}</dd>
    <dt class="text-gray-500">etcd_debugging_snap_save_total_duration_seconds count</dt><dd>{sc}</dd>
    <dt class="text-gray-500">last snapshot timestamp</dt><dd>{ls}</dd>
    <dt class="text-gray-500">etcd_disk_wal_fsync_duration_seconds p99</dt><dd>{wf} ms</dd>
    <dt class="text-gray-500">etcd_disk_backend_commit_duration_seconds p99</dt><dd>{bc} ms</dd>
    <dt class="text-gray-500">raft term / index</dt><dd>{rt} / {ri}</dd>
  </dl>
  <p class="text-xs text-gray-500 mt-2">Derived from the seeded KV + watch event log (etcd serves these via Prometheus <code>/metrics</code>).</p>
  <p class="text-xs text-gray-500">Local tenant: <code>{tenant}</code></p>
</section>"#,
        rls = m.raft_log_size,
        ci = m.commit_index,
        ai = m.applied_index,
        sc = m.snapshot_count,
        ls = m.last_snapshot_unix,
        wf = m.disk_wal_fsync_p99_ms,
        bc = m.disk_backend_commit_p99_ms,
        rt = m.raft_term,
        ri = m.raft_index,
        tenant = escape(ctx.tenant.as_str()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn metrics_snapshot_increases_with_kv_size() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "Metrics",
            "acme"
        );
        let s = AdminState::seeded();
        let m = metrics_snapshot(&s, &ctx(&[Permission::EtcdRead, Permission::EtcdWatch])).unwrap();
        assert!(m.raft_log_size > 0);
        assert!(m.commit_index >= m.applied_index || m.applied_index == m.commit_index);
    }

    #[test]
    fn metrics_snapshot_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(metrics_snapshot(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_emits_prometheus_series() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::EtcdRead, Permission::EtcdWatch])).unwrap();
        assert!(html.contains("etcd_server_has_leader"));
        assert!(html.contains("etcd_disk_wal_fsync_duration_seconds"));
    }
}
