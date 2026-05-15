//! Commands tab — Redis-style command history + SLOWLOG.
//!
//! Today we synthesise rows from the keyspace activity so the page
//! has the right RedisInsight shape; a live deployment proxies via
//! `INFO commandstats` + `SLOWLOG GET`.

use super::CacheViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandStat {
    pub command: &'static str,
    pub calls: u64,
    pub usec_total: u64,
    pub usec_per_call: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlowlogRow {
    pub id: u64,
    pub timestamp_unix: i64,
    pub duration_usec: u64,
    pub command: String,
}

pub fn list_command_stats(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<CommandStat>, CacheViewError> {
    let entries = super::keyspace::list_entries(state, ctx)?;
    let n = entries.len() as u64;
    Ok(vec![
        CommandStat { command: "GET",    calls: n * 100, usec_total: n * 1500,  usec_per_call: 15 },
        CommandStat { command: "SET",    calls: n * 20,  usec_total: n * 800,   usec_per_call: 40 },
        CommandStat { command: "DEL",    calls: n * 3,   usec_total: n * 30,    usec_per_call: 10 },
        CommandStat { command: "EXPIRE", calls: n * 5,   usec_total: n * 60,    usec_per_call: 12 },
        CommandStat { command: "MGET",   calls: n,       usec_total: n * 25,    usec_per_call: 25 },
        CommandStat { command: "HGETALL",calls: n / 2,   usec_total: n * 30,    usec_per_call: 60 },
    ])
}

pub fn list_slowlog(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<SlowlogRow>, CacheViewError> {
    let entries = super::keyspace::list_entries(state, ctx)?;
    Ok(entries
        .iter()
        .enumerate()
        .map(|(i, e)| SlowlogRow {
            id: i as u64,
            timestamp_unix: 1_700_000_000 + (i as i64) * 60,
            duration_usec: 10_000 + (e.size_bytes as u64),
            command: format!("HGETALL {}/{}", e.namespace, e.key),
        })
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let stats = list_command_stats(state, ctx)?;
    let slow = list_slowlog(state, ctx)?;
    let stat_rows: Vec<Vec<String>> = stats
        .iter()
        .map(|s| {
            vec![
                s.command.into(),
                s.calls.to_string(),
                s.usec_total.to_string(),
                s.usec_per_call.to_string(),
            ]
        })
        .collect();
    let slow_rows: Vec<Vec<String>> = slow
        .iter()
        .map(|s| {
            vec![
                s.id.to_string(),
                s.timestamp_unix.to_string(),
                format!("{} µs", s.duration_usec),
                s.command.clone(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-commands" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Commands</h2>
  <h3 class="text-md font-semibold mt-2 mb-1">INFO commandstats</h3>
  {stats_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">SLOWLOG ({n})</h3>
  {slow_tbl}
</section>"#,
        n = slow.len(),
        stats_tbl = table(
            &["command", "calls", "usec total", "usec / call"],
            &stat_rows
        ),
        slow_tbl = table(
            &["id", "time", "duration", "command"],
            &slow_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_command_stats_includes_canonical_commands() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/redis-cache/src/components/CommandStats.tsx",
            "CommandStats",
            "acme"
        );
        let stats = list_command_stats(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        let cmds: Vec<_> = stats.iter().map(|s| s.command).collect();
        for c in ["GET", "SET", "DEL", "EXPIRE", "MGET", "HGETALL"] {
            assert!(cmds.contains(&c));
        }
    }

    #[test]
    fn list_slowlog_orders_by_id() {
        let slow = list_slowlog(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        for w in slow.windows(2) {
            assert!(w[0].id < w[1].id);
        }
    }

    #[test]
    fn refuses_without_cache_read() {
        assert!(list_command_stats(&AdminState::seeded(), &ctx(&[])).is_err());
        assert!(list_slowlog(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_emits_both_subsections() {
        let html =
            render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(html.contains("INFO commandstats"));
        assert!(html.contains("SLOWLOG"));
    }
}
