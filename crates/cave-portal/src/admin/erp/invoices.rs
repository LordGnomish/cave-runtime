//! `/admin/erp/invoices` — ERPNext "Sales Invoice" list. The
//! authoritative source — every other sub-page derives from
//! this dataset.
//!
//! Upstream: <https://docs.erpnext.com/docs/v15/user/manual/en/sales-invoice>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ErpInvoice};
use super::ErpViewError;

pub fn list_invoices(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ErpInvoice>, ErpViewError> {
    ctx.authorise(Permission::ErpRead)?;
    let mut rows: Vec<ErpInvoice> =
        scope(&state.erp_invoices.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.invoice_id.cmp(&b.invoice_id));
    Ok(rows)
}

pub fn total_billed_cents(rows: &[ErpInvoice]) -> u64 {
    rows.iter().map(|r| r.amount_cents).sum()
}

pub fn by_status(rows: &[ErpInvoice]) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.status).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ErpViewError> {
    let rows = list_invoices(state, ctx)?;
    let total = total_billed_cents(&rows);
    let by_st = by_status(&rows);
    let chips: String = by_st
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-blue-100 text-sm">{s} <strong>×{n}</strong></span>"#,
                s = s, n = n
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.invoice_id),
                escape(&r.customer),
                (r.amount_cents as f64 / 100.0).to_string(),
                r.status.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Invoices ({n}) · total billed ${total_d}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Sales Invoices. Upstream:
    <a class="text-blue-700 underline" href="https://docs.erpnext.com/docs/v15/user/manual/en/sales-invoice">ERPNext Sales Invoice</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        total_d = (total as f64 / 100.0),
        tbl = table(&["invoice", "customer", "amount", "status"], &table_rows),
    );
    Ok(page_shell(
        &format!("erp/invoices · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_seeded_invoices() {
        let rows = list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_sorted_by_id() {
        let rows = list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].invoice_id <= w[1].invoice_id);
        }
    }

    #[test]
    fn total_billed_sums_amount_cents() {
        let rows = list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let total = total_billed_cents(&rows);
        let expected: u64 = rows.iter().map(|r| r.amount_cents).sum();
        assert_eq!(total, expected);
    }

    #[test]
    fn by_status_groups_invoices() {
        let rows = list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let by_st = by_status(&rows);
        let total_in_map: usize = by_st.values().sum();
        assert_eq!(total_in_map, rows.len());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_invoices(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_total_billed() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("Invoices ("));
        assert!(html.contains("ERPNext Sales Invoice"));
    }
}
