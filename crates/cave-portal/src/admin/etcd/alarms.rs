// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alarms tab — `etcdctl alarm list` parity (NOSPACE / CORRUPT).

use super::EtcdViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmRow {
    pub member_id: &'static str,
    pub kind: &'static str, // "NOSPACE" | "CORRUPT"
    pub raised_at_unix: i64,
    pub message: &'static str,
}

pub fn list_alarms(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<AlarmRow>, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    // Derive from KV size: large stores get a synthetic NOSPACE warning.
    let kv = super::keyspace::list_kv(state, ctx)?;
    let total_value_bytes: u64 = kv.iter().map(|r| r.value.len() as u64).sum();
    let mut out = Vec::new();
    if total_value_bytes > 100_000 {
        out.push(AlarmRow {
            member_id: "8211f1d0f64f3269",
            kind: "NOSPACE",
            raised_at_unix: 1_700_000_000,
            message: "Storage size > 100KB threshold; compact + defrag recommended.",
        });
    }
    Ok(out)
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, EtcdViewError> {
    let rows = list_alarms(state, ctx)?;
    if rows.is_empty() {
        return Ok(r#"<section id="etcd-alarms" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Alarms (0)</h2>
  <p class="text-sm text-green-700">No active alarms.</p>
</section>"#
            .into());
    }
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|a| {
            vec![
                a.member_id.into(),
                a.kind.into(),
                a.raised_at_unix.to_string(),
                a.message.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="etcd-alarms" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Alarms ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["member", "kind", "raised at", "message"], &table_rows),
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
    fn list_alarms_empty_for_small_seed() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "Alarms",
            "acme"
        );
        let s = AdminState::seeded();
        let alarms = list_alarms(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        assert!(alarms.is_empty(), "seed is small; no NOSPACE alarm");
    }

    #[test]
    fn list_alarms_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_alarms(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_shows_green_when_empty() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        assert!(html.contains("No active alarms"));
    }
}
