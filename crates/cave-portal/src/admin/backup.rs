// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/backup` view — backup job browser + manual trigger.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, BackupJob};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BackupViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("job {0} not found in this tenant")]
    JobNotFound(String),
    #[error("job {0} is already Running")]
    JobAlreadyRunning(String),
}

pub fn list_jobs(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<BackupJob>, BackupViewError> {
    ctx.authorise(Permission::BackupRead)?;
    Ok(scope(&state.backup_jobs.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn trigger(state: &AdminState, ctx: &RequestCtx, name: &str, now_unix: i64) -> Result<(), BackupViewError> {
    ctx.authorise(Permission::BackupTrigger)?;
    let mut jobs = state.backup_jobs.write().unwrap();
    let target = jobs.iter_mut().find(|j| j.tenant == ctx.tenant && j.name == name)
        .ok_or_else(|| BackupViewError::JobNotFound(name.into()))?;
    if target.state == "Running" {
        return Err(BackupViewError::JobAlreadyRunning(name.into()));
    }
    target.state = "Running";
    target.last_run_unix = Some(now_unix);
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, BackupViewError> {
    let jobs = list_jobs(state, ctx)?;
    let rows: Vec<Vec<String>> = jobs.iter().map(|j| vec![
        j.name.clone(), j.source.clone(), j.destination.clone(),
        j.schedule_cron.clone(),
        j.last_run_unix.map(|x| x.to_string()).unwrap_or_else(|| "—".into()),
        j.state.into(),
    ]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Backup jobs ({n})</h2>{tbl}</section>"#,
        n = jobs.len(),
        tbl = table(&["name", "source", "destination", "cron", "last_run", "state"], &rows),
    );
    Ok(page_shell_full(ctx, "/admin/backup", &format!("backup · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/backup/src/components/JobsList.tsx", "JobsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_jobs_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/backup/src/components/JobsList.tsx", "JobsList", "acme");
        let s = AdminState::seeded();
        let j = list_jobs(&s, &ctx(&[Permission::BackupRead])).unwrap();
        assert_eq!(j.len(), 2);
    }

    #[test]
    fn list_jobs_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_jobs(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn trigger_marks_running_and_rejects_already_running() {
        let (_c, _t) = portal_test_ctx!("plugins/backup/src/components/TriggerButton.tsx", "Trigger", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::BackupRead, Permission::BackupTrigger]);
        // pg-prod-daily was Completed → trigger ok
        trigger(&s, &c, "pg-prod-daily", 1_003_000).unwrap();
        let j = list_jobs(&s, &c).unwrap();
        let pg = j.iter().find(|x| x.name == "pg-prod-daily").unwrap();
        assert_eq!(pg.state, "Running");
        // etcd-hourly was Running → reject
        assert!(matches!(trigger(&s, &c, "etcd-hourly", 1_003_000).unwrap_err(), BackupViewError::JobAlreadyRunning(_)));
    }

    #[test]
    fn trigger_refuses_cross_tenant() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-backend/src/PermissionsService.ts", "tenantScopeGuard", "acme");
        let s = AdminState::seeded();
        let c = ctx(&[Permission::BackupRead, Permission::BackupTrigger]);
        assert!(matches!(trigger(&s, &c, "evil-backup", 0).unwrap_err(), BackupViewError::JobNotFound(_)));
    }

    #[test]
    fn render_excludes_evil_job() {
        let (_c, _t) = portal_test_ctx!("plugins/backup/src/components/JobsPage.tsx", "JobsPage", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::BackupRead])).unwrap();
        assert!(html.contains("Backup jobs (2)"));
        assert!(!html.contains("evil-backup"));
    }
}
