// SPDX-License-Identifier: AGPL-3.0-or-later
//! Policies tab — NetworkPolicy editor (Ingress / Egress / Both).
//! Endpoint browser lives in `nodes.rs`; this module is focused on
//! the policy CRUD path.

use super::NetViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState, NetPolicy};

pub fn list_policies(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<NetPolicy>, NetViewError> {
    ctx.authorise(Permission::NetRead)?;
    Ok(scope(&state.net_policies.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn create_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    direction: &str,
    selector: &str,
) -> Result<(), NetViewError> {
    ctx.authorise(Permission::NetWrite)?;
    if selector.trim().is_empty() {
        return Err(NetViewError::EmptySelector);
    }
    let normalised: &'static str = match direction {
        "Ingress" => "Ingress",
        "Egress" => "Egress",
        "Both" => "Both",
        other => return Err(NetViewError::InvalidDirection(other.into())),
    };
    let mut policies = state.net_policies.write().unwrap();
    if policies.iter().any(|p| p.tenant == ctx.tenant && p.name == name) {
        return Err(NetViewError::DuplicatePolicy(name.into()));
    }
    policies.push(NetPolicy {
        tenant: ctx.tenant.clone(),
        name: name.into(),
        direction: normalised,
        selector: selector.into(),
    });
    Ok(())
}

pub fn delete_policy(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<(), NetViewError> {
    ctx.authorise(Permission::NetWrite)?;
    let mut policies = state.net_policies.write().unwrap();
    let before = policies.len();
    policies.retain(|p| !(p.tenant == ctx.tenant && p.name == name));
    if policies.len() == before {
        return Err(NetViewError::PolicyNotFound(name.into()));
    }
    Ok(())
}

/// Impact analysis: count endpoints a policy's selector would
/// affect. Today the selector is a free-text glob; `*` matches all,
/// and literal selectors match endpoint identity strings.
pub fn policy_impact(
    state: &AdminState,
    ctx: &RequestCtx,
    policy: &NetPolicy,
) -> Result<u32, NetViewError> {
    let endpoints = super::nodes::list_endpoints(state, ctx)?;
    let sel = policy.selector.trim();
    if sel == "*" {
        return Ok(endpoints.len() as u32);
    }
    Ok(endpoints
        .iter()
        .filter(|e| e.identity.to_string() == sel || e.namespace == sel)
        .count() as u32)
}

pub(crate) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, NetViewError> {
    let policies = list_policies(state, ctx)?;
    let rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| {
            let impact = policy_impact(state, ctx, p).unwrap_or(0);
            vec![
                p.name.clone(),
                p.direction.into(),
                p.selector.clone(),
                format!("{} endpoints", impact),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="net-policies" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">NetworkPolicies ({n})</h2>
  {tbl}
</section>"#,
        n = policies.len(),
        tbl = table(&["name", "direction", "selector", "impact"], &rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn create_policy_appends_and_validates() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/PolicyEditor.tsx",
            "PolicyEditor",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::NetRead, Permission::NetWrite]);
        create_policy(&s, &c, "deny-all", "Both", "*").unwrap();
        assert!(matches!(
            create_policy(&s, &c, "deny-all", "Both", "*").unwrap_err(),
            NetViewError::DuplicatePolicy(_)
        ));
        assert!(matches!(
            create_policy(&s, &c, "x", "Sideways", "*").unwrap_err(),
            NetViewError::InvalidDirection(_)
        ));
        assert!(matches!(
            create_policy(&s, &c, "y", "Ingress", "  ").unwrap_err(),
            NetViewError::EmptySelector
        ));
    }

    #[test]
    fn delete_policy_removes_and_errors_on_missing() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::NetRead, Permission::NetWrite]);
        delete_policy(&s, &c, "allow-web").unwrap();
        assert_eq!(list_policies(&s, &c).unwrap().len(), 0);
        assert!(matches!(
            delete_policy(&s, &c, "allow-web").unwrap_err(),
            NetViewError::PolicyNotFound(_)
        ));
    }

    #[test]
    fn create_refuses_without_write_perm() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::NetRead]);
        assert!(create_policy(&s, &c, "x", "Both", "*").is_err());
    }

    #[test]
    fn policy_impact_star_matches_every_endpoint() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::NetRead, Permission::NetWrite]);
        // Create a wildcard policy.
        create_policy(&s, &c, "wide", "Both", "*").unwrap();
        let policies = list_policies(&s, &c).unwrap();
        let wide = policies.iter().find(|p| p.name == "wide").unwrap();
        let n = policy_impact(&s, &c, wide).unwrap();
        // Acme has 2 endpoints in the seed.
        assert_eq!(n, 2);
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::NetRead])).unwrap();
        for col in ["name", "direction", "selector", "impact"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
