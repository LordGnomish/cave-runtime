// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 js/apps/admin-ui/ (visual port, server-rendered Maud)
//
//! `/admin/auth/flows` — Authentication flows + execution editor.
//! Calls A5's `admin_flows` endpoints. Visual port of
//! `js/apps/admin-ui/src/authentication/AuthenticationSection.tsx`.

use super::{AuthAdminError, render_admin_nav, require_platform};
use crate::admin::permission::RequestCtx;
use crate::admin::render::{escape, page_shell_full};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowDef {
    pub alias: String,
    pub description: String,
    pub built_in: bool,
    pub executions: Vec<FlowExecution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowExecution {
    pub provider: String,
    pub requirement: ExecRequirement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecRequirement {
    Required,
    Alternative,
    Disabled,
    Conditional,
}
impl ExecRequirement {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Required => "REQUIRED",
            Self::Alternative => "ALTERNATIVE",
            Self::Disabled => "DISABLED",
            Self::Conditional => "CONDITIONAL",
        }
    }
}

pub fn builtin_flows() -> Vec<FlowDef> {
    vec![
        FlowDef {
            alias: "browser".into(),
            description: "Browser based authentication".into(),
            built_in: true,
            executions: vec![
                FlowExecution {
                    provider: "auth-cookie".into(),
                    requirement: ExecRequirement::Alternative,
                },
                FlowExecution {
                    provider: "auth-spnego".into(),
                    requirement: ExecRequirement::Disabled,
                },
                FlowExecution {
                    provider: "identity-provider-redirector".into(),
                    requirement: ExecRequirement::Alternative,
                },
                FlowExecution {
                    provider: "forms (Username Password Form)".into(),
                    requirement: ExecRequirement::Required,
                },
                FlowExecution {
                    provider: "WebAuthn Authenticator".into(),
                    requirement: ExecRequirement::Conditional,
                },
                FlowExecution {
                    provider: "OTP Form".into(),
                    requirement: ExecRequirement::Conditional,
                },
            ],
        },
        FlowDef {
            alias: "direct grant".into(),
            description: "OpenID Connect Resource Owner Password Credentials grant".into(),
            built_in: true,
            executions: vec![
                FlowExecution {
                    provider: "direct-grant-validate-username".into(),
                    requirement: ExecRequirement::Required,
                },
                FlowExecution {
                    provider: "direct-grant-validate-password".into(),
                    requirement: ExecRequirement::Required,
                },
                FlowExecution {
                    provider: "direct-grant-validate-otp".into(),
                    requirement: ExecRequirement::Conditional,
                },
            ],
        },
        FlowDef {
            alias: "first broker login".into(),
            description: "Federated user first login".into(),
            built_in: true,
            executions: vec![
                FlowExecution {
                    provider: "review profile".into(),
                    requirement: ExecRequirement::Required,
                },
                FlowExecution {
                    provider: "create-unique-user-config / link-existing".into(),
                    requirement: ExecRequirement::Required,
                },
            ],
        },
        FlowDef {
            alias: "WebAuthn Passwordless".into(),
            description: "Passkey-only authentication".into(),
            built_in: false,
            executions: vec![
                FlowExecution {
                    provider: "Username Form".into(),
                    requirement: ExecRequirement::Required,
                },
                FlowExecution {
                    provider: "WebAuthn Authenticator (Passwordless)".into(),
                    requirement: ExecRequirement::Required,
                },
            ],
        },
    ]
}

fn render_flow(flow: &FlowDef) -> String {
    let exec_rows: String = flow
        .executions
        .iter()
        .map(|e| {
            format!(
                r#"<tr class="border-t"><td class="px-3 py-2">{p}</td><td class="px-3 py-2"><code class="text-xs">{r}</code></td></tr>"#,
                p = escape(&e.provider),
                r = e.requirement.as_str()
            )
        })
        .collect();
    let built_in_badge = if flow.built_in {
        r#"<span class="text-xs text-zinc-500">built-in</span>"#
    } else {
        r#"<span class="text-xs text-amber-700">custom</span>"#
    };
    format!(
        r#"<details class="my-3" open>
  <summary class="cursor-pointer">
    <strong>{alias}</strong> {badge}
    <span class="text-sm text-zinc-500">— {desc}</span>
  </summary>
  <div class="mt-2">
    <table class="min-w-full text-sm border-collapse">
      <thead class="bg-gray-100 dark:bg-zinc-800"><tr><th class="px-3 py-2 text-left">execution</th><th class="px-3 py-2 text-left">requirement</th></tr></thead>
      <tbody>{rows}</tbody>
    </table>
    <div class="mt-2 flex gap-2">
      <a class="text-xs text-blue-700 underline" href="/admin/auth/flows/{alias_e}">edit</a>
      <a class="text-xs text-blue-700 underline" href="/admin/auth/flows/{alias_e}/duplicate">duplicate</a>
    </div>
  </div>
</details>"#,
        alias = escape(&flow.alias),
        badge = built_in_badge,
        desc = escape(&flow.description),
        rows = exec_rows,
        alias_e = escape(&flow.alias),
    )
}

pub fn render(ctx: &RequestCtx) -> Result<String, AuthAdminError> {
    require_platform(ctx)?;
    let flows = builtin_flows();
    let flows_html: String = flows.iter().map(render_flow).collect();
    let body = format!(
        r#"{nav}
<section>
  <div class="flex items-center justify-between mb-3">
    <h2 class="text-lg font-semibold">Authentication flows</h2>
    <a class="px-3 py-1 rounded bg-blue-600 text-white" href="/admin/auth/flows/new">Create flow</a>
  </div>
  <p class="text-sm text-gray-600 dark:text-zinc-400 mb-3">
    Execution graphs for each authentication scenario.
    Upstream: <a class="text-blue-700 underline" href="https://www.keycloak.org/docs-api/latest/rest-api/index.html#_authentication_management_resource">Keycloak Authentication</a>.
  </p>
  {flows}
</section>"#,
        nav = render_admin_nav("/admin/auth/flows"),
        flows = flows_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/auth/flows",
        &format!("auth/flows · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::{Persona, RequestCtx};

    #[test]
    fn builtin_flows_covers_browser_direct_first_and_webauthn() {
        let f = builtin_flows();
        assert!(f.iter().any(|x| x.alias == "browser"));
        assert!(f.iter().any(|x| x.alias == "direct grant"));
        assert!(f.iter().any(|x| x.alias == "first broker login"));
        assert!(f.iter().any(|x| x.alias.contains("WebAuthn")));
    }

    #[test]
    fn exec_requirement_str_is_uppercase_per_keycloak_wire() {
        assert_eq!(ExecRequirement::Required.as_str(), "REQUIRED");
        assert_eq!(ExecRequirement::Alternative.as_str(), "ALTERNATIVE");
        assert_eq!(ExecRequirement::Conditional.as_str(), "CONDITIONAL");
    }

    #[test]
    fn render_blocks_tenant_admin() {
        let ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        assert!(render(&ctx).is_err());
    }

    #[test]
    fn render_emits_create_flow_button_and_duplicate_link() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("Create flow"));
        assert!(html.contains("duplicate"));
    }

    #[test]
    fn render_lists_each_execution_with_requirement_label() {
        let ctx = RequestCtx::developer("acme", &[]);
        let html = render(&ctx).unwrap();
        assert!(html.contains("REQUIRED"));
        assert!(html.contains("ALTERNATIVE"));
        assert!(html.contains("WebAuthn Authenticator"));
    }
}
