// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/erp` — ERPNext parity. Mirrors the upstream's Invoices
//! landing page with status-group pills + totals.
//!
//! Tab layout — mirrors the ERPNext sidebar:
//!
//! * [`invoices`]   — Sales Invoice list
//! * [`inventory`]  — Stock view, per customer
//! * [`accounting`] — AR waterfall by status
//! * [`hr`]         — synthesised relationship-manager directory
//! * [`projects`]   — per-customer roll-up
//!
//! Upstream UI: <https://erpnext.com/>

pub mod accounting;
pub mod hr;
pub mod inventory;
pub mod invoices;
pub mod projects;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, ErpInvoice, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ErpViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ErpInvoice>, ErpViewError> {
    ctx.authorise(Permission::ErpRead)?;
    let mut rows: Vec<ErpInvoice> = scope(&state.erp_invoices.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| {
        b.amount_cents
            .cmp(&a.amount_cents)
            .then(a.invoice_id.cmp(&b.invoice_id))
    });
    Ok(rows)
}

pub fn group_by_status(rows: &[ErpInvoice]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.status.to_string()).or_insert(0) += 1;
    }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

pub fn total_outstanding_cents(rows: &[ErpInvoice]) -> u64 {
    rows.iter()
        .filter(|r| r.status != "Paid")
        .map(|r| r.amount_cents)
        .sum()
}

pub fn by_status<'a>(rows: &'a [ErpInvoice], status: &str) -> Vec<&'a ErpInvoice> {
    rows.iter().filter(|r| r.status == status).collect()
}

pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    invoice_id: &str,
) -> Result<Option<ErpInvoice>, ErpViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.invoice_id == invoice_id))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ErpViewError> {
    let rows = list_records(state, ctx)?;
    let outstanding = total_outstanding_cents(&rows);
    let groups = group_by_status(&rows);
    let chips: String = groups.iter().map(|(s, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
        s = escape(s), n = n
    )).collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.invoice_id),
                escape(&r.customer),
                format!("${}.{:02}", r.amount_cents / 100, r.amount_cents % 100),
                r.status.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">ERPNext parity (cave-erp). Upstream: <a class="text-blue-700 underline" href="https://erpnext.com/">erpnext.com</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> invoices</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>${od}.{oc:02}</strong> outstanding</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Invoices ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        od = outstanding / 100,
        oc = outstanding % 100,
        chips = chips,
        tbl = table(&["invoice", "customer", "amount", "status"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/erp",
        &format!("erp · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/erp/src/components/InvoicesList.tsx",
    "InvoicesList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner_and_sorts_by_amount_desc() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/erp/src/components/InvoicesList.tsx",
            "InvoicesList",
            "acme"
        );
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) {
            assert!(w[0].amount_cents >= w[1].amount_cents);
        }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_status_counts() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let g = group_by_status(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn total_outstanding_excludes_paid() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let expected: u64 = r
            .iter()
            .filter(|x| x.status != "Paid")
            .map(|x| x.amount_cents)
            .sum();
        assert_eq!(total_outstanding_cents(&r), expected);
    }

    #[test]
    fn by_status_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let paid = by_status(&r, "Paid");
        assert!(paid.iter().all(|x| x.status == "Paid"));
        assert!(by_status(&r, "Bogus").is_empty());
    }

    #[test]
    fn detail_returns_invoice_by_id() {
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::ErpRead])).unwrap();
        if let Some(f) = r.first() {
            assert!(
                detail(&s, &ctx(&[Permission::ErpRead]), &f.invoice_id)
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            detail(&s, &ctx(&[Permission::ErpRead]), "no-such")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("INV-001"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(!html.contains("EVIL-INV"));
    }

    #[test]
    fn render_includes_outstanding_total_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("outstanding"));
        assert!(html.contains("erpnext.com"));
    }
}
