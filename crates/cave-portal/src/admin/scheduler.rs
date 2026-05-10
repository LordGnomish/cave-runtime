//! `/admin/scheduler` view — node-pool browser + scheduling policy editor.
//!
//! Mirrors the kube-scheduler dashboard tab Backstage's `kubernetes` plugin
//! exposes via `NodeStatus` + the in-tree scheduling-profile picker.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, SchedulerNode, SchedulerPolicy};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SchedulerViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("policy {0} already exists in this tenant")]
    DuplicatePolicy(String),
    #[error("policy {0} not found")]
    PolicyNotFound(String),
    #[error("predicate must be non-empty")]
    EmptyPredicate,
    #[error("weight must be between 1 and 100")]
    InvalidWeight,
}

pub fn list_nodes(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SchedulerNode>, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    Ok(scope(&state.scheduler_nodes.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_policies(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SchedulerPolicy>, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    Ok(scope(&state.scheduler_policies.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn create_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    predicate: &str,
    weight: u32,
) -> Result<(), SchedulerViewError> {
    ctx.authorise(Permission::SchedulerWrite)?;
    if predicate.trim().is_empty() {
        return Err(SchedulerViewError::EmptyPredicate);
    }
    if !(1..=100).contains(&weight) {
        return Err(SchedulerViewError::InvalidWeight);
    }
    let mut policies = state.scheduler_policies.write().unwrap();
    if policies.iter().any(|p| p.tenant == ctx.tenant && p.name == name) {
        return Err(SchedulerViewError::DuplicatePolicy(name.into()));
    }
    policies.push(SchedulerPolicy {
        tenant: ctx.tenant.clone(),
        name: name.into(),
        predicate: predicate.into(),
        weight,
    });
    Ok(())
}

pub fn delete_policy(state: &AdminState, ctx: &RequestCtx, name: &str) -> Result<(), SchedulerViewError> {
    ctx.authorise(Permission::SchedulerWrite)?;
    let mut policies = state.scheduler_policies.write().unwrap();
    let before = policies.len();
    policies.retain(|p| !(p.tenant == ctx.tenant && p.name == name));
    if policies.len() == before {
        return Err(SchedulerViewError::PolicyNotFound(name.into()));
    }
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SchedulerViewError> {
    let nodes = list_nodes(state, ctx)?;
    let policies = list_policies(state, ctx)?;
    let n_rows: Vec<Vec<String>> = nodes
        .iter()
        .map(|n| {
            vec![
                n.name.clone(),
                if n.ready { "Ready" } else { "NotReady" }.into(),
                format!("{} m", n.allocatable_cpu_milli),
                format!("{} MiB", n.allocatable_mem_mib),
                n.taints.join(","),
            ]
        })
        .collect();
    let p_rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| vec![p.name.clone(), p.predicate.clone(), p.weight.to_string()])
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Nodes ({n_n})</h2>{n_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Policies ({n_p})</h2>{p_tbl}</section>"#,
        n_n = nodes.len(),
        n_p = policies.len(),
        n_tbl = table(&["name", "ready", "cpu", "mem", "taints"], &n_rows),
        p_tbl = table(&["name", "predicate", "weight"], &p_rows),
    );
    Ok(page_shell(
        &format!("scheduler · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/kubernetes/src/components/Pods/Pods.tsx",
    "NodeList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_nodes_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Pods.tsx",
            "NodeList",
            "acme"
        );
        let s = AdminState::seeded();
        let n = list_nodes(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        assert_eq!(n.len(), 2);
        assert!(n.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_nodes_refuses_without_read() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Pods.tsx",
            "RbacGate",
            "acme"
        );
        let s = AdminState::seeded();
        assert!(list_nodes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn create_policy_appends_and_rejects_duplicate() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Profile/PolicyEditor.tsx",
            "PolicyEditor",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::SchedulerRead, Permission::SchedulerWrite]);
        create_policy(&s, &c, "spread", "topology=zone", 3).unwrap();
        let err = create_policy(&s, &c, "spread", "topology=zone", 3).unwrap_err();
        assert!(matches!(err, SchedulerViewError::DuplicatePolicy(_)));
    }

    #[test]
    fn invalid_weight_or_empty_predicate_rejected() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Profile/PolicyEditor.tsx",
            "validatePolicy",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::SchedulerRead, Permission::SchedulerWrite]);
        assert!(matches!(
            create_policy(&s, &c, "x", "p", 0).unwrap_err(),
            SchedulerViewError::InvalidWeight
        ));
        assert!(matches!(
            create_policy(&s, &c, "y", "  ", 5).unwrap_err(),
            SchedulerViewError::EmptyPredicate
        ));
    }

    #[test]
    fn delete_policy_removes_and_errors_on_missing() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Profile/PolicyEditor.tsx",
            "deletePolicy",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::SchedulerRead, Permission::SchedulerWrite]);
        delete_policy(&s, &c, "least-utilised").unwrap();
        assert_eq!(list_policies(&s, &c).unwrap().len(), 0);
        assert!(matches!(
            delete_policy(&s, &c, "least-utilised").unwrap_err(),
            SchedulerViewError::PolicyNotFound(_)
        ));
    }

    #[test]
    fn render_omits_foreign_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/PodsPage.tsx",
            "PodsPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        assert!(html.contains("Nodes (2)"));
        assert!(html.contains("node-a"));
        assert!(!html.contains("evil-node"));
    }
}
