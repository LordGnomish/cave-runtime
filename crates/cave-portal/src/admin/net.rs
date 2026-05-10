//! `/admin/net` view — Cilium endpoint browser + NetworkPolicy editor.
//!
//! Mirrors the `cilium-hubble` flow widget plus the
//! `kubernetes-network-policies` plugin pane Backstage exposes.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, NetEndpoint, NetPolicy};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NetViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("policy {0} already exists in this tenant")]
    DuplicatePolicy(String),
    #[error("policy {0} not found")]
    PolicyNotFound(String),
    #[error("invalid direction {0}: must be Ingress, Egress or Both")]
    InvalidDirection(String),
    #[error("selector must be non-empty")]
    EmptySelector,
}

pub fn list_endpoints(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<NetEndpoint>, NetViewError> {
    ctx.authorise(Permission::NetRead)?;
    Ok(scope(&state.net_endpoints.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

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

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, NetViewError> {
    let endpoints = list_endpoints(state, ctx)?;
    let policies = list_policies(state, ctx)?;
    let e_rows: Vec<Vec<String>> = endpoints
        .iter()
        .map(|e| {
            vec![
                e.identity.to_string(),
                e.namespace.clone(),
                e.ip.clone(),
                if e.ready { "Ready" } else { "NotReady" }.into(),
            ]
        })
        .collect();
    let p_rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| vec![p.name.clone(), p.direction.into(), p.selector.clone()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Endpoints ({n_e})</h2>{e_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">NetworkPolicies ({n_p})</h2>{p_tbl}</section>"#,
        n_e = endpoints.len(),
        n_p = policies.len(),
        e_tbl = table(&["identity", "namespace", "ip", "ready"], &e_rows),
        p_tbl = table(&["name", "direction", "selector"], &p_rows),
    );
    Ok(page_shell(
        &format!("net · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Network/NetworkPoliciesTab.tsx",
    "NetworkPoliciesTab",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_endpoints_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/EndpointsTab.tsx",
            "EndpointsTab",
            "acme"
        );
        let s = AdminState::seeded();
        let e = list_endpoints(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert_eq!(e.len(), 2);
        assert!(e.iter().all(|x| x.tenant.as_str() == "acme"));
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
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/PolicyEditor.tsx",
            "deletePolicy",
            "acme"
        );
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
    fn create_refuses_cross_tenant_via_perm_layer() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::NetRead]);
        assert!(create_policy(&s, &c, "x", "Both", "*").is_err());
    }

    #[test]
    fn render_does_not_leak_evil_endpoint_or_policy() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Network/NetworkPage.tsx",
            "NetworkPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::NetRead])).unwrap();
        assert!(html.contains("Endpoints (2)"));
        assert!(html.contains("NetworkPolicies (1)"));
        assert!(!html.contains("evil-allow-all"));
        assert!(!html.contains("10.0.99.99"));
    }
}
