// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/crm/workflows` — Twenty CRM "Workflows" tab. Lists
//! the lifecycle automation operators have configured. cave-portal
//! exposes a fixed canonical set keyed off the plan tiers
//! (Enterprise/Pro/Free) — same shape as a real Twenty install
//! after first-run setup.
//!
//! Upstream: <https://twenty.com/docs>

use super::CrmViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRow {
    pub workflow_id: &'static str,
    pub trigger: &'static str,
    pub action: &'static str,
    pub matching_accounts: usize,
}

pub fn list_workflows(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<WorkflowRow>, CrmViewError> {
    let contacts = super::contacts::list_contacts(state, ctx)?;
    let n_enterprise = contacts.iter().filter(|a| a.plan == "Enterprise").count();
    let n_pro = contacts.iter().filter(|a| a.plan == "Pro").count();
    let n_free = contacts.iter().filter(|a| a.plan == "Free").count();
    Ok(vec![
        WorkflowRow {
            workflow_id: "wf-enterprise-qbr",
            trigger: "schedule:quarterly",
            action: "create_activity:BUSINESS_REVIEW",
            matching_accounts: n_enterprise,
        },
        WorkflowRow {
            workflow_id: "wf-pro-checkin",
            trigger: "schedule:monthly",
            action: "create_activity:CHECK_IN",
            matching_accounts: n_pro,
        },
        WorkflowRow {
            workflow_id: "wf-free-upsell",
            trigger: "mrr_cents == 0",
            action: "create_activity:ONBOARDING",
            matching_accounts: n_free,
        },
    ])
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CrmViewError> {
    let rows = list_workflows(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.workflow_id.to_string(),
                r.trigger.to_string(),
                r.action.to_string(),
                r.matching_accounts.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Workflows ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Lifecycle automation. Upstream:
    <a class="text-blue-700 underline" href="https://twenty.com/docs">Twenty CRM Workflows</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["workflow_id", "trigger", "action", "match"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/crm/workflows",
        &format!("crm/workflows · {}", escape(ctx.tenant.as_str())),
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
    fn list_returns_three_canonical_workflows() {
        let rows = list_workflows(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn matching_accounts_sum_to_contact_count() {
        let contacts = super::super::contacts::list_contacts(
            &AdminState::seeded(),
            &ctx(&[Permission::CrmRead]),
        )
        .unwrap();
        let rows = list_workflows(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let total: usize = rows.iter().map(|w| w.matching_accounts).sum();
        assert_eq!(total, contacts.len());
    }

    #[test]
    fn enterprise_workflow_present() {
        let rows = list_workflows(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(rows.iter().any(|w| w.workflow_id == "wf-enterprise-qbr"));
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_workflows(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_lists_workflows_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("Workflows ("));
        assert!(html.contains("wf-enterprise-qbr"));
    }
}
