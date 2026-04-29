//! `/admin/mesh` view — AuthorizationPolicy editor + flow log viewer.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, MeshAuthzPolicy, MeshFlow};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MeshViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("policy {0} already exists in this tenant")]
    DuplicatePolicy(String),
    #[error("policy {0} not found")]
    PolicyNotFound(String),
    #[error("invalid action {0}: must be Allow or Deny")]
    InvalidAction(String),
    #[error("principal_glob must be non-empty")]
    EmptyPrincipalGlob,
}

pub fn list_policies(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<MeshAuthzPolicy>, MeshViewError> {
    ctx.authorise(Permission::MeshRead)?;
    Ok(scope(&state.mesh_authz.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_flows(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<MeshFlow>, MeshViewError> {
    ctx.authorise(Permission::MeshRead)?;
    Ok(scope(&state.mesh_flows.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

/// Create-or-update an AuthorizationPolicy. Mirrors the editor's Save action.
pub fn upsert_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    action: &str,
    principal_glob: &str,
) -> Result<(), MeshViewError> {
    ctx.authorise(Permission::MeshWrite)?;
    if action != "Allow" && action != "Deny" {
        return Err(MeshViewError::InvalidAction(action.into()));
    }
    if principal_glob.trim().is_empty() {
        return Err(MeshViewError::EmptyPrincipalGlob);
    }
    let normalised_action: &'static str = if action == "Allow" { "Allow" } else { "Deny" };
    let mut policies = state.mesh_authz.write().unwrap();
    if let Some(existing) = policies
        .iter_mut()
        .find(|p| p.tenant == ctx.tenant && p.name == name)
    {
        existing.action = normalised_action;
        existing.principal_glob = principal_glob.to_string();
    } else {
        policies.push(MeshAuthzPolicy {
            tenant: ctx.tenant.clone(),
            name: name.to_string(),
            action: normalised_action,
            principal_glob: principal_glob.to_string(),
        });
    }
    Ok(())
}

/// Reject duplicate-create. Mirrors the "fail if exists" branch of the editor.
pub fn create_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    action: &str,
    principal_glob: &str,
) -> Result<(), MeshViewError> {
    ctx.authorise(Permission::MeshWrite)?;
    {
        let policies = state.mesh_authz.read().unwrap();
        if policies.iter().any(|p| p.tenant == ctx.tenant && p.name == name) {
            return Err(MeshViewError::DuplicatePolicy(name.into()));
        }
    }
    upsert_policy(state, ctx, name, action, principal_glob)
}

pub fn delete_policy(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<(), MeshViewError> {
    ctx.authorise(Permission::MeshWrite)?;
    let mut policies = state.mesh_authz.write().unwrap();
    let before = policies.len();
    policies.retain(|p| !(p.tenant == ctx.tenant && p.name == name));
    if policies.len() == before {
        return Err(MeshViewError::PolicyNotFound(name.into()));
    }
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, MeshViewError> {
    let policies = list_policies(state, ctx)?;
    let flows = list_flows(state, ctx)?;
    let p_rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| vec![p.name.clone(), p.action.into(), p.principal_glob.clone()])
        .collect();
    let f_rows: Vec<Vec<String>> = flows
        .iter()
        .map(|f| vec![f.source.clone(), f.destination.clone(), f.verdict.into(), f.bytes.to_string()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">AuthZ ({n_p})</h2>{p_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Flows ({n_f})</h2>{f_tbl}</section>"#,
        n_p = policies.len(),
        n_f = flows.len(),
        p_tbl = table(&["name", "action", "principal_glob"], &p_rows),
        f_tbl = table(&["src", "dst", "verdict", "bytes"], &f_rows),
    );
    Ok(page_shell(
        &format!("mesh · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/catalog-graph/src/components/CatalogGraphPage/CatalogGraphPage.tsx",
    "CatalogGraphPage",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_policies_filters_to_owner() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/CatalogGraphPage/CatalogGraphPage.tsx",
            "PolicyTable",
            "acme"
        );
        let state = AdminState::seeded();
        let p = list_policies(&state, &ctx(&[Permission::MeshRead])).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "allow-web");
    }

    #[test]
    fn create_policy_appends_and_rejects_duplicate() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "PolicyEditor",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        create_policy(&state, &c, "deny-bots", "Deny", "spiffe://*/sa/bot").unwrap();
        let err = create_policy(&state, &c, "deny-bots", "Deny", "spiffe://*/sa/bot").unwrap_err();
        assert!(matches!(err, MeshViewError::DuplicatePolicy(_)));
    }

    #[test]
    fn invalid_action_is_rejected() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "validateAction",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        let err = create_policy(&state, &c, "x", "Maybe", "*").unwrap_err();
        assert!(matches!(err, MeshViewError::InvalidAction(_)));
    }

    #[test]
    fn delete_policy_removes_and_errors_on_missing() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/PolicyEditor.tsx",
            "deletePolicy",
            "acme"
        );
        let state = AdminState::seeded();
        let c = ctx(&[Permission::MeshRead, Permission::MeshWrite]);
        delete_policy(&state, &c, "allow-web").unwrap();
        assert_eq!(list_policies(&state, &c).unwrap().len(), 0);
        let err = delete_policy(&state, &c, "allow-web").unwrap_err();
        assert!(matches!(err, MeshViewError::PolicyNotFound(_)));
    }

    #[test]
    fn list_flows_only_returns_owner_flows() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/catalog-graph/src/components/FlowViewer.tsx",
            "FlowViewer",
            "acme"
        );
        let state = AdminState::seeded();
        let f = list_flows(&state, &ctx(&[Permission::MeshRead])).unwrap();
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| x.tenant.as_str() == "acme"));
    }
}
