//! `/admin/keda` view — ScaledObject + ScaledJob browser with pause/resume.
//!
//! Mirrors the `kubernetes-keda` Backstage plugin pane. `pause_scaled_object`
//! freezes the current_replicas at min_replicas (a hard floor) until the
//! caller resumes it. KEDA's reconciler will still update other fields on
//! its own clock; this view just exposes the levers.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, KedaScaledJob, KedaScaledObject};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KedaViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("scaled object {0} not found")]
    ScaledObjectNotFound(String),
}

pub fn list_scaled_objects(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<KedaScaledObject>, KedaViewError> {
    ctx.authorise(Permission::KedaRead)?;
    Ok(scope(&state.keda_scaled_objects.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_scaled_jobs(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<KedaScaledJob>, KedaViewError> {
    ctx.authorise(Permission::KedaRead)?;
    Ok(scope(&state.keda_scaled_jobs.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

/// Pause a ScaledObject: pin current_replicas to min_replicas.
pub fn pause_scaled_object(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), KedaViewError> {
    ctx.authorise(Permission::KedaWrite)?;
    let mut sos = state.keda_scaled_objects.write().unwrap();
    let target = sos
        .iter_mut()
        .find(|s| s.tenant == ctx.tenant && s.name == name)
        .ok_or_else(|| KedaViewError::ScaledObjectNotFound(name.into()))?;
    target.current_replicas = target.min_replicas;
    Ok(())
}

/// Resume by setting current_replicas to a midpoint between min and max
/// — KEDA's reconciler then takes over again.
pub fn resume_scaled_object(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), KedaViewError> {
    ctx.authorise(Permission::KedaWrite)?;
    let mut sos = state.keda_scaled_objects.write().unwrap();
    let target = sos
        .iter_mut()
        .find(|s| s.tenant == ctx.tenant && s.name == name)
        .ok_or_else(|| KedaViewError::ScaledObjectNotFound(name.into()))?;
    target.current_replicas = target.min_replicas
        + (target.max_replicas - target.min_replicas) / 2;
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KedaViewError> {
    let sos = list_scaled_objects(state, ctx)?;
    let jobs = list_scaled_jobs(state, ctx)?;
    let so_rows: Vec<Vec<String>> = sos
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                format!("{}/{}", s.target_kind, s.target_name),
                format!("{}-{}", s.min_replicas, s.max_replicas),
                s.current_replicas.to_string(),
            ]
        })
        .collect();
    let job_rows: Vec<Vec<String>> = jobs
        .iter()
        .map(|j| {
            vec![
                j.name.clone(),
                j.parallelism.to_string(),
                j.completions.to_string(),
                j.last_run_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">ScaledObjects ({n_so})</h2>{so_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">ScaledJobs ({n_j})</h2>{j_tbl}</section>"#,
        n_so = sos.len(),
        n_j = jobs.len(),
        so_tbl = table(&["name", "target", "range", "current"], &so_rows),
        j_tbl = table(&["name", "parallelism", "completions", "last_run_unix"], &job_rows),
    );
    Ok(page_shell(
        &format!("keda · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/keda/src/components/ScaledObjectsList.tsx",
    "ScaledObjectsList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_scaled_objects_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/keda/src/components/ScaledObjectsList.tsx",
            "ScaledObjectsList",
            "acme"
        );
        let s = AdminState::seeded();
        let so = list_scaled_objects(&s, &ctx(&[Permission::KedaRead])).unwrap();
        assert_eq!(so.len(), 2);
        assert!(so.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_scaled_jobs_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/keda/src/components/ScaledJobsList.tsx",
            "ScaledJobsList",
            "acme"
        );
        let s = AdminState::seeded();
        let j = list_scaled_jobs(&s, &ctx(&[Permission::KedaRead])).unwrap();
        assert_eq!(j.len(), 1);
        assert_eq!(j[0].name, "ingest");
    }

    #[test]
    fn pause_scaled_object_pins_to_min() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/keda/src/components/ScaledObjectActions.tsx",
            "PauseAction",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KedaRead, Permission::KedaWrite]);
        pause_scaled_object(&s, &c, "web-so").unwrap();
        let so = list_scaled_objects(&s, &c).unwrap();
        let web = so.iter().find(|x| x.name == "web-so").unwrap();
        assert_eq!(web.current_replicas, web.min_replicas);
    }

    #[test]
    fn pause_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "tenantScopeGuard",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KedaRead, Permission::KedaWrite]);
        assert!(matches!(
            pause_scaled_object(&s, &c, "evil-so").unwrap_err(),
            KedaViewError::ScaledObjectNotFound(_)
        ));
    }

    #[test]
    fn resume_scaled_object_lands_in_range() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/keda/src/components/ScaledObjectActions.tsx",
            "ResumeAction",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::KedaRead, Permission::KedaWrite]);
        resume_scaled_object(&s, &c, "api-so").unwrap();
        let so = list_scaled_objects(&s, &c).unwrap();
        let api = so.iter().find(|x| x.name == "api-so").unwrap();
        assert!(api.current_replicas >= api.min_replicas);
        assert!(api.current_replicas <= api.max_replicas);
    }

    #[test]
    fn render_excludes_evil_so() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/keda/src/components/ScaledObjectsPage.tsx",
            "ScaledObjectsPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::KedaRead])).unwrap();
        assert!(html.contains("ScaledObjects (2)"));
        assert!(html.contains("web-so"));
        assert!(!html.contains("evil-so"));
    }
}
