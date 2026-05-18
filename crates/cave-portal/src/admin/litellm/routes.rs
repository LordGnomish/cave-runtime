// SPDX-License-Identifier: AGPL-3.0-or-later
//! Routes sub-page.

use super::types::{LiteLlmRoute, LiteLlmViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmRoute>, LiteLlmViewError> {
    ctx.authorise(Permission::LiteLlmRead)?;
    let mut rows: Vec<LiteLlmRoute> =
        scope(&state.litellm_routes.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}

pub fn list_enabled(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmRoute>, LiteLlmViewError> {
    Ok(list(state, ctx)?.into_iter().filter(|r| r.enabled).collect())
}

pub fn get(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<LiteLlmRoute, LiteLlmViewError> {
    list(state, ctx)?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| LiteLlmViewError::RouteNotFound(name.into()))
}

pub fn strategy_histogram(rows: &[LiteLlmRoute]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.strategy.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LiteLlmViewError> {
    let rows = list(state, ctx)?;
    let hist = strategy_histogram(&rows);
    let chips: String = hist
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
                s = escape(s),
                n = n
            )
        })
        .collect();
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                escape(&r.pattern),
                r.target_models.join(", "),
                r.strategy.clone(),
                if r.enabled { "enabled" } else { "disabled" }.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3">{chips}</div>{tbl}</section>"#,
        chips = chips,
        tbl = table(&["name", "pattern", "targets", "strategy", "state"], &rows_html),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/litellm/routes",
        &format!("litellm/routes · {}", escape(ctx.tenant.as_str())),
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

    fn r(tenant: &str, name: &str, strategy: &str, enabled: bool) -> LiteLlmRoute {
        LiteLlmRoute {
            tenant: TenantId::new(tenant).expect("t"),
            name: name.into(),
            pattern: format!("{name}*"),
            target_models: vec!["gpt-4o".into(), "claude-3-5-sonnet".into()],
            strategy: strategy.into(),
            weights: vec![("gpt-4o".into(), 70), ("claude-3-5-sonnet".into(), 30)],
            enabled,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.litellm_routes.write().unwrap();
        g.push(r("acme", "premium", "weighted", true));
        g.push(r("acme", "budget", "lowest_cost", true));
        g.push(r("acme", "legacy", "round_robin", false));
        g.push(r("evil", "secret", "round_robin", true));
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
    fn list_enabled_filters_disabled() {
        let s = seeded();
        let rows = list_enabled(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.enabled));
    }

    #[test]
    fn get_returns_route_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::LiteLlmRead]);
        assert_eq!(get(&s, &c, "premium").unwrap().strategy, "weighted");
        assert!(matches!(get(&s, &c, "nope").unwrap_err(), LiteLlmViewError::RouteNotFound(_)));
    }

    #[test]
    fn strategy_histogram_counts() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let h = strategy_histogram(&rows);
        let rr = h.iter().find(|(s, _)| s == "round_robin").map(|(_, n)| *n).unwrap();
        assert_eq!(rr, 1);
    }

    #[test]
    fn render_includes_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        for col in ["name", "pattern", "targets", "strategy"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
    }
}
