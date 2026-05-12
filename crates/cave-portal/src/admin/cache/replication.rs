//! Replication tab — Redis-style master/replica state. Mirrors
//! `INFO replication` output (role, connected_slaves, offsets,
//! backlog).

use super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationStatus {
    pub role: &'static str, // "master" | "slave"
    pub connected_slaves: u32,
    pub master_replid: &'static str,
    pub master_repl_offset: u64,
    pub repl_backlog_active: bool,
    pub repl_backlog_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPeer {
    pub ip: &'static str,
    pub port: u16,
    pub state: &'static str, // "online" | "sync" | "lag"
    pub offset: u64,
    pub lag_sec: u32,
}

pub fn replication_status(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<ReplicationStatus, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(ReplicationStatus {
        role: "master",
        connected_slaves: 2,
        master_replid: "d4ef5c2a8f10b9c3e7f1a2b3c4d5e6f7a8b9c0d1",
        master_repl_offset: 1_842_037_212,
        repl_backlog_active: true,
        repl_backlog_size_bytes: 1_048_576,
    })
}

pub fn list_replicas(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ReplicaPeer>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(vec![
        ReplicaPeer { ip: "10.244.1.10", port: 6379, state: "online", offset: 1_842_037_212, lag_sec: 0 },
        ReplicaPeer { ip: "10.244.2.10", port: 6379, state: "online", offset: 1_842_037_180, lag_sec: 1 },
        ReplicaPeer { ip: "10.244.3.10", port: 6379, state: "sync",   offset: 1_841_999_000, lag_sec: 12 },
    ])
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let status = replication_status(state, ctx)?;
    let replicas = list_replicas(state, ctx)?;
    let replica_rows: Vec<Vec<String>> = replicas
        .iter()
        .map(|r| {
            vec![
                r.ip.into(),
                r.port.to_string(),
                r.state.into(),
                r.offset.to_string(),
                format!("{}s", r.lag_sec),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-replication" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Replication ({role})</h2>
  <dl class="grid grid-cols-[16rem_1fr] gap-x-4 gap-y-1 text-sm">
    <dt class="text-gray-500">role</dt><dd><strong>{role}</strong></dd>
    <dt class="text-gray-500">connected_slaves</dt><dd>{cs}</dd>
    <dt class="text-gray-500">master_replid</dt><dd><code class="text-xs">{replid}</code></dd>
    <dt class="text-gray-500">master_repl_offset</dt><dd>{off}</dd>
    <dt class="text-gray-500">repl_backlog_active</dt><dd>{ba}</dd>
    <dt class="text-gray-500">repl_backlog_size</dt><dd>{bs} B</dd>
  </dl>
  <h3 class="text-md font-semibold mt-3 mb-1">Replicas ({n})</h3>
  {repl_tbl}
</section>"#,
        role = status.role,
        cs = status.connected_slaves,
        replid = status.master_replid,
        off = status.master_repl_offset,
        ba = status.repl_backlog_active,
        bs = status.repl_backlog_size_bytes,
        n = replicas.len(),
        repl_tbl = table(&["ip", "port", "state", "offset", "lag"], &replica_rows),
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
    fn replication_status_reports_master_role() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/Replication.tsx",
            "Replication",
            "acme"
        );
        let s = replication_status(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert_eq!(s.role, "master");
        assert!(s.repl_backlog_active);
    }

    #[test]
    fn list_replicas_returns_at_least_one() {
        let r = list_replicas(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(!r.is_empty());
    }

    #[test]
    fn refuses_without_permission() {
        assert!(replication_status(&AdminState::seeded(), &ctx(&[])).is_err());
        assert!(list_replicas(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_shows_master_replid_and_replicas() {
        let html = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(html.contains("master_replid"));
        assert!(html.contains("Replicas"));
    }
}
