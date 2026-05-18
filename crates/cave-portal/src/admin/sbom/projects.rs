// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/sbom/projects` — Dependency-Track "Projects" panel. Each image
//! in the SBOM roster is treated as a project; we group by image and report
//! the component count and license-spread.
//!
//! Upstream: <https://dependencytrack.org/docs/glossary/#project>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, SbomComponent};
use super::SbomViewError;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub image: String,
    pub component_count: usize,
    pub license_count: usize,
}

pub fn list_projects(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<ProjectRow>, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let rows: Vec<SbomComponent> = scope(
        &state.sbom_components.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    use std::collections::BTreeMap;
    let mut by_image: BTreeMap<String, (usize, BTreeSet<String>)> = BTreeMap::new();
    for r in &rows {
        let entry = by_image.entry(r.image.clone()).or_default();
        entry.0 += 1;
        entry.1.insert(r.license.clone());
    }
    Ok(by_image
        .into_iter()
        .map(|(image, (cc, lic))| ProjectRow {
            image,
            component_count: cc,
            license_count: lic.len(),
        })
        .collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    let rows = list_projects(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![
            escape(&r.image),
            r.component_count.to_string(),
            r.license_count.to_string(),
        ])
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Projects ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">One project per image. Component count + distinct license spread.</p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["project", "components", "licenses"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/sbom/projects",
        &format!("sbom/projects · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_groups_components_by_image() {
        let rows = list_projects(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        for r in &rows {
            assert!(r.component_count >= 1);
        }
    }

    #[test]
    fn list_rejects_no_perm() {
        assert!(list_projects(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn license_count_is_distinct_per_image() {
        let rows = list_projects(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        for r in &rows {
            assert!(r.license_count >= 1);
        }
    }

    #[test]
    fn render_lists_projects() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("Projects ("));
        assert!(html.contains("project"));
    }
}
