// SPDX-License-Identifier: AGPL-3.0-or-later
//! Volume controller tab — managed cloud volumes (attach/detach).

use super::CloudControllerViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState, CloudVolume};

pub fn list_volumes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<CloudVolume>, CloudControllerViewError> {
    ctx.authorise(Permission::CloudControllerRead)?;
    Ok(scope(&state.cloud_volumes.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn unattached_volumes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<CloudVolume>, CloudControllerViewError> {
    let all = list_volumes(state, ctx)?;
    Ok(all.into_iter().filter(|v| v.attached_node.is_none()).collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CloudControllerViewError> {
    let vols = list_volumes(state, ctx)?;
    let rows: Vec<Vec<String>> = vols
        .iter()
        .map(|v| {
            vec![
                v.id.clone(),
                v.region.clone(),
                format!("{} GB", v.size_gb),
                v.attached_node.clone().unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="ccm-volumes" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Volumes ({n}, {u} unattached)</h2>
  {tbl}
</section>"#,
        n = vols.len(),
        u = unattached_volumes(state, ctx)?.len(),
        tbl = table(&["id", "region", "size", "attached"], &rows),
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
    fn list_volumes_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/Volumes.tsx",
            "VolumesList",
            "acme"
        );
        let s = AdminState::seeded();
        let v = list_volumes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn unattached_volumes_filters_correctly() {
        let s = AdminState::seeded();
        let u = unattached_volumes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert!(u.iter().all(|v| v.attached_node.is_none()));
    }

    #[test]
    fn list_volumes_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_volumes(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_includes_volume_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for col in ["id", "region", "size", "attached"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
