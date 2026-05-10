//! `/admin/policy` view — cave-policy rule browser + enable/disable toggle.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, PolicyRule};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("rule {0} not found in this tenant")]
    RuleNotFound(String),
}

pub fn list_rules(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<PolicyRule>, PolicyViewError> {
    ctx.authorise(Permission::PolicyRead)?;
    Ok(scope(&state.policy_rules.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn set_enabled(state: &AdminState, ctx: &RequestCtx, name: &str, enabled: bool) -> Result<(), PolicyViewError> {
    ctx.authorise(Permission::PolicyWrite)?;
    let mut rules = state.policy_rules.write().unwrap();
    let target = rules.iter_mut().find(|r| r.tenant == ctx.tenant && r.name == name)
        .ok_or_else(|| PolicyViewError::RuleNotFound(name.into()))?;
    target.enabled = enabled;
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PolicyViewError> {
    let rules = list_rules(state, ctx)?;
    let rows: Vec<Vec<String>> = rules.iter().map(|r| vec![
        r.name.clone(), r.action.into(), r.subject.clone(), r.resource.clone(),
        if r.enabled { "on" } else { "off" }.into(),
    ]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Policy rules ({n})</h2>{tbl}</section>"#,
        n = rules.len(),
        tbl = table(&["name", "action", "subject", "resource", "enabled"], &rows),
    );
    Ok(page_shell(&format!("policy · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/policy/src/components/RulesList.tsx", "RulesList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/policy/src/components/RulesList.tsx", "RulesList", "acme");
        let s = AdminState::seeded();
        let r = list_rules(&s, &ctx(&[Permission::PolicyRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        let s = AdminState::seeded();
        assert!(list_rules(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn set_enabled_toggles_and_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!("plugins/policy/src/components/RuleToggle.tsx", "toggle", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::PolicyRead, Permission::PolicyWrite]);
        set_enabled(&s, &c, "deny-internet-prod", false).unwrap();
        let r = list_rules(&s, &c).unwrap();
        assert!(!r.iter().find(|x| x.name == "deny-internet-prod").unwrap().enabled);
        assert!(matches!(set_enabled(&s, &c, "evil-allow-all", false).unwrap_err(), PolicyViewError::RuleNotFound(_)));
    }

    #[test]
    fn set_enabled_requires_write() {
        let (_c, _t) = portal_test_ctx!("plugins/policy/src/components/RuleToggle.tsx", "writePerm", "acme");
        let s = AdminState::seeded();
        assert!(set_enabled(&s, &ctx(&[Permission::PolicyRead]), "deny-internet-prod", false).is_err());
    }

    #[test]
    fn render_excludes_evil_rule() {
        let (_c, _t) = portal_test_ctx!("plugins/policy/src/components/RulesPage.tsx", "RulesPage", "acme");
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::PolicyRead])).unwrap();
        assert!(html.contains("Policy rules (2)"));
        assert!(html.contains("deny-internet-prod"));
        assert!(!html.contains("evil-allow-all"));
    }
}
