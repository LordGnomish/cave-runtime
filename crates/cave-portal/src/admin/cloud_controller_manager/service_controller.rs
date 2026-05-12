//! Service controller tab — LoadBalancer provisioning state.

use super::CloudControllerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LbServiceRow {
    pub name: String,
    pub namespace: String,
    pub external_ip: String,
    pub state: &'static str, // "Provisioned" | "Provisioning" | "Failed"
}

pub fn list_load_balancers(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<LbServiceRow>, CloudControllerViewError> {
    let vols = super::volume_controller::list_volumes(state, ctx)?;
    Ok(vols
        .into_iter()
        .enumerate()
        .map(|(idx, v)| LbServiceRow {
            name: format!("{}-lb", v.id),
            namespace: "default".into(),
            external_ip: format!("203.0.113.{}", 10 + idx),
            state: if v.attached_node.is_some() {
                "Provisioned"
            } else {
                "Provisioning"
            },
        })
        .collect())
}

pub fn provisioned_count(rows: &[LbServiceRow]) -> usize {
    rows.iter().filter(|r| r.state == "Provisioned").count()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CloudControllerViewError> {
    let rows = list_load_balancers(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.namespace.clone(),
                r.external_ip.clone(),
                r.state.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="ccm-services" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">ServiceController ({n}, {p} Provisioned)</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        p = provisioned_count(&rows),
        tbl = table(
            &["name", "namespace", "externalIP", "state"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_load_balancers_one_per_volume() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/LoadBalancers.tsx",
            "LB",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_load_balancers(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        let vols = super::super::volume_controller::list_volumes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        assert_eq!(rows.len(), vols.len());
    }

    #[test]
    fn list_load_balancers_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_load_balancers(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn state_matches_attached_node_flag() {
        let s = AdminState::seeded();
        let rows = list_load_balancers(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        let vols = super::super::volume_controller::list_volumes(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for (r, v) in rows.iter().zip(vols.iter()) {
            let expected = if v.attached_node.is_some() {
                "Provisioned"
            } else {
                "Provisioning"
            };
            assert_eq!(r.state, expected);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for col in ["name", "namespace", "externalIP", "state"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
