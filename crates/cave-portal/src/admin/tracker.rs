// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/tracker` — Linear / Plane parity. Issue browser grouped
//! by state with assignee summary.
//!
//! Upstream UI: <https://linear.app/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, TrackerIssue, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TrackerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<TrackerIssue>, TrackerViewError> {
    ctx.authorise(Permission::TrackerRead)?;
    let mut rows: Vec<TrackerIssue> =
        scope(&state.tracker_issues.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.state.cmp(b.state).then(a.id.cmp(&b.id)));
    Ok(rows)
}

pub fn group_by_state(rows: &[TrackerIssue]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.state.to_string()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn unassigned<'a>(rows: &'a [TrackerIssue]) -> Vec<&'a TrackerIssue> {
    rows.iter().filter(|r| r.assignee.is_none()).collect()
}

pub fn by_assignee<'a>(rows: &'a [TrackerIssue], assignee: &str) -> Vec<&'a TrackerIssue> {
    rows.iter()
        .filter(|r| r.assignee.as_deref() == Some(assignee))
        .collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, TrackerViewError> {
    let rows = list_records(state, ctx)?;
    let groups = group_by_state(&rows);
    let unassigned_n = unassigned(&rows).len();
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n)).collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.id),
                escape(&r.title),
                r.state.into(),
                r.assignee.clone().unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Linear / Plane (cave-tracker). Upstream: <a class="text-blue-700 underline" href="https://linear.app/">linear.app</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> issues</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{u}</strong> unassigned</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Issues ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        u = unassigned_n,
        chips = chips,
        tbl = table(&["id", "title", "state", "assignee"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/tracker",
        &format!("tracker · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/tracker/src/components/IssuesList.tsx",
    "IssuesList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner_sorted_by_state() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) {
            assert!(w[0].state <= w[1].state || w[0].id.as_str() <= w[1].id.as_str());
        }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_state_counts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        let g = group_by_state(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn unassigned_filters_no_assignee() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        let u = unassigned(&r);
        assert!(u.iter().all(|x| x.assignee.is_none()));
    }

    #[test]
    fn by_assignee_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        if let Some(a) = r.iter().find_map(|x| x.assignee.as_deref()) {
            let an = a.to_string();
            assert!(
                by_assignee(&r, &an)
                    .iter()
                    .all(|x| x.assignee.as_deref() == Some(an.as_str()))
            );
        }
        assert!(by_assignee(&r, "ghost").is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        assert!(html.contains("ISS-100"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        assert!(!html.contains("EVIL-1"));
    }

    #[test]
    fn render_includes_unassigned_count_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::TrackerRead])).unwrap();
        assert!(html.contains("unassigned"));
        assert!(html.contains("linear.app"));
    }
}
