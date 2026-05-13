//! `/admin/erp/projects` — ERPNext "Projects" tab. Synthesises
//! one project row per customer aggregating their invoice
//! activity — the view operators use to see "what's the
//! Roll-up for this account".
//!
//! Upstream: <https://docs.erpnext.com/docs/v15/user/manual/en/projects>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;
use super::ErpViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub project_id: String,
    pub customer: String,
    pub invoice_count: usize,
    pub committed_cents: u64,
    pub completion_pct: u8,
}

pub fn list_projects(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ProjectRow>, ErpViewError> {
    let invoices = super::invoices::list_invoices(state, ctx)?;
    let mut acc: std::collections::BTreeMap<String, ProjectRow> = std::collections::BTreeMap::new();
    for inv in &invoices {
        let entry = acc.entry(inv.customer.clone()).or_insert(ProjectRow {
            project_id: format!("PROJ-{}", inv.customer.replace(' ', "-").to_uppercase()),
            customer: inv.customer.clone(),
            invoice_count: 0,
            committed_cents: 0,
            completion_pct: 0,
        });
        entry.invoice_count += 1;
        entry.committed_cents += inv.amount_cents;
    }
    // Completion% = paid/total ratio per project — synthesised
    // from invoice status, capped at 100.
    let paid_by_customer: std::collections::HashMap<String, u64> = invoices
        .iter()
        .filter(|i| i.status == "Paid")
        .fold(std::collections::HashMap::new(), |mut acc, i| {
            *acc.entry(i.customer.clone()).or_insert(0) += i.amount_cents;
            acc
        });
    for row in acc.values_mut() {
        let paid = *paid_by_customer.get(&row.customer).unwrap_or(&0);
        row.completion_pct = if row.committed_cents == 0 {
            0
        } else {
            ((paid * 100) / row.committed_cents).min(100) as u8
        };
    }
    Ok(acc.into_values().collect())
}

pub fn fully_complete<'a>(rows: &'a [ProjectRow]) -> Vec<&'a ProjectRow> {
    rows.iter().filter(|r| r.completion_pct == 100).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ErpViewError> {
    let rows = list_projects(state, ctx)?;
    let done = fully_complete(&rows).len();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.project_id),
                escape(&r.customer),
                r.invoice_count.to_string(),
                (r.committed_cents as f64 / 100.0).to_string(),
                format!("{}%", r.completion_pct),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Projects ({n}) · {done} complete</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-customer roll-up. Upstream:
    <a class="text-blue-700 underline" href="https://docs.erpnext.com/docs/v15/user/manual/en/projects">ERPNext Projects</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        done = done,
        tbl = table(
            &["project_id", "customer", "invoices", "committed", "complete%"],
            &table_rows
        ),
    );
    Ok(page_shell(
        &format!("erp/projects · {}", escape(ctx.tenant.as_str())),
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
    fn list_one_project_per_customer() {
        let invoices = super::super::invoices::list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let customers: std::collections::HashSet<_> = invoices.iter().map(|i| i.customer.clone()).collect();
        let rows = list_projects(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert_eq!(rows.len(), customers.len());
    }

    #[test]
    fn project_id_uppercase_normalised() {
        let rows = list_projects(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        for r in &rows {
            assert!(r.project_id.starts_with("PROJ-"));
            assert!(r.project_id.chars().all(|c| !c.is_lowercase()));
        }
    }

    #[test]
    fn completion_capped_at_100() {
        let rows = list_projects(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(rows.iter().all(|r| r.completion_pct <= 100));
    }

    #[test]
    fn committed_matches_invoice_sums() {
        let invoices = super::super::invoices::list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let rows = list_projects(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let inv_total: u64 = invoices.iter().map(|i| i.amount_cents).sum();
        let proj_total: u64 = rows.iter().map(|r| r.committed_cents).sum();
        assert_eq!(inv_total, proj_total);
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_projects(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_projects_count() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("Projects ("));
        assert!(html.contains("ERPNext Projects"));
    }
}
