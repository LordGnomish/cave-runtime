// SPDX-License-Identifier: AGPL-3.0-or-later
//! Controllers tab — full registered controller catalog (k-c-m has
//! ~30 built-in controllers in modern upstream releases). Each row
//! tells the operator whether the controller is enabled and how
//! many leases the local node currently holds for it.

use super::ControllerManagerViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerRow {
    pub name: &'static str,
    pub enabled: bool,
    pub leases_held: u32,
}

/// Names taken from `kube-controller-manager` `--controllers` default
/// in upstream release-1.31 (a subset for shape).
const ALL_CONTROLLERS: &[&str] = &[
    "deployment",
    "replicaset",
    "statefulset",
    "daemonset",
    "job",
    "cronjob",
    "endpoint",
    "endpoint-slice",
    "garbagecollector",
    "horizontalpodautoscaling",
    "ingress",
    "namespace",
    "node-ipam",
    "node-lifecycle",
    "persistentvolume-binder",
    "persistentvolume-expander",
    "podgc",
    "pv-protection",
    "pvc-protection",
    "resourcequota",
    "root-ca-cert-publisher",
    "route",
    "service",
    "service-account",
    "service-account-token",
    "serviceaccount",
    "statefulset",
    "tokencleaner",
    "ttl",
    "ttl-after-finished",
];

pub fn list_controllers(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ControllerRow>, ControllerManagerViewError> {
    let leases = super::leader_election::list_leases(state, ctx)?;
    Ok(ALL_CONTROLLERS
        .iter()
        .map(|name| {
            let leases_held = leases.iter().filter(|l| l.controller == *name).count() as u32;
            ControllerRow {
                name,
                enabled: true,
                leases_held,
            }
        })
        .collect())
}

pub fn enabled_count(rows: &[ControllerRow]) -> usize {
    rows.iter().filter(|r| r.enabled).count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, ControllerManagerViewError> {
    let rows = list_controllers(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|c| {
            vec![
                c.name.into(),
                if c.enabled { "Enabled" } else { "Disabled" }.into(),
                c.leases_held.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cm-controllers" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Controllers ({n}, {e} Enabled)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        e = enabled_count(&rows),
        tbl = table(&["name", "state", "leases"], &table_rows),
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
    fn list_controllers_includes_canonical_names() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Controllers.tsx",
            "Controllers",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_controllers(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        let names: std::collections::HashSet<_> = rows.iter().map(|r| r.name).collect();
        for n in ["deployment", "replicaset", "daemonset", "job", "service"] {
            assert!(names.contains(n));
        }
    }

    #[test]
    fn list_controllers_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_controllers(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn lease_count_matches_seeded_leases() {
        let s = AdminState::seeded();
        let rows = list_controllers(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        // Seed has 2 leases for acme (deployment + replicaset).
        let total: u32 = rows.iter().map(|r| r.leases_held).sum();
        assert_eq!(total, 2);
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for col in ["name", "state", "leases"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
