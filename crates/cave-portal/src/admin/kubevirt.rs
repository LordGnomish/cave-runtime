//! `/admin/kubevirt` view — kubevirt resource browser.

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::{scope, AdminState, VirtualMachine};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KubevirtViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<VirtualMachine>, KubevirtViewError> {
    ctx.authorise(Permission::KubevirtRead)?;
    Ok(scope(&state.virtual_machines.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter().cloned().collect())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, KubevirtViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows.iter().map(|r| vec![r.name.clone(), r.phase.into(), r.cpu.to_string(), r.memory_mib.to_string()]).collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Kubevirt ({n})</h2>{tbl}</section>"#,
        n = rows.len(),
        tbl = table(&["name", "phase", "cpu", "memory_mib"], &table_rows),
    );
    Ok(page_shell(&format!("kubevirt · {}", escape(ctx.tenant.as_str())), &body))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/kubevirt/src/components/VmsList.tsx", "VmsList");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!("plugins/kubevirt/src/components/VmsList.tsx", "VmsList", "acme");
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::KubevirtRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!("plugins/permission-react/src/PermissionApi.ts", "authorize", "acme");
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!("plugins/kubevirt/src/components/VmsList.tsx", "RenderOwner", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        assert!(html.contains("vm-1"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!("plugins/kubevirt/src/components/VmsList.tsx", "RenderEvil", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        assert!(!html.contains("evil-vm"));
    }

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!("plugins/kubevirt/src/components/VmsList.tsx", "Count", "acme");
        let html = render(&AdminState::seeded(), &ctx(&[Permission::KubevirtRead])).unwrap();
        assert!(html.contains("(2)"));
    }
}
