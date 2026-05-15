//! Budgets sub-page.

use super::types::{LiteLlmBudget, LiteLlmViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmBudget>, LiteLlmViewError> {
    ctx.authorise(Permission::LiteLlmRead)?;
    let mut rows: Vec<LiteLlmBudget> =
        scope(&state.litellm_budgets.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub fn percent_used(b: &LiteLlmBudget) -> u8 {
    if b.limit_usd_cents == 0 {
        return 0;
    }
    ((b.spent_usd_cents.min(b.limit_usd_cents) * 100) / b.limit_usd_cents) as u8
}

pub fn breaching<'a>(rows: &'a [LiteLlmBudget]) -> Vec<&'a LiteLlmBudget> {
    rows.iter().filter(|b| percent_used(b) >= b.alert_threshold_pct).collect()
}

pub fn scope_histogram(rows: &[LiteLlmBudget]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.scope.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LiteLlmViewError> {
    let rows = list(state, ctx)?;
    let breach_count = breaching(&rows).len();
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|b| {
            let pct = percent_used(b);
            vec![
                escape(&b.name),
                b.scope.clone(),
                b.period.clone(),
                format!("${}.{}", b.limit_usd_cents / 100, b.limit_usd_cents % 100),
                format!("${}.{}", b.spent_usd_cents / 100, b.spent_usd_cents % 100),
                format!("{pct}%"),
                b.alert_threshold_pct.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3 text-sm">
  <span class="px-2 py-1 rounded bg-red-100">breaching threshold × {breach_count}</span>
</div>{tbl}</section>"#,
        breach_count = breach_count,
        tbl = table(
            &["name", "scope", "period", "limit", "spent", "used", "alert@"],
            &rows_html,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/litellm/budgets",
        &format!("litellm/budgets · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn b(tenant: &str, name: &str, scope: &str, limit: u64, spent: u64, threshold: u8) -> LiteLlmBudget {
        LiteLlmBudget {
            tenant: TenantId::new(tenant).expect("t"),
            name: name.into(),
            scope: scope.into(),
            limit_usd_cents: limit,
            spent_usd_cents: spent,
            period: "monthly".into(),
            reset_at_unix: 0,
            alert_threshold_pct: threshold,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.litellm_budgets.write().unwrap();
        g.push(b("acme", "tenant-cap", "tenant", 100_000, 30_000, 80)); // 30% — fine
        g.push(b("acme", "team-cap", "team", 50_000, 45_000, 80));      // 90% — breach
        g.push(b("acme", "key-cap", "key", 10_000, 9_500, 75));         // 95% — breach
        g.push(b("evil", "secret-cap", "tenant", 1, 1, 100));
        drop(g);
        s
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn list_refuses_without_perm() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn percent_used_caps_at_100() {
        let bud = b("acme", "x", "tenant", 100, 150, 80);
        assert_eq!(percent_used(&bud), 100);
    }

    #[test]
    fn percent_used_zero_limit_safe() {
        let bud = b("acme", "x", "tenant", 0, 5, 80);
        assert_eq!(percent_used(&bud), 0);
    }

    #[test]
    fn breaching_lists_over_threshold() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let breach = breaching(&rows);
        assert_eq!(breach.len(), 2);
        assert!(breach.iter().any(|b| b.name == "team-cap"));
        assert!(breach.iter().any(|b| b.name == "key-cap"));
    }

    #[test]
    fn scope_histogram_counts_per_scope() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let h = scope_histogram(&rows);
        let team = h.iter().find(|(s, _)| s == "team").map(|(_, n)| *n).unwrap();
        assert_eq!(team, 1);
    }

    #[test]
    fn render_includes_columns_and_breach_count() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        for col in ["name", "scope", "period", "limit", "spent"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("breaching"));
    }
}
