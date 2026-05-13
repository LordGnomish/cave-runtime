//! `/admin/crm/reports` — Twenty CRM "Reports" tab. Computes a
//! per-plan MRR roll-up + total ARR for the operator.
//!
//! Upstream: <https://twenty.com/docs>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use super::CrmViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub plan: &'static str,
    pub account_count: usize,
    pub mrr_cents: u64,
    /// arr = mrr × 12, kept explicit so the caller doesn't have to do the
    /// multiplication themselves and risk a unit confusion.
    pub arr_cents: u64,
}

pub fn build_reports(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ReportRow>, CrmViewError> {
    let contacts = super::contacts::list_contacts(state, ctx)?;
    let mut acc: std::collections::BTreeMap<&'static str, ReportRow> = std::collections::BTreeMap::new();
    for a in &contacts {
        let r = acc.entry(a.plan).or_insert(ReportRow {
            plan: a.plan,
            account_count: 0,
            mrr_cents: 0,
            arr_cents: 0,
        });
        r.account_count += 1;
        r.mrr_cents += a.mrr_cents;
        r.arr_cents += a.mrr_cents * 12;
    }
    Ok(acc.into_values().collect())
}

pub fn total_arr(rows: &[ReportRow]) -> u64 {
    rows.iter().map(|r| r.arr_cents).sum()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CrmViewError> {
    let rows = build_reports(state, ctx)?;
    let arr = total_arr(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.plan.to_string(),
                r.account_count.to_string(),
                (r.mrr_cents as f64 / 100.0).to_string(),
                (r.arr_cents as f64 / 100.0).to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Reports — total ARR ${arr_d}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-plan revenue roll-up. Upstream:
    <a class="text-blue-700 underline" href="https://twenty.com/docs">Twenty CRM Reports</a>.
  </p>
  {tbl}
</section>"#,
        arr_d = (arr as f64 / 100.0),
        tbl = table(&["plan", "accounts", "mrr", "arr"], &table_rows),
    );
    Ok(page_shell(&format!("crm/reports · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn reports_grouped_by_plan() {
        let rows = build_reports(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let plans: std::collections::HashSet<_> = rows.iter().map(|r| r.plan).collect();
        assert_eq!(plans.len(), rows.len());
    }

    #[test]
    fn arr_is_12x_mrr() {
        let rows = build_reports(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        for r in &rows {
            assert_eq!(r.arr_cents, r.mrr_cents * 12);
        }
    }

    #[test]
    fn total_arr_sums_per_row() {
        let rows = build_reports(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let expected: u64 = rows.iter().map(|r| r.arr_cents).sum();
        assert_eq!(total_arr(&rows), expected);
    }

    #[test]
    fn reports_rejects_without_permission() {
        assert!(build_reports(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_arr_label() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("Reports"));
        assert!(html.contains("Twenty CRM Reports"));
    }
}
