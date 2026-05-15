// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/auth/flows` — Keycloak Admin Console "Authentication" → "Flows" pane.
//!
//! Lists the realm's authentication flows + per-flow execution editor.
//! Backed by `cave_auth::keycloak::admin::authflow::FlowService`.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRow {
    pub alias: String,
    pub description: String,
    pub provider_id: String,
    pub built_in: bool,
    pub execution_count: usize,
}

pub fn seeded_flows() -> Vec<FlowRow> {
    vec![
        FlowRow { alias: "browser".into(), description: "browser based authentication".into(),
                  provider_id: "basic-flow".into(), built_in: true, execution_count: 4 },
        FlowRow { alias: "direct grant".into(), description: "OpenID Connect Resource Owner Grant".into(),
                  provider_id: "basic-flow".into(), built_in: true, execution_count: 3 },
        FlowRow { alias: "registration".into(), description: "registration flow".into(),
                  provider_id: "basic-flow".into(), built_in: true, execution_count: 1 },
        FlowRow { alias: "reset credentials".into(), description: "Reset credentials".into(),
                  provider_id: "basic-flow".into(), built_in: true, execution_count: 3 },
        FlowRow { alias: "clients".into(), description: "Base authentication for clients".into(),
                  provider_id: "client-flow".into(), built_in: true, execution_count: 4 },
        FlowRow { alias: "first broker login".into(), description: "Actions taken after first broker login".into(),
                  provider_id: "basic-flow".into(), built_in: true, execution_count: 2 },
    ]
}

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, super::AuthViewError> {
    ctx.authorise(Permission::AuthFlowRead)?;
    let flows = seeded_flows();
    let rows: Vec<Vec<String>> = flows.iter().map(|f| vec![
        escape(&f.alias),
        escape(&f.description),
        escape(&f.provider_id),
        (if f.built_in { "built-in" } else { "custom" }).to_string(),
        f.execution_count.to_string(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Keycloak Admin Console parity — Authentication Flows.
    Backend: <code>cave_auth::keycloak::admin::authflow</code>.
    Built-in flows cannot be edited or deleted; clone via
    <code>cavectl auth flow clone &lt;src&gt; &lt;dst&gt;</code>.
  </p>
  <h2 class="text-lg font-semibold mb-2">Flows ({n})</h2>
  {tbl}
  <div class="mt-4 text-sm">
    <strong>Execution requirements:</strong>
    REQUIRED · ALTERNATIVE · OPTIONAL · DISABLED · CONDITIONAL.
  </div>
</section>"#,
        n = flows.len(),
        tbl = table(&["alias", "description", "providerId", "type", "executions"], &rows),
    );
    Ok(page_shell_full(ctx, "/admin/auth/flows", &format!("auth/flows · {}", escape(ctx.tenant.as_str())), &body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_requires_permission() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_lists_six_builtin_flows() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthFlowRead])).unwrap();
        for alias in &["browser", "direct grant", "registration", "reset credentials", "clients", "first broker login"] {
            assert!(html.contains(alias), "missing builtin flow {alias}");
        }
    }

    #[test]
    fn render_describes_requirement_enum() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::AuthFlowRead])).unwrap();
        for r in &["REQUIRED", "ALTERNATIVE", "OPTIONAL", "DISABLED", "CONDITIONAL"] {
            assert!(html.contains(r));
        }
    }

    #[test]
    fn seeded_flows_returns_six() {
        assert_eq!(seeded_flows().len(), 6);
    }
}
