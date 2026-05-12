//! Leases tab — `etcdctl lease list` parity.

use super::EtcdViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState, EtcdLease};

pub fn list_leases(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<EtcdLease>, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    let leases = state.etcd_leases.read().unwrap();
    Ok(scope(&leases, &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn total_keys_per_lease(rows: &[EtcdLease]) -> u32 {
    rows.iter().map(|l| l.keys.len() as u32).sum()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, EtcdViewError> {
    let leases = list_leases(state, ctx)?;
    let rows: Vec<Vec<String>> = leases
        .iter()
        .map(|l| {
            vec![
                l.lease_id.to_string(),
                format!("{}s", l.ttl_seconds),
                l.keys.join(", "),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="etcd-leases" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Leases ({n}, {k} keys bound)</h2>
  {tbl}
</section>"#,
        n = leases.len(),
        k = total_keys_per_lease(&leases),
        tbl = table(&["lease", "ttl", "keys"], &rows),
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
    fn list_leases_filters_to_tenant() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "EtcdLeaseTab",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = list_leases(&state, &ctx(&[Permission::EtcdRead])).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].lease_id, 1001);
    }

    #[test]
    fn list_leases_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_leases(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn total_keys_per_lease_sums_keys() {
        let s = AdminState::seeded();
        let rows = list_leases(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        let manual: u32 = rows.iter().map(|l| l.keys.len() as u32).sum();
        assert_eq!(total_keys_per_lease(&rows), manual);
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        for col in ["lease", "ttl", "keys"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
