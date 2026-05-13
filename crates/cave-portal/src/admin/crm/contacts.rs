//! `/admin/crm/contacts` — Twenty CRM "Contacts" tab. cave-portal
//! treats one row per `CrmAccount` as a primary contact (the
//! account-owner-of-record), surfacing plan tier and MRR.
//!
//! Upstream: <https://twenty.com/docs>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, CrmAccount};
use super::CrmViewError;

pub fn list_contacts(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<CrmAccount>, CrmViewError> {
    ctx.authorise(Permission::CrmRead)?;
    let mut rows: Vec<CrmAccount> =
        scope(&state.crm_accounts.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub fn count_by_plan(rows: &[CrmAccount]) -> std::collections::BTreeMap<&'static str, usize> {
    let mut acc = std::collections::BTreeMap::new();
    for r in rows {
        *acc.entry(r.plan).or_insert(0) += 1;
    }
    acc
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, CrmViewError> {
    let rows = list_contacts(state, ctx)?;
    let by_plan = count_by_plan(&rows);
    let chips: String = by_plan
        .iter()
        .map(|(p, n)| format!(
            r#"<span class="px-2 py-1 mr-2 rounded bg-pink-100 text-sm">{p} <strong>×{n}</strong></span>"#,
            p = p, n = n,
        ))
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.id),
                escape(&r.name),
                r.plan.to_string(),
                (r.mrr_cents as f64 / 100.0).to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Contacts ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">
    Account-owners-of-record. Upstream:
    <a class="text-blue-700 underline" href="https://twenty.com/docs">Twenty CRM</a>.
  </p>
  {tbl}
</section>"#,
        chips = chips,
        n = rows.len(),
        tbl = table(&["id", "name", "plan", "mrr"], &table_rows),
    );
    Ok(page_shell(&format!("crm/contacts · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_tenant() {
        let rows = list_contacts(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_sorted_by_name() {
        let rows = list_contacts(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].name <= w[1].name);
        }
    }

    #[test]
    fn count_by_plan_sums_to_total() {
        let rows = list_contacts(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        let by_plan = count_by_plan(&rows);
        let total: usize = by_plan.values().sum();
        assert_eq!(total, rows.len());
    }

    #[test]
    fn list_rejects_without_permission() {
        assert!(list_contacts(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_contacts_count() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::CrmRead])).unwrap();
        assert!(html.contains("Contacts ("));
        assert!(html.contains("Twenty CRM"));
    }
}
