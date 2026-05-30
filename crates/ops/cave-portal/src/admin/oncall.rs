// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/oncall` — Grafana OnCall parity. Schedule grid grouped by
//! rotation with active-shift filter.
//!
//! Upstream UI: <https://grafana.com/docs/oncall/latest/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, OncallShift, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OncallViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<OncallShift>, OncallViewError> {
    ctx.authorise(Permission::OncallRead)?;
    let mut rows: Vec<OncallShift> =
        scope(&state.oncall_shifts.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        a.start_unix
            .cmp(&b.start_unix)
            .then(a.rotation.cmp(&b.rotation))
    });
    Ok(rows)
}

pub fn group_by_rotation(rows: &[OncallShift]) -> Vec<(String, Vec<OncallShift>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<OncallShift>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.rotation.clone()).or_default().push(r.clone());
    }
    acc.into_iter().collect()
}

/// Shifts active at `at_unix` (start ≤ at < end). Mirrors OnCall's
/// "who's on right now?" header.
pub fn active_at<'a>(rows: &'a [OncallShift], at_unix: i64) -> Vec<&'a OncallShift> {
    rows.iter()
        .filter(|s| s.start_unix <= at_unix && at_unix < s.end_unix)
        .collect()
}

/// Per-source webhook receivers cave-oncall normalizes — mirrors
/// `cave_oncall::integrations::IntegrationType::all()`. Surfaced read-only so
/// operators can see which monitoring sources can page this stack.
pub fn supported_integrations() -> [&'static str; 5] {
    [
        "alertmanager",
        "grafana_alerting",
        "grafana",
        "formatted_webhook",
        "webhook",
    ]
}

/// The Grafana OnCall basic-role ladder (LegacyAccessControlRole), surfaced
/// read-only. Mirrors `cave_oncall::rbac::Role` — lower value = more access.
pub fn role_ladder() -> [(&'static str, u8); 4] {
    [("admin", 0), ("editor", 1), ("viewer", 2), ("none", 3)]
}

pub fn unique_oncallers(rows: &[OncallShift]) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut set: BTreeSet<String> = BTreeSet::new();
    for r in rows {
        set.insert(r.oncaller.clone());
    }
    set.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, OncallViewError> {
    let rows = list_records(state, ctx)?;
    let oncallers = unique_oncallers(&rows);
    let groups = group_by_rotation(&rows);
    let chips: String = groups.iter().map(|(r, v)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{r} <strong>×{n}</strong></span>"#,
        r = escape(r), n = v.len())).collect();
    let integration_chips: String = supported_integrations()
        .iter()
        .map(|s| format!(
            r#"<span class="px-2 py-1 mr-2 rounded bg-blue-100 text-blue-800 text-sm font-mono">{}</span>"#,
            escape(s)
        ))
        .collect();
    let role_chips: String = role_ladder()
        .iter()
        .map(|(name, lvl)| format!(
            r#"<span class="px-2 py-1 mr-2 rounded bg-amber-100 text-amber-800 text-sm">{} <strong>({})</strong></span>"#,
            escape(name), lvl
        ))
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.rotation),
                escape(&r.oncaller),
                r.start_unix.to_string(),
                r.end_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Grafana OnCall parity (cave-oncall). Upstream: <a class="text-blue-700 underline" href="https://grafana.com/docs/oncall/latest/">grafana.com/docs/oncall</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> shifts</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{u}</strong> oncallers</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Alert integrations</h2>
  <div class="mb-4">{integrations}</div>
  <h2 class="text-lg font-semibold mb-2">Access roles</h2>
  <div class="mb-4">{roles}</div>
  <h2 class="text-lg font-semibold mb-2">Shifts ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        u = oncallers.len(),
        chips = chips,
        integrations = integration_chips,
        roles = role_chips,
        tbl = table(&["rotation", "oncaller", "start", "end"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/oncall",
        &format!("oncall · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/oncall/src/components/ShiftsList.tsx", "ShiftsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner_and_sorts_by_start() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) {
            assert!(w[0].start_unix <= w[1].start_unix);
        }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_rotation_collects() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        let g = group_by_rotation(&r);
        assert_eq!(g.iter().map(|(_, v)| v.len()).sum::<usize>(), r.len());
    }

    #[test]
    fn active_at_window_inclusive_start_exclusive_end() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        if let Some(s) = r.first() {
            let at = s.start_unix;
            assert!(active_at(&r, at).iter().any(|x| x.start_unix == at));
            assert!(
                active_at(&r, s.end_unix)
                    .iter()
                    .all(|x| x.start_unix != at || x.end_unix > s.end_unix)
            );
        }
        assert!(active_at(&r, i64::MIN).is_empty());
    }

    #[test]
    fn unique_oncallers_dedup() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        let u = unique_oncallers(&r);
        let names: std::collections::BTreeSet<&str> =
            r.iter().map(|s| s.oncaller.as_str()).collect();
        assert_eq!(u.len(), names.len());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        assert!(html.contains("sre-primary"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        assert!(!html.contains("evil-rotation"));
    }

    #[test]
    fn render_lists_supported_integrations() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        assert!(html.contains("Alert integrations"));
        for slug in supported_integrations() {
            assert!(html.contains(slug), "integration {slug} missing from page");
        }
    }

    #[test]
    fn render_lists_access_roles() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        assert!(html.contains("Access roles"));
        for (name, _) in role_ladder() {
            assert!(html.contains(name), "role {name} missing from page");
        }
    }

    #[test]
    fn render_includes_oncaller_count_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::OncallRead])).unwrap();
        assert!(html.contains("oncallers"));
        assert!(html.contains("grafana.com/docs/oncall"));
    }
}
