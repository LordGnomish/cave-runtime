// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Leader election tab — `kube-system/Lease` browser. Mirrors the
//! upstream kube-controller-manager leader-election surface.

use super::ControllerManagerViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{AdminState, ControllerLease, scope};

pub fn list_leases(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ControllerLease>, ControllerManagerViewError> {
    ctx.authorise(Permission::ControllerManagerRead)?;
    let mut rows: Vec<ControllerLease> =
        scope(&state.controller_leases.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.controller.cmp(&b.controller));
    Ok(rows)
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, ControllerManagerViewError> {
    let leases = list_leases(state, ctx)?;
    let rows: Vec<Vec<String>> = leases
        .iter()
        .map(|l| {
            vec![
                l.controller.clone(),
                l.leader_id.clone(),
                l.renewals.to_string(),
                l.expires_unix.to_string(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="cm-leader-election" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Leader election ({n})</h2>
  {tbl}
</section>"#,
        n = leases.len(),
        tbl = table(&["controller", "leader", "renewals", "expires_unix"], &rows),
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
    fn list_leases_filters_to_owner_and_sorts() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Resources/Leases.tsx",
            "LeaseList",
            "acme"
        );
        let s = AdminState::seeded();
        let l = list_leases(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].controller, "deployment");
        assert_eq!(l[1].controller, "replicaset");
    }

    #[test]
    fn list_leases_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_leases(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_leases_excludes_evil_controllers() {
        let s = AdminState::seeded();
        let l = list_leases(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        assert!(!l.iter().any(|x| x.controller == "evil-loop"));
    }

    #[test]
    fn render_section_includes_leader_id() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::ControllerManagerRead])).unwrap();
        for col in ["controller", "leader", "renewals", "expires_unix"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
