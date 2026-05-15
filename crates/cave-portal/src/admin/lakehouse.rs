// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/lakehouse` view — Iceberg table browser + snapshot timeline.
//!
//! Mirrors the Apache Iceberg REST catalog dashboard: namespaces +
//! tables with their format version, partition/file counts, total
//! size, and a per-table snapshot history. The single mutator is
//! `tag_snapshot`, which marks the current snapshot as a named
//! reference (Iceberg branches/tags semantics).

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState, LakehouseSnapshot, LakehouseTable};
use crate::admin::types::Cite;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LakehouseViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("table {0}.{1} not found in this tenant")]
    TableNotFound(String, String),
    #[error("snapshot {0} not found for {1}.{2}")]
    SnapshotNotFound(u64, String, String),
    #[error("op must be Append, Overwrite, Delete or Replace")]
    InvalidOp,
}

pub fn list_tables(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<LakehouseTable>, LakehouseViewError> {
    ctx.authorise(Permission::LakehouseRead)?;
    Ok(scope(&state.lakehouse_tables.read().unwrap(), &ctx.tenant, |r| &r.tenant)
        .into_iter()
        .cloned()
        .collect())
}

pub fn list_snapshots(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    table: &str,
) -> Result<Vec<LakehouseSnapshot>, LakehouseViewError> {
    ctx.authorise(Permission::LakehouseRead)?;
    let mut rows: Vec<LakehouseSnapshot> = scope(
        &state.lakehouse_snapshots.read().unwrap(),
        &ctx.tenant,
        |r| &r.tenant,
    )
    .into_iter()
    .filter(|s| s.namespace == namespace && s.table == table)
    .cloned()
    .collect();
    // Newest first.
    rows.sort_by(|a, b| b.committed_unix.cmp(&a.committed_unix));
    Ok(rows)
}

/// Aggregate file/size totals by namespace — used for the top-level
/// dashboard cards.
pub fn namespace_totals(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<BTreeMap<String, (u64, u64)>, LakehouseViewError> {
    let tables = list_tables(state, ctx)?;
    let mut out: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for t in tables {
        let entry = out.entry(t.namespace).or_insert((0, 0));
        entry.0 += t.file_count;
        entry.1 += t.size_bytes;
    }
    Ok(out)
}

/// Append a new snapshot with the supplied op + added-files count.
/// Mirrors the Iceberg `Append`/`Overwrite`/`Delete`/`Replace` ops.
/// The new snapshot becomes the current snapshot for its table.
pub fn append_snapshot(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    table: &str,
    snapshot_id: u64,
    committed_unix: i64,
    op: &str,
    added_files: u64,
) -> Result<(), LakehouseViewError> {
    ctx.authorise(Permission::LakehouseSnapshot)?;
    let normalised: &'static str = match op {
        "Append" => "Append",
        "Overwrite" => "Overwrite",
        "Delete" => "Delete",
        "Replace" => "Replace",
        _ => return Err(LakehouseViewError::InvalidOp),
    };
    {
        let tables = state.lakehouse_tables.read().unwrap();
        if !tables.iter().any(|t| {
            t.tenant == ctx.tenant && t.namespace == namespace && t.name == table
        }) {
            return Err(LakehouseViewError::TableNotFound(
                namespace.into(),
                table.into(),
            ));
        }
    }
    state.lakehouse_snapshots.write().unwrap().push(LakehouseSnapshot {
        tenant: ctx.tenant.clone(),
        namespace: namespace.into(),
        table: table.into(),
        snapshot_id,
        committed_unix,
        op: normalised,
        added_files,
    });
    if let Some(t) = state
        .lakehouse_tables
        .write()
        .unwrap()
        .iter_mut()
        .find(|t| t.tenant == ctx.tenant && t.namespace == namespace && t.name == table)
    {
        t.current_snapshot_id = snapshot_id;
        t.file_count = t.file_count.saturating_add(added_files);
    }
    Ok(())
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, LakehouseViewError> {
    let tables = list_tables(state, ctx)?;
    let totals = namespace_totals(state, ctx)?;
    let t_rows: Vec<Vec<String>> = tables
        .iter()
        .map(|t| {
            vec![
                t.namespace.clone(),
                t.name.clone(),
                format!("v{}", t.format_version),
                t.partition_count.to_string(),
                t.file_count.to_string(),
                format!("{} B", t.size_bytes),
                t.current_snapshot_id.to_string(),
            ]
        })
        .collect();
    let n_rows: Vec<Vec<String>> = totals
        .iter()
        .map(|(ns, (files, size))| {
            vec![ns.clone(), files.to_string(), format!("{} B", size)]
        })
        .collect();
    let body = format!(
        r#"<section><h2 class="text-lg font-semibold mb-2">Tables ({n_t})</h2>{t_tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Namespace totals ({n_n})</h2>{n_tbl}</section>"#,
        n_t = tables.len(),
        n_n = totals.len(),
        t_tbl = table(
            &["namespace", "table", "fmt", "partitions", "files", "size", "snapshot"],
            &t_rows,
        ),
        n_tbl = table(&["namespace", "files", "size"], &n_rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/lakehouse",
        &format!("lakehouse · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/lakehouse/src/components/Tables/TablesPage.tsx",
    "TablesPage",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_tables_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/lakehouse/src/components/Tables/TablesList.tsx",
            "TablesList",
            "acme"
        );
        let s = AdminState::seeded();
        let t = list_tables(&s, &ctx(&[Permission::LakehouseRead])).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.tenant.as_str() == "acme"));
        assert!(t.iter().all(|x| x.namespace == "warehouse"));
    }

    #[test]
    fn list_snapshots_returns_newest_first_within_table() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/lakehouse/src/components/Snapshots/SnapshotTimeline.tsx",
            "SnapshotTimeline",
            "acme"
        );
        let s = AdminState::seeded();
        let snaps = list_snapshots(&s, &ctx(&[Permission::LakehouseRead]), "warehouse", "orders")
            .unwrap();
        assert_eq!(snaps.len(), 2);
        assert!(snaps[0].committed_unix > snaps[1].committed_unix);
        assert_eq!(snaps[0].snapshot_id, 1001);
    }

    #[test]
    fn list_snapshots_does_not_leak_evil_namespace() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-backend/src/PermissionsService.ts",
            "tenantScopeGuard",
            "acme"
        );
        let s = AdminState::seeded();
        let snaps = list_snapshots(&s, &ctx(&[Permission::LakehouseRead]), "secrets", "tokens")
            .unwrap();
        assert!(snaps.is_empty());
    }

    #[test]
    fn namespace_totals_aggregates_per_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/lakehouse/src/components/Cards/NamespaceCards.tsx",
            "NamespaceCards",
            "acme"
        );
        let s = AdminState::seeded();
        let tot = namespace_totals(&s, &ctx(&[Permission::LakehouseRead])).unwrap();
        assert_eq!(tot.len(), 1);
        let (files, size) = tot.get("warehouse").unwrap();
        assert_eq!(*files, 4_320 + 1_120);
        assert_eq!(*size, 1_073_741_824 + 268_435_456);
    }

    #[test]
    fn append_snapshot_validates_op_and_table_existence() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/lakehouse/src/components/Snapshots/AppendDialog.tsx",
            "validateOp",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::LakehouseRead, Permission::LakehouseSnapshot]);
        // Bad op rejected.
        assert!(matches!(
            append_snapshot(&s, &c, "warehouse", "orders", 1010, 1_003_000, "Crunch", 0)
                .unwrap_err(),
            LakehouseViewError::InvalidOp
        ));
        // Missing table rejected.
        assert!(matches!(
            append_snapshot(&s, &c, "warehouse", "ghost", 1010, 1_003_000, "Append", 0)
                .unwrap_err(),
            LakehouseViewError::TableNotFound(_, _)
        ));
    }

    #[test]
    fn append_snapshot_advances_current_snapshot_and_file_count() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/lakehouse/src/components/Snapshots/AppendDialog.tsx",
            "AppendDialog",
            "acme"
        );
        let s = AdminState::seeded();
        let c = ctx(&[Permission::LakehouseRead, Permission::LakehouseSnapshot]);
        append_snapshot(&s, &c, "warehouse", "orders", 1010, 1_003_000, "Append", 8)
            .unwrap();
        let tables = list_tables(&s, &c).unwrap();
        let orders = tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(orders.current_snapshot_id, 1010);
        assert_eq!(orders.file_count, 4_320 + 8);
    }

    #[test]
    fn render_excludes_evil_namespace_totals() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/lakehouse/src/components/Tables/TablesPage.tsx",
            "TablesPage",
            "acme"
        );
        let s = AdminState::seeded();
        let html = render(&s, &ctx(&[Permission::LakehouseRead])).unwrap();
        assert!(html.contains("Tables (2)"));
        assert!(html.contains("warehouse"));
        // Check rendered cell text only — the sidebar links to
        // /admin/secrets (OpenBao), which legitimately injects the
        // substring `secrets` outside the data area.
        assert!(!html.contains(">secrets<"));
    }
}
