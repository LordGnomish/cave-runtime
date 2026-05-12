//! Audit-log tab — append-only Vault audit entries. Mirrors the
//! Vault UI's audit tail surface: newest first, with the operation
//! kind and the principal that issued it.

use super::VaultViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::{scope, AdminState, VaultAuditEntry};

pub fn list_audit(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<VaultAuditEntry>, VaultViewError> {
    ctx.authorise(Permission::VaultRead)?;
    let mut rows: Vec<VaultAuditEntry> = scope(
        &state.vault_audit.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .cloned()
    .collect();
    rows.sort_by(|a, b| b.time_unix.cmp(&a.time_unix));
    Ok(rows)
}

/// Filter the audit tail to one operation kind. Mirrors
/// `vault audit list --op=<op>`.
pub fn by_op<'a>(
    entries: &'a [VaultAuditEntry],
    op: &str,
) -> Vec<&'a VaultAuditEntry> {
    entries.iter().filter(|e| e.op == op).collect()
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, VaultViewError> {
    let rows = list_audit(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|a| {
            vec![
                a.time_unix.to_string(),
                escape(&a.principal),
                a.op.into(),
                escape(&a.path),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="audit" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Audit ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["time", "principal", "op", "path"], &table_rows),
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
    fn list_returns_newest_first() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/AuditTail.tsx",
            "AuditTail",
            "acme"
        );
        let s = AdminState::seeded();
        let audit = list_audit(&s, &ctx(&[Permission::VaultRead])).unwrap();
        assert_eq!(audit.len(), 2);
        assert!(audit[0].time_unix >= audit[1].time_unix);
    }

    #[test]
    fn list_refuses_without_permission() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_audit(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn by_op_filters_by_kind() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/auth-react/src/components/UserSettings/AuditByOp.tsx",
            "ByOp",
            "acme"
        );
        let s = AdminState::seeded();
        let audit = list_audit(&s, &ctx(&[Permission::VaultRead])).unwrap();
        let reads = by_op(&audit, "read-meta");
        assert_eq!(reads.len(), 2);
        let writes = by_op(&audit, "write");
        assert!(writes.is_empty());
    }
}
