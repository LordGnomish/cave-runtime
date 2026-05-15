// SPDX-License-Identifier: AGPL-3.0-or-later
//! Migration tab — active resharding status.

use super::super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMigrationRow {
    pub slot: u16,
    pub direction: &'static str, // "migrating" | "importing"
    pub peer_node_id: &'static str,
    pub keys_total: u64,
    pub keys_migrated: u64,
    pub started_at_unix: i64,
}

impl ActiveMigrationRow {
    pub fn progress_pct(&self) -> u32 {
        if self.keys_total == 0 {
            return 0;
        }
        ((self.keys_migrated as f64 / self.keys_total as f64) * 100.0).min(100.0) as u32
    }
}

pub fn active_migrations(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ActiveMigrationRow>, CacheViewError> {
    ctx.authorise(Permission::CacheRead)?;
    Ok(vec![
        ActiveMigrationRow {
            slot: 1234,
            direction: "migrating",
            peer_node_id: "b2c3d4e5",
            keys_total: 2_000,
            keys_migrated: 1_400,
            started_at_unix: 1_715_749_000,
        },
        ActiveMigrationRow {
            slot: 7_812,
            direction: "importing",
            peer_node_id: "c3d4e5f6",
            keys_total: 5_000,
            keys_migrated: 200,
            started_at_unix: 1_715_749_800,
        },
    ])
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    let migs = active_migrations(state, ctx)?;
    let rows: Vec<Vec<String>> = migs
        .iter()
        .map(|m| {
            vec![
                m.slot.to_string(),
                m.direction.into(),
                m.peer_node_id.into(),
                format!("{}/{}", m.keys_migrated, m.keys_total),
                format!("{}%", m.progress_pct()),
                m.started_at_unix.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cache-cluster-migration" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Resharding ({n} active)</h2>
  {tbl}
</section>"#,
        n = migs.len(),
        tbl = table(
            &["slot", "direction", "peer", "keys", "progress", "started"],
            &rows,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn active_migrations_returns_both_directions() {
        let m = active_migrations(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        let dirs: std::collections::HashSet<&str> = m.iter().map(|x| x.direction).collect();
        assert!(dirs.contains("migrating"));
        assert!(dirs.contains("importing"));
    }

    #[test]
    fn progress_pct_clamps_to_100() {
        let r = ActiveMigrationRow {
            slot: 0,
            direction: "migrating",
            peer_node_id: "x",
            keys_total: 5,
            keys_migrated: 999,
            started_at_unix: 0,
        };
        assert_eq!(r.progress_pct(), 100);
    }

    #[test]
    fn progress_pct_zero_when_total_zero() {
        let r = ActiveMigrationRow {
            slot: 0,
            direction: "migrating",
            peer_node_id: "x",
            keys_total: 0,
            keys_migrated: 0,
            started_at_unix: 0,
        };
        assert_eq!(r.progress_pct(), 0);
    }

    #[test]
    fn render_includes_progress_column() {
        let h = render_section(&AdminState::seeded(), &ctx(&[Permission::CacheRead])).unwrap();
        assert!(h.contains("progress"));
        assert!(h.contains("Resharding"));
    }

    #[test]
    fn refuses_listing_without_cache_read() {
        assert!(active_migrations(&AdminState::seeded(), &ctx(&[])).is_err());
    }
}
