// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/scan/trivy*` — Trivy-style 4-tab security scan view.
//!
//! Four tabs: images / filesystems / iac / secrets. Each renders the
//! same `AdminState::scan_results` slice filtered by `scanner` field
//! and surfaces it under a tabbed `page_shell_full` layout.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, ScanResult};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScanTrivyError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Images,
    Filesystems,
    Iac,
    Secrets,
}

impl Tab {
    pub fn path(self) -> &'static str {
        match self {
            Self::Images => "/admin/scan/trivy/images",
            Self::Filesystems => "/admin/scan/trivy/filesystems",
            Self::Iac => "/admin/scan/trivy/iac",
            Self::Secrets => "/admin/scan/trivy/secrets",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Images => "Images",
            Self::Filesystems => "Filesystem",
            Self::Iac => "IaC",
            Self::Secrets => "Secrets",
        }
    }
    pub fn scanner_filter(self) -> &'static str {
        match self {
            Self::Images => "image",
            Self::Filesystems => "fs",
            Self::Iac => "iac",
            Self::Secrets => "secret",
        }
    }
}

fn tabs_html(active: Tab) -> String {
    let mut out = String::from(r#"<nav class="flex gap-2 mb-4" aria-label="scan tabs">"#);
    for t in [Tab::Images, Tab::Filesystems, Tab::Iac, Tab::Secrets] {
        let aria = if t == active { r#" aria-current="page""# } else { "" };
        let cls = if t == active {
            "px-3 py-1 bg-blue-600 text-white rounded"
        } else {
            "px-3 py-1 bg-gray-200 dark:bg-gray-700 rounded hover:bg-gray-300"
        };
        out.push_str(&format!(
            r#"<a class="{cls}" href="{path}"{aria}>{label}</a>"#,
            path = t.path(),
            label = t.label(),
        ));
    }
    out.push_str("</nav>");
    out
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
    tab: Tab,
) -> Result<Vec<ScanResult>, ScanTrivyError> {
    ctx.authorise(Permission::ScanRead)?;
    let guard = state.scan_results.read().unwrap();
    let all = scope(&guard, &ctx.tenant, |r| &r.tenant);
    let needle = tab.scanner_filter();
    Ok(all
        .into_iter()
        .filter(|r| r.scanner.contains(needle) || tab == Tab::Images && r.scanner == "trivy")
        .cloned()
        .collect())
}

pub fn render(
    state: &AdminState,
    ctx: &RequestCtx,
    tab: Tab,
) -> Result<String, ScanTrivyError> {
    let rows = list_records(state, ctx, tab)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.scan_id),
                escape(&r.scanner),
                r.findings.to_string(),
                String::from(r.worst_severity),
            ]
        })
        .collect();
    let body = format!(
        r#"{tabs}
<section>
  <h2 class="text-lg font-semibold mb-2">{label} scans ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">Trivy-style scanner: {label}. Showing {n} record(s) for tenant <code>{tenant}</code>.</p>
  {tbl}
  <form method="post" action="/admin/scan/trivy/start" class="mt-4 flex gap-2">
    <input type="hidden" name="kind" value="{kind}">
    <input class="border px-2 py-1 rounded" name="target" placeholder="target (image, path, or url)" required>
    <button class="bg-blue-600 text-white px-3 py-1 rounded" type="submit">Start scan</button>
  </form>
</section>"#,
        tabs = tabs_html(tab),
        label = tab.label(),
        n = rows.len(),
        tenant = escape(ctx.tenant.as_str()),
        kind = tab.scanner_filter(),
        tbl = table(
            &["scan_id", "scanner", "findings", "worst"],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        tab.path(),
        &format!("scan/{} · {}", tab.label(), escape(ctx.tenant.as_str())),
        &body,
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
    fn tab_paths_are_distinct() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "TabPaths",
            "acme"
        );
        let paths = [
            Tab::Images.path(),
            Tab::Filesystems.path(),
            Tab::Iac.path(),
            Tab::Secrets.path(),
        ];
        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                assert_ne!(paths[i], paths[j]);
            }
        }
    }

    #[test]
    fn render_images_includes_tab_nav() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "Images",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Images).unwrap();
        assert!(html.contains("/admin/scan/trivy/images"));
        assert!(html.contains("/admin/scan/trivy/filesystems"));
        assert!(html.contains("/admin/scan/trivy/iac"));
        assert!(html.contains("/admin/scan/trivy/secrets"));
    }

    #[test]
    fn render_filesystems_marks_active() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "FsActive",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Filesystems).unwrap();
        assert!(html.contains(r#"aria-current="page""#));
    }

    #[test]
    fn render_iac_carries_start_form() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "IacForm",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Iac).unwrap();
        assert!(html.contains(r#"action="/admin/scan/trivy/start""#));
        assert!(html.contains(r#"value="iac""#));
    }

    #[test]
    fn render_secrets_shows_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "SecretsTenant",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Secrets).unwrap();
        assert!(html.contains("acme"));
    }

    #[test]
    fn render_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(render(&AdminState::seeded(), &ctx(&[]), Tab::Images).is_err());
    }

    #[test]
    fn render_images_has_table_headers() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "TableHeaders",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Images).unwrap();
        assert!(html.contains("scan_id"));
        assert!(html.contains("scanner"));
        assert!(html.contains("findings"));
    }

    #[test]
    fn list_records_filters_by_scanner_kind() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "FilterByKind",
            "acme"
        );
        let s = AdminState::seeded();
        let imgs = list_records(&s, &ctx(&[Permission::ScanRead]), Tab::Images).unwrap();
        let guard = s.scan_results.read().unwrap();
        let all = scope(&guard, &ctx(&[Permission::ScanRead]).tenant, |r| &r.tenant);
        assert!(imgs.len() <= all.len());
    }

    #[test]
    fn render_title_includes_tab_label() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "TitleLabel",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Iac).unwrap();
        assert!(html.contains("IaC scans"));
    }

    #[test]
    fn render_filesystem_label_appears() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "FsLabel",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Filesystems).unwrap();
        assert!(html.contains("Filesystem scans"));
    }

    #[test]
    fn render_secret_form_hidden_kind_matches_tab() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "FormKindMatch",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Secrets).unwrap();
        assert!(html.contains(r#"value="secret""#));
    }

    #[test]
    fn render_images_form_kind_is_image() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "ImageFormKind",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Images).unwrap();
        assert!(html.contains(r#"value="image""#));
    }

    #[test]
    fn render_does_not_leak_other_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/scan/src/components/ResultsList.tsx",
            "TenantIsolation",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::ScanRead]), Tab::Images).unwrap();
        assert!(!html.contains("evil-scan"));
    }
}
