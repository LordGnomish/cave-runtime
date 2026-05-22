// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/sbom/components` — Dependency-Track "Components" panel. Same
//! component roster the parent page shows, but with explicit purl/license
//! sort and a license dropdown summary.
//!
//! Upstream: <https://dependencytrack.org/docs/glossary/#components>

use super::SbomViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, SbomComponent, scope};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<SbomComponent>, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let mut rows: Vec<SbomComponent> =
        scope(&state.sbom_components.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.package.cmp(&b.package).then(a.version.cmp(&b.version)));
    Ok(rows)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    let rows = list(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.package),
                escape(&r.version),
                escape(&r.license),
                escape(&r.image),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Components ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">Component browser — sorted by package then version.</p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["package", "version", "license", "image"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/sbom/components",
        &format!("sbom/components · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_is_sorted_by_package_then_version() {
        let r = list(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        for w in r.windows(2) {
            assert!(
                (w[0].package.as_str(), w[0].version.as_str())
                    <= (w[1].package.as_str(), w[1].version.as_str())
            );
        }
    }

    #[test]
    fn list_rejects_no_perm() {
        assert!(list(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_table_header() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("Components ("));
        assert!(html.contains("package"));
    }
}
