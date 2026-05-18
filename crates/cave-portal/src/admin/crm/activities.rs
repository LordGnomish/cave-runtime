// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/crm/activities` — Twenty CRM "Activities" tab.
//! Synthesises an activity-log row per account from MRR
//! magnitude — high-MRR accounts get a `BUSINESS_REVIEW`
//! activity, mid-MRR get `CHECK_IN`, free accounts get
//! `ONBOARDING`.
//!
//! Upstream: <https://twenty.com/docs>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;
use super::CrmViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    pub account: String,
    pub activity_type: &'static str,
}

pub fn list_activities(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ActivityRow>, CrmViewError> {
    let contacts = super::contacts::list_contacts(state, ctx)?;
    Ok(contacts
        .iter()
        .map(|a| ActivityRow {
            account: a.name.clone(),
            activity_type: if a.mrr_cents >= 100_000 {
                "BUSINESS_REVIEW"
            } else if a.mrr_cents > 0 {
                "CHECK_IN"
            } else {
                "ONBOARDING"
            },
        })
        .collect())
}

pub fn count_by_type(rows: &[ActivityRow]) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.activity_type).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CrmViewError> {
    let rows = list_activities(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![escape(&r.account), r.activity_type.to_string()])
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Activities ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Per-account next-touch. Upstream:
    <a class="text-blue-700 underline" href="https://twenty.com/docs">Twenty CRM Activities</a>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["account", "activity_type"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/crm/activities", &format!("crm/activities · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_one_activity_per_contact() {
        let contacts = super::super::contacts::list_contacts(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let activities = list_activities(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert_eq!(activities.len(), contacts.len());
    }

    #[test]
    fn high_mrr_gets_business_review() {
        let rows = list_activities(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        // Acme Robotics has mrr_cents=1_000_000 → BUSINESS_REVIEW.
        assert!(rows.iter().any(|r| r.account == "Acme Robotics" && r.activity_type == "BUSINESS_REVIEW"));
    }

    #[test]
    fn count_by_type_groups_by_kind() {
        let rows = list_activities(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let total: usize = count_by_type(&rows).values().sum();
        assert_eq!(total, rows.len());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_activities(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_activity_count() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("Activities ("));
        assert!(html.contains("Twenty CRM"));
    }
}
