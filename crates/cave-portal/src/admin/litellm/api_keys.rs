// SPDX-License-Identifier: AGPL-3.0-or-later
//! API keys sub-page.

use super::types::{LiteLlmApiKey, LiteLlmViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmApiKey>, LiteLlmViewError> {
    ctx.authorise(Permission::LiteLlmRead)?;
    let mut rows: Vec<LiteLlmApiKey> =
        scope(&state.litellm_api_keys.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| b.created_at_unix.cmp(&a.created_at_unix));
    Ok(rows)
}

pub fn list_active(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<LiteLlmApiKey>, LiteLlmViewError> {
    Ok(list(state, ctx)?.into_iter().filter(|k| k.status == "active").collect())
}

pub fn get(state: &AdminState, ctx: &RequestCtx, key_id: &str) -> Result<LiteLlmApiKey, LiteLlmViewError> {
    list(state, ctx)?
        .into_iter()
        .find(|k| k.key_id == key_id)
        .ok_or_else(|| LiteLlmViewError::KeyNotFound(key_id.into()))
}

pub fn over_budget<'a>(rows: &'a [LiteLlmApiKey]) -> Vec<&'a LiteLlmApiKey> {
    rows.iter()
        .filter(|k| match k.max_budget_usd_cents {
            Some(max) => k.spent_usd_cents >= max,
            None => false,
        })
        .collect()
}

pub fn near_budget<'a>(rows: &'a [LiteLlmApiKey], threshold_pct: u8) -> Vec<&'a LiteLlmApiKey> {
    rows.iter()
        .filter(|k| match k.max_budget_usd_cents {
            Some(max) if max > 0 => {
                let pct = (k.spent_usd_cents * 100) / max;
                pct >= u64::from(threshold_pct) && k.spent_usd_cents < max
            }
            _ => false,
        })
        .collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LiteLlmViewError> {
    let rows = list(state, ctx)?;
    let over = over_budget(&rows).len();
    let near = near_budget(&rows, 80).len();
    let rows_html: Vec<Vec<String>> = rows
        .iter()
        .map(|k| {
            vec![
                escape(&k.key_id),
                escape(&k.label),
                k.status.clone(),
                k.allowed_models.join(", "),
                k.max_budget_usd_cents
                    .map(|c| format!("${}.{}", c / 100, c % 100))
                    .unwrap_or_else(|| "—".into()),
                format!("${}.{}", k.spent_usd_cents / 100, k.spent_usd_cents % 100),
                k.expires_at_unix.map(|t| t.to_string()).unwrap_or_else(|| "never".into()),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3 text-sm">
  <span class="px-2 py-1 mr-2 rounded bg-red-100">over budget × {over}</span>
  <span class="px-2 py-1 rounded bg-yellow-100">≥80% spent × {near}</span>
</div>{tbl}</section>"#,
        over = over,
        near = near,
        tbl = table(
            &["key_id", "label", "status", "models", "budget", "spent", "expires"],
            &rows_html,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/litellm/api-keys",
        &format!("litellm/api-keys · {}", escape(ctx.tenant.as_str())),
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

    fn key(tenant: &str, id: &str, status: &str, budget: Option<u64>, spent: u64) -> LiteLlmApiKey {
        LiteLlmApiKey {
            tenant: TenantId::new(tenant).expect("t"),
            key_id: id.into(),
            label: format!("key-{id}"),
            allowed_models: vec!["gpt-4o".into()],
            status: status.into(),
            max_budget_usd_cents: budget,
            spent_usd_cents: spent,
            created_at_unix: 0,
            expires_at_unix: None,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.litellm_api_keys.write().unwrap();
        g.push(key("acme", "k1", "active", Some(10_000), 500));   // 5%
        g.push(key("acme", "k2", "active", Some(10_000), 8_500)); // 85%
        g.push(key("acme", "k3", "active", Some(10_000), 10_500));// over
        g.push(key("acme", "k4", "revoked", None, 0));
        g.push(key("evil", "k9", "active", None, 0));
        drop(g);
        s
    }

    #[test]
    fn list_filters_by_tenant() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn list_refuses_without_perm() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_active_excludes_revoked() {
        let s = seeded();
        let rows = list_active(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|k| k.status == "active"));
    }

    #[test]
    fn get_returns_key_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::LiteLlmRead]);
        assert_eq!(get(&s, &c, "k1").unwrap().label, "key-k1");
        assert!(matches!(get(&s, &c, "nope").unwrap_err(), LiteLlmViewError::KeyNotFound(_)));
    }

    #[test]
    fn over_budget_finds_exceeded_keys() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let over = over_budget(&rows);
        assert_eq!(over.len(), 1);
        assert_eq!(over[0].key_id, "k3");
    }

    #[test]
    fn near_budget_uses_threshold() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let near = near_budget(&rows, 80);
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].key_id, "k2");
    }

    #[test]
    fn near_budget_excludes_over_budget_keys() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        let near = near_budget(&rows, 80);
        assert!(!near.iter().any(|k| k.key_id == "k3"));
    }

    #[test]
    fn render_includes_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::LiteLlmRead])).unwrap();
        for col in ["key_id", "status", "budget", "spent"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
        assert!(html.contains("over budget"));
    }
}
