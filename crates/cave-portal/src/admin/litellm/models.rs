//! Models sub-page.

use super::types::{LiteLlmModel, LiteLlmViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmModel>, LiteLlmViewError> {
    ctx.authorise(Permission::LiteLlmRead)?;
    let mut rows: Vec<LiteLlmModel> =
        scope(&state.litellm_models.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn list_active(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmModel>, LiteLlmViewError> {
    Ok(list(state, ctx)?.into_iter().filter(|m| m.status == "active").collect())
}

pub fn get(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<LiteLlmModel, LiteLlmViewError> {
    list(state, ctx)?
        .into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| LiteLlmViewError::ModelNotFound(name.into()))
}

pub fn provider_histogram(rows: &[LiteLlmModel]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.provider.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn with_fallback<'a>(rows: &'a [LiteLlmModel]) -> Vec<&'a LiteLlmModel> {
    rows.iter().filter(|m| !m.fallback_chain.is_empty()).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LiteLlmViewError> {
    let rows = list(state, ctx)?;
    let hist = provider_histogram(&rows);
    let chips: String = hist
        .iter()
        .map(|(p, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{p} <strong>×{n}</strong></span>"#,
                p = escape(p),
                n = n
            )
        })
        .collect();
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            vec![
                escape(&m.name),
                escape(&m.provider),
                escape(&m.model_id),
                m.status.clone(),
                m.rpm_limit.to_string(),
                m.tpm_limit.to_string(),
                m.fallback_chain.join(" → "),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3">{chips}</div>{tbl}</section>"#,
        chips = chips,
        tbl = table(
            &["name", "provider", "model_id", "status", "rpm", "tpm", "fallback"],
            &rows_html,
        ),
    );
    Ok(page_shell(
        &format!("litellm/models · {}", escape(ctx.tenant.as_str())),
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

    fn m(tenant: &str, name: &str, provider: &str, status: &str, fallback: Vec<&str>) -> LiteLlmModel {
        LiteLlmModel {
            tenant: TenantId::new(tenant).expect("t"),
            name: name.into(),
            provider: provider.into(),
            model_id: format!("{provider}/{name}"),
            status: status.into(),
            rpm_limit: 1000,
            tpm_limit: 1_000_000,
            fallback_chain: fallback.into_iter().map(String::from).collect(),
            created_at_unix: 0,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.litellm_models.write().unwrap();
        g.push(m("acme", "gpt-4o", "openai", "active", vec!["claude-3-5-sonnet"]));
        g.push(m("acme", "claude-3-5-sonnet", "anthropic", "active", vec![]));
        g.push(m("acme", "old-model", "openai", "disabled", vec![]));
        g.push(m("evil", "secret", "openai", "active", vec![]));
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
    fn list_active_excludes_disabled() {
        let s = seeded();
        let rows = list_active(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|m| m.status == "active"));
    }

    #[test]
    fn get_returns_model_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::LiteLlmRead]);
        assert_eq!(get(&s, &c, "gpt-4o").unwrap().provider, "openai");
        assert!(matches!(get(&s, &c, "nope").unwrap_err(), LiteLlmViewError::ModelNotFound(_)));
    }

    #[test]
    fn provider_histogram_groups() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let h = provider_histogram(&rows);
        let openai = h.iter().find(|(p, _)| p == "openai").map(|(_, n)| *n).unwrap();
        assert_eq!(openai, 2);
    }

    #[test]
    fn with_fallback_filters_to_chained() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let f = with_fallback(&rows);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].name, "gpt-4o");
    }

    #[test]
    fn render_includes_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        for col in ["name", "provider", "model_id", "status", "rpm", "tpm"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("openai"));
    }

    #[test]
    fn render_excludes_foreign_tenant() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert!(!html.contains(">secret<"));
    }
}
