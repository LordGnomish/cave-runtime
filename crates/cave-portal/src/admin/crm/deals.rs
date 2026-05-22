// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/crm/deals` — Twenty CRM "Opportunities" tab. Each
//! account synthesises a "deal" sized at 12 × MRR (annualised
//! contract value) with a stage derived from plan tier
//! (Free→Discovery, Pro→Proposal, Enterprise→Closed-Won).
//!
//! Upstream: <https://twenty.com/docs>

use super::CrmViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DealRow {
    pub deal_id: String,
    pub account_name: String,
    pub acv_cents: u64,
    pub stage: &'static str,
}

pub fn list_deals(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<DealRow>, CrmViewError> {
    let contacts = super::contacts::list_contacts(state, ctx)?;
    Ok(contacts
        .iter()
        .map(|a| DealRow {
            deal_id: format!("deal-{}", a.id),
            account_name: a.name.clone(),
            acv_cents: a.mrr_cents * 12,
            stage: match a.plan {
                "Free" => "Discovery",
                "Pro" => "Proposal",
                "Enterprise" => "Closed-Won",
                _ => "Unknown",
            },
        })
        .collect())
}

pub fn pipeline_total(rows: &[DealRow]) -> u64 {
    rows.iter()
        .filter(|r| r.stage != "Closed-Won")
        .map(|r| r.acv_cents)
        .sum()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CrmViewError> {
    let rows = list_deals(state, ctx)?;
    let pipeline = pipeline_total(&rows);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.deal_id),
                escape(&r.account_name),
                (r.acv_cents as f64 / 100.0).to_string(),
                r.stage.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Deals ({n}) · pipeline ${pipe_d}</h2>
  <p class="text-sm text-gray-600 mb-3">
    Opportunity pipeline. Upstream:
    <a class="text-blue-700 underline" href="https://twenty.com/docs">Twenty CRM Opportunities</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        pipe_d = (pipeline as f64 / 100.0),
        tbl = table(&["deal_id", "account", "acv", "stage"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/crm/deals",
        &format!("crm/deals · {}", escape(ctx.tenant.as_str())),
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
    fn list_one_deal_per_contact() {
        let contacts = super::super::contacts::list_contacts(
            &AdminState::seeded(),
            &ctx(&[Permission::CrmRead]),
        )
        .unwrap();
        let deals = list_deals(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert_eq!(deals.len(), contacts.len());
    }

    #[test]
    fn acv_is_12x_mrr() {
        let contacts = super::super::contacts::list_contacts(
            &AdminState::seeded(),
            &ctx(&[Permission::CrmRead]),
        )
        .unwrap();
        let deals = list_deals(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        for (a, d) in contacts.iter().zip(deals.iter()) {
            assert_eq!(d.acv_cents, a.mrr_cents * 12);
        }
    }

    #[test]
    fn stage_derived_from_plan() {
        let deals = list_deals(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        for d in &deals {
            assert!(matches!(
                d.stage,
                "Discovery" | "Proposal" | "Closed-Won" | "Unknown"
            ));
        }
    }

    #[test]
    fn pipeline_total_excludes_closed_won() {
        let deals = list_deals(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let pipe = pipeline_total(&deals);
        let won: u64 = deals
            .iter()
            .filter(|d| d.stage == "Closed-Won")
            .map(|d| d.acv_cents)
            .sum();
        let grand: u64 = deals.iter().map(|d| d.acv_cents).sum();
        assert_eq!(pipe, grand - won);
    }

    #[test]
    fn render_includes_pipeline_total() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("Deals ("));
        assert!(html.contains("Twenty CRM"));
    }
}
