//! `/admin/chaos` view — chaos experiment browser + on-demand trigger.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, ChaosExperiment};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChaosViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("experiment {0} not found in this tenant")]
    ExperimentNotFound(String),
}

pub fn list_experiments(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ChaosExperiment>, ChaosViewError> {
    ctx.authorise(Permission::ChaosRead)?;
    Ok(scope(&state.chaos_experiments.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn trigger(state: &AdminState, ctx: &RequestCtx, name: &str, now_unix: i64) -> Result<(), ChaosViewError> {
    ctx.authorise(Permission::ChaosTrigger)?;
    let mut exps = state.chaos_experiments.write().unwrap();
    let target = exps.iter_mut().find(|e| e.tenant == ctx.tenant && e.name == name)
        .ok_or_else(|| ChaosViewError::ExperimentNotFound(name.into()))?;
    target.last_run_unix = Some(now_unix);
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, ChaosViewError> {
    let exps = list_experiments(state, ctx)?;
    let rows: Vec<Vec<String>> = exps.iter().map(|e| vec![
        e.name.clone(), e.kind.clone(), e.target_selector.clone(),
        e.schedule.into(),
        e.last_run_unix.map(|x| x.to_string()).unwrap_or_else(|| "never".into()),
    ]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Chaos experiments ({n})</h2>{tbl}</section>"#,
        n = exps.len(),
        tbl = table(&["name", "kind", "target", "schedule", "last_run"], &rows),
    );
    Ok(page_shell(&format!("chaos · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/chaos/src/components/ExperimentsList.tsx", "ExperimentsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/chaos/src/components/ExperimentsList.tsx", "ExperimentsList", "acme");
        let s = AdminState::seeded();
        let e = list_experiments(&s, &ctx(&[Permission::ChaosRead])).unwrap();
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_experiments(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn trigger_updates_last_run() {
        let (_c, _t) = portal_test_ctx!("plugins/chaos/src/components/TriggerButton.tsx", "TriggerButton", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::ChaosRead, Permission::ChaosTrigger]);
        trigger(&s, &c, "delay-api-egress", 1_003_000).unwrap();
        let e = list_experiments(&s, &c).unwrap();
        assert_eq!(e.iter().find(|x| x.name == "delay-api-egress").unwrap().last_run_unix, Some(1_003_000));
    }

    #[test]
    fn trigger_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-backend/src/PermissionsService.ts", "tenantScopeGuard", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::ChaosRead, Permission::ChaosTrigger]);
        assert!(matches!(trigger(&s, &c, "evil-chaos", 0).unwrap_err(), ChaosViewError::ExperimentNotFound(_)));
    }

    #[test]
    fn render_excludes_evil_experiment() {
        let (_c, _t) = portal_test_ctx!("plugins/chaos/src/components/ExperimentsPage.tsx", "ExperimentsPage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::ChaosRead])).unwrap();
        assert!(html.contains("Chaos experiments (2)"));
        assert!(!html.contains("evil-chaos"));
    }
}
