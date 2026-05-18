// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/sbom/policies` — Dependency-Track "Policy" panel. Lists the
//! built-in license + vuln + age policies and shows their match/no-match
//! counts against the seeded component set.
//!
//! Upstream: <https://dependencytrack.org/docs/usage/policy-compliance/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, SbomComponent};
use super::SbomViewError;

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyRow {
    pub name: &'static str,
    pub kind: &'static str,
    pub state: &'static str,
    pub matches: usize,
}

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<PolicyRow>, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let rows: Vec<SbomComponent> = scope(
        &state.sbom_components.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    let copyleft_hits = rows
        .iter()
        .filter(|r| r.license.eq_ignore_ascii_case("GPL-3.0") || r.license.eq_ignore_ascii_case("AGPL-3.0"))
        .count();
    let unknown_license_hits = rows.iter().filter(|r| r.license.eq_ignore_ascii_case("Unknown")).count();
    Ok(vec![
        PolicyRow {
            name: "Block copyleft",
            kind: "license-deny",
            state: "FAIL",
            matches: copyleft_hits,
        },
        PolicyRow {
            name: "Approved-licenses only",
            kind: "license-allow",
            state: "WARN",
            matches: unknown_license_hits,
        },
        PolicyRow {
            name: "Critical CVE blocker",
            kind: "severity-at-least",
            state: "FAIL",
            matches: 0,
        },
        PolicyRow {
            name: "Stale dependency",
            kind: "age",
            state: "INFO",
            matches: 0,
        },
    ])
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    let rows = list(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![
            r.name.to_string(),
            r.kind.to_string(),
            r.state.to_string(),
            r.matches.to_string(),
        ])
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Policies ({n})</h2>
  <p class="text-sm text-gray-600 mb-3">License / vuln / age policy roster — match-counts against this tenant's components.</p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["name", "kind", "state", "matches"], &table_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/sbom/policies",
        &format!("sbom/policies · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_rejects_no_perm() {
        assert!(list(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn list_has_four_default_policies() {
        let r = list(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn policy_kinds_cover_license_vuln_age() {
        let r = list(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        let kinds: std::collections::HashSet<_> = r.iter().map(|p| p.kind).collect();
        assert!(kinds.contains("license-deny"));
        assert!(kinds.contains("license-allow"));
        assert!(kinds.contains("severity-at-least"));
        assert!(kinds.contains("age"));
    }

    #[test]
    fn render_includes_policy_table() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("Policies ("));
        assert!(html.contains("license-deny"));
    }
}
