// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/erp/hr` — ERPNext "HR" tab. cave-portal doesn't
//! manage employees as first-class records, so this tab
//! synthesises a per-customer relationship-manager directory
//! from invoice activity — surfaced as "who's our point of
//! contact and how active are they".
//!
//! Upstream: <https://docs.erpnext.com/docs/v15/user/manual/en/human-resources>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::ErpViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmployeeRow {
    /// Synthesised relationship-manager id: `rm-<customer>`.
    pub employee_id: String,
    pub assigned_customer: String,
    pub active_invoices: usize,
}

pub fn list_employees(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<EmployeeRow>, ErpViewError> {
    let invoices = super::invoices::list_invoices(state, ctx)?;
    let mut acc: std::collections::BTreeMap<String, EmployeeRow> = std::collections::BTreeMap::new();
    for inv in &invoices {
        let entry = acc
            .entry(inv.customer.clone())
            .or_insert_with(|| EmployeeRow {
                employee_id: format!("rm-{}", inv.customer.replace(' ', "-").to_lowercase()),
                assigned_customer: inv.customer.clone(),
                active_invoices: 0,
            });
        entry.active_invoices += 1;
    }
    Ok(acc.into_values().collect())
}

pub fn count_active(rows: &[EmployeeRow]) -> usize {
    rows.iter().filter(|r| r.active_invoices > 0).count()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ErpViewError> {
    let rows = list_employees(state, ctx)?;
    let active = count_active(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.employee_id),
                escape(&r.assigned_customer),
                r.active_invoices.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">HR — {n} relationship managers · {active} active</h2>
  <p class="text-sm text-gray-600 mb-3">
    Customer-facing roster. Upstream:
    <a class="text-blue-700 underline" href="https://docs.erpnext.com/docs/v15/user/manual/en/human-resources">ERPNext HR</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        active = active,
        tbl = table(
            &["employee_id", "customer", "active_invoices"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/erp/hr",
        &format!("erp/hr · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_one_employee_per_customer() {
        let invoices = super::super::invoices::list_invoices(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        let customers: std::collections::HashSet<_> = invoices.iter().map(|i| i.customer.clone()).collect();
        let rows = list_employees(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert_eq!(rows.len(), customers.len());
    }

    #[test]
    fn employee_id_is_normalised_rm_prefix() {
        let rows = list_employees(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(rows.iter().all(|r| r.employee_id.starts_with("rm-")));
    }

    #[test]
    fn count_active_matches_seeded_invoices() {
        let rows = list_employees(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert_eq!(count_active(&rows), rows.len());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_employees(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_links_to_erpnext_hr() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ErpRead])).unwrap();
        assert!(html.contains("HR"));
        assert!(html.contains("ERPNext HR"));
    }
}
