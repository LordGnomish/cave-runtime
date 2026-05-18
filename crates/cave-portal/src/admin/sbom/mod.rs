// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/sbom` — Dependency-Track parity. Top-level component browser plus
//! sub-pages for projects, vulnerabilities, policies, portfolio metrics, BOM
//! ingest, notifications. Each tab mirrors a Dependency-Track UI panel.
//!
//! Upstream UI: <https://dependencytrack.org/>
//!
//! Sub-pages:
//! * [`components`]   — full component browser (default landing)
//! * [`projects`]     — project roster + last-import timestamp
//! * [`vulnerabilities`] — CVE table joined to affected components
//! * [`policies`]     — license / vuln / age policy roster
//! * [`portfolio`]    — Inherited-Risk-Score grid + trend sparkline
//! * [`ingest`]       — paste-and-parse BOM uploader
//! * [`notifications`]— rule roster + sink-config preview

pub mod components;
pub mod ingest;
pub mod notifications;
pub mod policies;
pub mod portfolio;
pub mod projects;
pub mod vulnerabilities;

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, SbomComponent};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SbomViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SbomComponent>, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let mut rows: Vec<SbomComponent> = scope(&state.sbom_components.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect();
    rows.sort_by(|a, b| a.image.cmp(&b.image).then(a.package.cmp(&b.package)));
    Ok(rows)
}

pub fn group_by_image(rows: &[SbomComponent]) -> Vec<(String, Vec<SbomComponent>)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, Vec<SbomComponent>> = BTreeMap::new();
    for r in rows { acc.entry(r.image.clone()).or_default().push(r.clone()); }
    acc.into_iter().collect()
}

pub fn unique_licenses(rows: &[SbomComponent]) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut set: BTreeSet<String> = BTreeSet::new();
    for r in rows { set.insert(r.license.clone()); }
    set.into_iter().collect()
}

pub fn by_license<'a>(rows: &'a [SbomComponent], license: &str) -> Vec<&'a SbomComponent> {
    rows.iter().filter(|r| r.license == license).collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    let rows = list_records(state, ctx)?;
    let licenses = unique_licenses(&rows);
    let images = group_by_image(&rows);
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![
        escape(&r.image), escape(&r.package), escape(&r.version), escape(&r.license),
    ]).collect();
    let body = format!(
        r#"<section>
  <p class="text-sm text-gray-600 mb-3">Dependency-Track (cave-sbom). Upstream: <a class="text-blue-700 underline" href="https://dependencytrack.org/">dependencytrack.org</a>.</p>
  <div class="mb-4 flex gap-4 text-sm">
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{n}</strong> components</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{i}</strong> images</span>
    <span class="px-2 py-1 rounded bg-gray-200"><strong>{l}</strong> licenses</span>
  </div>
  <h2 class="text-lg font-semibold mb-2">Components ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        i = images.len(),
        l = licenses.len(),
        tbl = table(&["image", "package", "version", "license"], &table_rows),
    );
    Ok(page_shell_full(ctx, "/admin/sbom", &format!("sbom · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/sbom/src/components/ComponentsList.tsx", "ComponentsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner_sorted_by_image_then_package() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert_eq!(r.len(), 2);
        for w in r.windows(2) {
            assert!((w[0].image.as_str(), w[0].package.as_str()) <= (w[1].image.as_str(), w[1].package.as_str()));
        }
    }

    #[test]
    fn list_refuses_without_perm() {
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn group_by_image_collects() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        let g = group_by_image(&r);
        assert_eq!(g.iter().map(|(_, v)| v.len()).sum::<usize>(), r.len());
    }

    #[test]
    fn unique_licenses_dedup() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        let u = unique_licenses(&r);
        let set: std::collections::BTreeSet<&str> = r.iter().map(|x| x.license.as_str()).collect();
        assert_eq!(u.len(), set.len());
    }

    #[test]
    fn by_license_filters() {
        let r = list_records(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        if let Some(f) = r.first() {
            let l = f.license.clone();
            assert!(by_license(&r, &l).iter().all(|x| x.license == l));
        }
        assert!(by_license(&r, "no-such").is_empty());
    }

    #[test]
    fn render_contains_owner_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("web:v17"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(!html.contains("evil:x"));
    }

    #[test]
    fn render_includes_license_count_and_upstream_link() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("licenses"));
        assert!(html.contains("dependencytrack.org"));
    }
}
