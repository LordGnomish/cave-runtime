// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/erp/inventory` — ERPNext "Stock" tab. cave-portal
//! doesn't track items independently; we synthesise an inventory
//! row per distinct customer-line on the invoice list (each
//! invoice ≈ one fulfilled item).
//!
//! Upstream: <https://docs.erpnext.com/docs/v15/user/manual/en/stock>

use super::ErpViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockRow {
    pub customer: String,
    pub units_sold: u64,
    pub revenue_cents: u64,
}

pub fn list_stock(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<StockRow>, ErpViewError> {
    let invoices = super::invoices::list_invoices(state, ctx)?;
    let mut acc: std::collections::BTreeMap<String, StockRow> = std::collections::BTreeMap::new();
    for inv in &invoices {
        let entry = acc.entry(inv.customer.clone()).or_insert(StockRow {
            customer: inv.customer.clone(),
            units_sold: 0,
            revenue_cents: 0,
        });
        entry.units_sold += 1;
        entry.revenue_cents += inv.amount_cents;
    }
    Ok(acc.into_values().collect())
}

pub fn top_customer_by_revenue(rows: &[StockRow]) -> Option<&StockRow> {
    rows.iter().max_by_key(|r| r.revenue_cents)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ErpViewError> {
    let rows = list_stock(state, ctx)?;
    let top = top_customer_by_revenue(&rows)
        .map(|r| format!("Top: {} (${})", escape(&r.customer), r.revenue_cents / 100))
        .unwrap_or_else(|| "Top: —".to_string());
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.customer),
                r.units_sold.to_string(),
                (r.revenue_cents as f64 / 100.0).to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Inventory — {top}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-customer fulfilment view. Upstream:
    <a class="text-blue-700 underline" href="https://docs.erpnext.com/docs/v15/user/manual/en/stock">ERPNext Stock</a>.
  </p>
  {tbl}
</section>"#,
        top = top,
        tbl = table(&["customer", "units_sold", "revenue"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/erp/inventory",
        &format!("erp/inventory · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_groups_by_customer() {
        let invoices = super::super::invoices::list_invoices(
            &AdminState::seeded(),
            &ctx(&[Permission::ErpRead]),
        )
        .unwrap();
        let rows = list_stock(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let customers: std::collections::HashSet<_> =
            invoices.iter().map(|i| i.customer.clone()).collect();
        assert_eq!(rows.len(), customers.len());
    }

    #[test]
    fn list_revenue_matches_invoice_amounts() {
        let invoices = super::super::invoices::list_invoices(
            &AdminState::seeded(),
            &ctx(&[Permission::ErpRead]),
        )
        .unwrap();
        let rows = list_stock(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let invoice_total: u64 = invoices.iter().map(|i| i.amount_cents).sum();
        let stock_total: u64 = rows.iter().map(|r| r.revenue_cents).sum();
        assert_eq!(invoice_total, stock_total);
    }

    #[test]
    fn top_customer_is_highest_revenue() {
        let rows = list_stock(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        if let Some(top) = top_customer_by_revenue(&rows) {
            assert!(rows.iter().all(|r| r.revenue_cents <= top.revenue_cents));
        }
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_stock(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_stock_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("Inventory"));
        assert!(html.contains("ERPNext Stock"));
    }
}
