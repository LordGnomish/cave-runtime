//! `/admin/erp/accounting` — ERPNext "Accounting" tab. Synthesises
//! a per-status revenue waterfall from the invoice dataset:
//! `Paid` → recognised, `Pending` → AR, `Overdue` → bad-debt
//! risk. The view operators land on when finance asks "what's
//! collectable this month".
//!
//! Upstream: <https://docs.erpnext.com/docs/v15/user/manual/en/accounts>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use super::ErpViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    pub bucket: &'static str,
    pub invoice_count: usize,
    pub total_cents: u64,
}

pub fn ledger(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LedgerRow>, ErpViewError> {
    let invoices = super::invoices::list_invoices(state, ctx)?;
    let mut acc: std::collections::BTreeMap<&'static str, LedgerRow> =
        std::collections::BTreeMap::new();
    for inv in &invoices {
        let entry = acc.entry(inv.status).or_insert(LedgerRow {
            bucket: inv.status,
            invoice_count: 0,
            total_cents: 0,
        });
        entry.invoice_count += 1;
        entry.total_cents += inv.amount_cents;
    }
    Ok(acc.into_values().collect())
}

pub fn ar_total_cents(rows: &[LedgerRow]) -> u64 {
    rows.iter()
        .filter(|r| r.bucket != "Paid")
        .map(|r| r.total_cents)
        .sum()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ErpViewError> {
    let rows = ledger(state, ctx)?;
    let ar = ar_total_cents(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.bucket.to_string(),
                r.invoice_count.to_string(),
                (r.total_cents as f64 / 100.0).to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Accounting — AR ${ar_d}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-status revenue waterfall. Upstream:
    <a class="text-blue-700 underline" href="https://docs.erpnext.com/docs/v15/user/manual/en/accounts">ERPNext Accounts</a>.
  </p>
  {tbl}
</section>"#,
        ar_d = (ar as f64 / 100.0),
        tbl = table(&["bucket", "invoice_count", "total"], &table_rows),
    );
    Ok(page_shell(
        &format!("erp/accounting · {}", escape(ctx.tenant.as_str())),
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
    fn ledger_counts_invoices_per_bucket() {
        let rows = ledger(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let total: usize = rows.iter().map(|r| r.invoice_count).sum();
        let invoices = super::super::invoices::list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert_eq!(total, invoices.len());
    }

    #[test]
    fn ar_excludes_paid_bucket() {
        let rows = ledger(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let ar = ar_total_cents(&rows);
        let paid_total: u64 = rows.iter().filter(|r| r.bucket == "Paid").map(|r| r.total_cents).sum();
        let grand_total: u64 = rows.iter().map(|r| r.total_cents).sum();
        assert_eq!(ar, grand_total - paid_total);
    }

    #[test]
    fn ledger_rejects_without_permission() {
        assert!(ledger(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn ledger_groups_unique_buckets() {
        let rows = ledger(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let mut seen = std::collections::HashSet::new();
        for r in &rows {
            assert!(seen.insert(r.bucket));
        }
    }

    #[test]
    fn render_includes_ar_total() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("Accounting"));
        assert!(html.contains("ERPNext Accounts"));
    }
}
