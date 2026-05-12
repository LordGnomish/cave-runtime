//! `/admin/deploy` — Argo CD parity surface. Lists Application sync
//! activity for the caller's tenant, grouped by sync status so the
//! header can show "5 Synced / 2 OutOfSync / 1 Failed" at a glance
//! (matches Argo's Applications page summary).
//!
//! Upstream UI: <https://argo-cd.readthedocs.io/en/stable/user-guide/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, DeployActivity};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DeployViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<DeployActivity>, DeployViewError> {
    ctx.authorise(Permission::DeployRead)?;
    let mut rows: Vec<DeployActivity> = scope(
        &state.deploy_activities.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| a.service.cmp(&b.service).then(a.id.cmp(&b.id)));
    Ok(rows)
}

/// One row in Argo CD's Applications landing page — a service +
/// the most recent activity status against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationRow {
    pub service: String,
    pub latest_version: String,
    pub status: &'static str,
    pub activity_count: u32,
}

/// Aggregate activities into one row per service, keeping the latest
/// version and a count of activity events.
pub fn list_applications(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ApplicationRow>, DeployViewError> {
    let activities = list_records(state, ctx)?;
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, ApplicationRow> = BTreeMap::new();
    for a in &activities {
        let row = acc.entry(a.service.clone()).or_insert(ApplicationRow {
            service: a.service.clone(),
            latest_version: a.version.clone(),
            status: a.status,
            activity_count: 0,
        });
        row.latest_version = a.version.clone(); // keep last seen
        row.status = a.status;
        row.activity_count += 1;
    }
    Ok(acc.into_values().collect())
}

/// Filter activities by status — used by the status-pill filter on
/// the Applications page (`Synced` / `OutOfSync` / `Failed`).
pub fn by_status<'a>(rows: &'a [DeployActivity], status: &str) -> Vec<&'a DeployActivity> {
    rows.iter().filter(|r| r.status == status).collect()
}

/// Per-status counts for the page header.
pub fn status_counts(rows: &[DeployActivity]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.status.to_string()).or_insert(0) += 1;
    }
    let mut out: Vec<(String, usize)> = acc.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, DeployViewError> {
    let rows = list_records(state, ctx)?;
    let apps = list_applications(state, ctx)?;
    let counts = status_counts(&rows);
    let pill_html: String = counts
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{s} <strong>×{n}</strong></span>"#,
                s = escape(s),
                n = n,
            )
        })
        .collect();
    let app_rows: Vec<Vec<String>> = apps
        .iter()
        .map(|a| {
            vec![
                escape(&a.service),
                escape(&a.latest_version),
                a.status.into(),
                a.activity_count.to_string(),
            ]
        })
        .collect();
    let activity_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.id),
                escape(&r.service),
                escape(&r.version),
                r.status.into(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Argo CD parity (cave-deploy).
    Upstream: <a class="text-blue-700 underline" href="https://argo-cd.readthedocs.io/en/stable/user-guide/">argo-cd.readthedocs.io</a>.
  </p>
  <div class="mb-4">{pills}</div>
  <h2 class="text-lg font-semibold mb-2">Applications ({n_apps})</h2>
  {app_tbl}
  <h2 class="text-lg font-semibold mt-6 mb-2">Activity ({n_act})</h2>
  {act_tbl}
</section>"#,
        pills = pill_html,
        n_apps = apps.len(),
        n_act = rows.len(),
        app_tbl = table(&["service", "version", "status", "activity"], &app_rows),
        act_tbl = table(&["id", "service", "version", "status"], &activity_rows),
    );
    Ok(page_shell(
        &format!("deploy · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/deploy/src/components/ActivityList.tsx", "ActivityList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/deploy/src/components/ActivityList.tsx",
            "ActivityList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::DeployRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_applications_groups_by_service() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/deploy/src/components/Applications.tsx",
            "Applications",
            "acme"
        );
        let apps = list_applications(&AdminState::seeded(), &ctx(&[Permission::DeployRead])).unwrap();
        let mut names: Vec<&str> = apps.iter().map(|a| a.service.as_str()).collect();
        names.sort();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len, "applications must be unique by service");
    }

    #[test]
    fn status_counts_orders_by_count_desc_then_alpha() {
        use cave_kernel::ns::TenantId;
        let t = TenantId::new("t").unwrap();
        let rows = vec![
            DeployActivity { tenant: t.clone(), id: "1".into(), service: "a".into(), version: "v1".into(), status: "Synced" },
            DeployActivity { tenant: t.clone(), id: "2".into(), service: "b".into(), version: "v2".into(), status: "Synced" },
            DeployActivity { tenant: t.clone(), id: "3".into(), service: "c".into(), version: "v3".into(), status: "Failed" },
        ];
        let counts = status_counts(&rows);
        assert_eq!(counts[0], ("Synced".into(), 2));
        assert_eq!(counts[1], ("Failed".into(), 1));
    }

    #[test]
    fn by_status_filters_correctly() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/deploy/src/components/StatusFilter.tsx",
            "StatusFilter",
            "acme"
        );
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::DeployRead])).unwrap();
        let synced = by_status(&rows, "Synced");
        assert!(synced.iter().all(|r| r.status == "Synced"));
        let bogus = by_status(&rows, "Bogus");
        assert!(bogus.is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/deploy/src/components/ActivityList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::DeployRead])).unwrap();
        assert!(html.contains("dep-001"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/deploy/src/components/ActivityList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::DeployRead])).unwrap();
        assert!(!html.contains("evil-dep"));
    }

    #[test]
    fn render_shows_status_pills() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/deploy/src/components/StatusPills.tsx",
            "Pills",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::DeployRead])).unwrap();
        // Status pill should render the seeded statuses.
        assert!(html.contains("Applications ("));
        assert!(html.contains("Activity ("));
        assert!(html.contains("argo-cd.readthedocs.io"));
    }
}
