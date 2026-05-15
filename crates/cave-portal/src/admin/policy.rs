//! `/admin/policy` — OPA Rego Playground parity. Rule browser with
//! action grouping + enable-toggle mutator (preserved).
//!
//! Upstream UI: <https://play.openpolicyagent.org/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
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

pub fn group_by_action(rows: &[PolicyRule]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows { *acc.entry(r.action.to_string()).or_insert(0) += 1; }
    acc.into_iter().collect()
}

pub fn enabled_count(rows: &[PolicyRule]) -> usize {
    rows.iter().filter(|r| r.enabled).count()
}

pub fn by_action<'a>(rows: &'a [PolicyRule], action: &str) -> Vec<&'a PolicyRule> {
    rows.iter().filter(|r| r.action == action).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PolicyViewError> {
    let rules = list_rules(state, ctx)?;
    let enabled = enabled_count(&rules);
    let groups = group_by_action(&rules);
    let chips: String = groups.iter().map(|(a, n)| format!(
        r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{a} <strong>×{n}</strong></span>"#,
        a = escape(a), n = n)).collect();
    let rows: Vec<Vec<String>> = rules.iter().map(|r| vec![
        r.name.clone(), r.action.into(), r.subject.clone(), r.resource.clone(),
        if r.enabled { "on" } else { "off" }.into(),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">OPA Rego (cave-policy). Upstream: <a class="text-blue-700 underline" href="https://play.openpolicyagent.org/">play.openpolicyagent.org</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> rules</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{enabled}</strong> enabled</span>
  </div>
  <div class="mb-4">{chips}</div>
  <h2 class="text-lg font-semibold mb-2">Policy rules ({n})</h2>{tbl}
</section>"#,
        n = rules.len(),
        enabled = enabled,
        chips = chips,
        tbl = table(&["name", "action", "subject", "resource", "enabled"], &rows),
    );
    Ok(page_shell_full(ctx, "/admin/policy", &format!("policy · {}", escape(ctx.tenant.as_str())), &body))
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
    fn group_by_action_counts() {
        let r = list_rules(&AdminState::seeded(), &ctx(&[Permission::PolicyRead])).unwrap();
        let g = group_by_action(&r);
        assert_eq!(g.iter().map(|(_, n)| n).sum::<usize>(), r.len());
    }

    #[test]
    fn enabled_count_filters_disabled() {
        let r = list_rules(&AdminState::seeded(), &ctx(&[Permission::PolicyRead])).unwrap();
        let on = enabled_count(&r);
        let expected = r.iter().filter(|x| x.enabled).count();
        assert_eq!(on, expected);
    }

    #[test]
    fn render_includes_action_chips_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::PolicyRead])).unwrap();
        assert!(html.contains("openpolicyagent.org"));
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
