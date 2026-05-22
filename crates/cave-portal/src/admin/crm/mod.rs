// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/crm` — Twenty CRM parity. Mirrors the upstream's
//! Accounts landing page with per-plan grouping and MRR summary card.
//!
//! Tab layout — mirrors the Twenty sidebar:
//!
//! * [`contacts`]   — account-owners-of-record list
//! * [`deals`]      — opportunity pipeline
//! * [`activities`] — per-account next-touch
//! * [`workflows`]  — lifecycle automations
//! * [`reports`]    — per-plan revenue roll-up
//!
//! Upstream UI: <https://twenty.com/>

pub mod activities;
pub mod contacts;
pub mod deals;
pub mod reports;
pub mod workflows;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, CrmAccount, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CrmViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CrmAccount>, CrmViewError> {
    ctx.authorise(Permission::CrmRead)?;
    let mut rows: Vec<CrmAccount> = scope(&state.crm_accounts.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| b.mrr_cents.cmp(&a.mrr_cents).then(a.name.cmp(&b.name)));
    Ok(rows)
}

/// Group accounts by plan tier (Twenty's "segment" axis).
pub fn group_by_plan(rows: &[CrmAccount]) -> Vec<(String, Vec<CrmAccount>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<CrmAccount>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.plan.to_string()).or_default().push(r.clone());
    }
    acc.into_iter().collect()
}

/// Total MRR across all accounts (in cents).
pub fn total_mrr_cents(rows: &[CrmAccount]) -> u64 {
    rows.iter().map(|r| r.mrr_cents).sum()
}

pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    id: &str,
) -> Result<Option<CrmAccount>, CrmViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.id == id))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CrmViewError> {
    let rows = list_records(state, ctx)?;
    let total = total_mrr_cents(&rows);
    let groups = group_by_plan(&rows);
    let chips: String = groups.iter().map(|(p, v)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{p} <strong>×{n}</strong></span>"#,
        p = escape(p), n = v.len()
    )).collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.id),
                escape(&r.name),
                r.plan.into(),
                format!("${}.{:02}", r.mrr_cents / 100, r.mrr_cents % 100),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Twenty CRM parity (cave-crm). Upstream: <a class="text-blue-700 underline" href="https://twenty.com/">twenty.com</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> accounts</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>${total_dollars}.{total_cents:02}</strong> MRR total</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Accounts ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        total_dollars = total / 100,
        total_cents = total % 100,
        chips = chips,
        tbl = table(&["id", "name", "plan", "MRR"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/crm",
        &format!("crm · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/crm/src/components/AccountsList.tsx",
    "AccountsList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner_and_sorts_by_mrr_desc() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/crm/src/components/AccountsList.tsx",
            "AccountsList",
            "acme"
        );
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
        for w in r.windows(2) {
            assert!(w[0].mrr_cents >= w[1].mrr_cents);
        }
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_plan_collects_accounts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let groups = group_by_plan(&r);
        assert_eq!(groups.iter().map(|(_, v)| v.len()).sum::<usize>(), r.len());
    }

    #[test]
    fn total_mrr_cents_sums_all() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let expected: u64 = r.iter().map(|a| a.mrr_cents).sum();
        assert_eq!(total_mrr_cents(&r), expected);
        assert_eq!(total_mrr_cents(&[]), 0);
    }

    #[test]
    fn detail_returns_account_by_id() {
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::CrmRead])).unwrap();
        if let Some(f) = r.first() {
            assert!(
                detail(&s, &ctx(&[Permission::CrmRead]), &f.id)
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            detail(&s, &ctx(&[Permission::CrmRead]), "no-such")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("Acme Robotics"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(!html.contains("Evil Corp"));
    }

    #[test]
    fn render_includes_total_mrr_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("MRR total"));
        assert!(html.contains("twenty.com"));
    }
}
