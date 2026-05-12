//! `/admin/gitops-config` — Flux HelmRelease / Kustomization parity.
//! Flux's main artefacts (HelmRelease, Kustomization, GitRepository)
//! all share the `(name, repo, path, synced_at)` shape; the page
//! groups them by repo to mirror Flux's `/sources` + `/kustomize` UI.
//!
//! Upstream: <https://fluxcd.io/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, GitopsApp};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GitopsConfigViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<GitopsApp>, GitopsConfigViewError> {
    ctx.authorise(Permission::GitopsRead)?;
    let mut rows: Vec<GitopsApp> = scope(
        &state.gitops_apps.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| {
        b.synced_at_unix
            .cmp(&a.synced_at_unix)
            .then(a.name.cmp(&b.name))
    });
    Ok(rows)
}

/// Group apps by their `repo` field so the UI can mirror Flux's
/// per-source landing page.
pub fn group_by_repo(rows: &[GitopsApp]) -> Vec<(String, Vec<GitopsApp>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<GitopsApp>> = BTreeMap::new();
    for r in rows {
        acc.entry(r.repo.clone()).or_default().push(r.clone());
    }
    let mut out: Vec<(String, Vec<GitopsApp>)> = acc.into_iter().collect();
    for (_, v) in &mut out {
        v.sort_by(|a, b| a.name.cmp(&b.name));
    }
    out
}

/// Apps whose `synced_at_unix` is at least `cutoff_unix` seconds ago
/// look stale; Flux flags those in its dashboard.
pub fn stale_since<'a>(rows: &'a [GitopsApp], stale_cutoff_unix: i64) -> Vec<&'a GitopsApp> {
    rows.iter().filter(|r| r.synced_at_unix < stale_cutoff_unix).collect()
}

pub fn detail(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<Option<GitopsApp>, GitopsConfigViewError> {
    let rows = list_records(state, ctx)?;
    Ok(rows.into_iter().find(|r| r.name == name))
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, GitopsConfigViewError> {
    let rows = list_records(state, ctx)?;
    let groups = group_by_repo(&rows);
    let group_html: String = groups
        .iter()
        .map(|(repo, items)| {
            let inner = items
                .iter()
                .map(|r| {
                    format!(
                        r#"<tr><td><code>{name}</code></td><td>{path}</td><td>{ts}</td></tr>"#,
                        name = escape(&r.name),
                        path = escape(&r.path),
                        ts = r.synced_at_unix,
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            format!(
                r#"<details open class="mb-2 p-2 bg-white rounded shadow-sm">
  <summary class="cursor-pointer font-semibold">🗂 {repo} <small class="text-gray-500">({n})</small></summary>
  <table class="mt-2 w-full text-sm"><thead><tr><th>name</th><th>path</th><th>synced</th></tr></thead><tbody>{inner}</tbody></table>
</details>"#,
                repo = escape(repo),
                n = items.len(),
                inner = inner,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.name),
                escape(&r.repo),
                escape(&r.path),
                r.synced_at_unix.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">
    Flux HelmRelease / Kustomization (cave-gitops-config).
    Upstream: <a class="text-blue-700 underline" href="https://fluxcd.io/">fluxcd.io</a>.
  </p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> apps</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{r}</strong> repos</span>
  </div>
  <h2 class="text-lg font-semibold mb-2">By repository</h2>
  {group_html}
  <h2 class="text-lg font-semibold mt-6 mb-2">Flat app list</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        r = groups.len(),
        group_html = group_html,
        tbl = table(&["name", "repo", "path", "synced"], &table_rows),
    );
    Ok(page_shell(
        &format!("gitops-config · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite =
    Cite::backstage("plugins/gitops/src/components/AppsList.tsx", "AppsList");

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
            "plugins/gitops/src/components/AppsList.tsx",
            "AppsList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::GitopsRead])).unwrap();
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
    fn list_sorted_newest_synced_first() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::GitopsRead])).unwrap();
        for w in rows.windows(2) {
            assert!(w[0].synced_at_unix >= w[1].synced_at_unix);
        }
    }

    #[test]
    fn group_by_repo_returns_one_entry_per_repo() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::GitopsRead])).unwrap();
        let groups = group_by_repo(&rows);
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, rows.len());
        let repos: Vec<&str> = groups.iter().map(|(r, _)| r.as_str()).collect();
        let mut sorted = repos.clone();
        sorted.sort();
        assert_eq!(repos, sorted);
    }

    #[test]
    fn stale_since_filters_old_apps() {
        let rows = list_records(&AdminState::seeded(), &ctx(&[Permission::GitopsRead])).unwrap();
        // Future cutoff → everything is stale.
        let cutoff = i64::MAX;
        let all_stale = stale_since(&rows, cutoff);
        assert_eq!(all_stale.len(), rows.len());
        // Past cutoff → nothing is stale.
        let none_stale = stale_since(&rows, 0);
        assert!(none_stale.is_empty());
    }

    #[test]
    fn detail_returns_app_by_name() {
        let s = AdminState::seeded();
        let rows = list_records(&s, &ctx(&[Permission::GitopsRead])).unwrap();
        if let Some(first) = rows.first() {
            let name = first.name.clone();
            assert!(detail(&s, &ctx(&[Permission::GitopsRead]), &name)
                .unwrap()
                .is_some());
        }
        assert!(detail(&s, &ctx(&[Permission::GitopsRead]), "no-such")
            .unwrap()
            .is_none());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::GitopsRead])).unwrap();
        assert!(html.contains("web-app"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::GitopsRead])).unwrap();
        assert!(!html.contains("evil-app"));
    }

    #[test]
    fn render_includes_repo_groups_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::GitopsRead])).unwrap();
        assert!(html.contains("By repository"));
        assert!(html.contains("Flat app list"));
        assert!(html.contains("fluxcd.io"));
    }
}
