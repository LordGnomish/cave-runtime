//! Snapshots sub-page — per-table snapshot timeline.
//!
//! Iceberg snapshots form a parent-pointer chain so the operator can
//! roll back / time-travel between any two committed states. This view
//! renders that chain newest-first with the summary stats from each
//! commit (`added_records`, `added_files`, `deleted_records`).

use super::tables;
use super::types::{IcebergSnapshot, IcebergTable, IcebergViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;

/// Derive a deterministic snapshot history for every table in the
/// tenant. The synthetic timeline mirrors what an append-only ingest
/// pipeline produces: one parent-chain per table, three snapshots
/// per parent.
pub fn list_all(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<IcebergSnapshot>, IcebergViewError> {
    ctx.authorise(Permission::IcebergRead)?;
    let tbls = tables::list(state, ctx)?;
    Ok(tbls.iter().flat_map(|t| derive_history(t)).collect())
}

pub fn list_for_table(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<Vec<IcebergSnapshot>, IcebergViewError> {
    let t = tables::get(state, ctx, namespace, name)?;
    Ok(derive_history(&t))
}

/// Public alias for the derive_history helper — used by the manifests
/// sub-page so it can fan out from each table's snapshot chain.
pub fn derive_history_pub(t: &IcebergTable) -> Vec<IcebergSnapshot> {
    derive_history(t)
}

/// Build a deterministic 3-snapshot history newest-first. Snapshot IDs
/// are `current_snapshot_id − k` so they're stable across reads.
fn derive_history(t: &IcebergTable) -> Vec<IcebergSnapshot> {
    let head = t.current_snapshot_id.unwrap_or(1);
    (0..3)
        .map(|k| {
            let id = head - i64::from(k);
            IcebergSnapshot {
                tenant: t.tenant.clone(),
                table_fqn: t.fqn(),
                snapshot_id: id,
                parent_snapshot_id: if k < 2 { Some(id - 1) } else { None },
                sequence_number: u64::try_from(3 - k).unwrap_or(0),
                timestamp_ms: t.last_updated_ms - i64::from(k) * 60_000,
                operation: if k == 0 { "append".into() } else if k == 1 { "overwrite".into() } else { "append".into() },
                manifest_list: format!("s3://lake/{}/snap-{}.avro", t.fqn(), id),
                summary_added_records: if k < 2 { 10_000 } else { 50_000 },
                summary_added_files: if k < 2 { 2 } else { 5 },
                summary_deleted_records: if k == 1 { 500 } else { 0 },
            }
        })
        .collect()
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    snapshot_id: i64,
) -> Result<IcebergSnapshot, IcebergViewError> {
    list_all(state, ctx)?
        .into_iter()
        .find(|s| s.snapshot_id == snapshot_id)
        .ok_or(IcebergViewError::SnapshotNotFound(snapshot_id))
}

/// Sum of `added_records` across a snapshot chain.
pub fn total_added_records(rows: &[IcebergSnapshot]) -> u64 {
    rows.iter().map(|s| s.summary_added_records).sum()
}

/// Operation histogram for the "what's been happening" chip row.
pub fn op_histogram(rows: &[IcebergSnapshot]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.operation.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IcebergViewError> {
    let rows = list_all(state, ctx)?;
    let hist = op_histogram(&rows);
    let chips: String = hist
        .iter()
        .map(|(op, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{op} <strong>×{n}</strong></span>"#,
                op = escape(op),
                n = n
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|s| {
            vec![
                escape(&s.table_fqn),
                s.snapshot_id.to_string(),
                s.parent_snapshot_id.map(|p| p.to_string()).unwrap_or_else(|| "—".into()),
                s.operation.clone(),
                s.summary_added_records.to_string(),
                s.summary_deleted_records.to_string(),
                s.timestamp_ms.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3">{chips}</div>{tbl}</section>"#,
        chips = chips,
        tbl = table(
            &[
                "table", "snapshot", "parent", "op", "added", "deleted", "ts",
            ],
            &table_rows,
        ),
    );
    Ok(page_shell(
        &format!("iceberg/snapshots · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::iceberg::types::IcebergTable;
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        s.iceberg_tables.write().unwrap().push(IcebergTable {
            tenant: TenantId::new("acme").expect("t"),
            namespace: "analytics".into(),
            name: "orders".into(),
            location: "s3://lake/analytics/orders".into(),
            format_version: 2,
            current_snapshot_id: Some(7_000_001),
            schema_id: 1,
            last_updated_ms: 1_730_000_000_000,
            row_count: 100,
            file_count: 5,
            total_data_files_bytes: 1024,
            partition_spec_id: 1,
        });
        s
    }

    #[test]
    fn list_all_emits_three_per_table() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn list_all_refuses_without_perm() {
        let s = seeded();
        assert!(list_all(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_for_table_filters_to_table() {
        let s = seeded();
        let rows = list_for_table(&s, &ctx(&[Permission::IcebergRead]), "analytics", "orders").unwrap();
        assert!(rows.iter().all(|r| r.table_fqn == "analytics.orders"));
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn list_for_unknown_table_errors() {
        let s = seeded();
        assert!(matches!(
            list_for_table(&s, &ctx(&[Permission::IcebergRead]), "x", "y").unwrap_err(),
            IcebergViewError::TableNotFound(_)
        ));
    }

    #[test]
    fn get_returns_snapshot_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::IcebergRead]);
        let head = get(&s, &c, 7_000_001).unwrap();
        assert_eq!(head.operation, "append");
        assert!(matches!(
            get(&s, &c, 999).unwrap_err(),
            IcebergViewError::SnapshotNotFound(999)
        ));
    }

    #[test]
    fn head_snapshot_has_no_grandparent_chain_break() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        // Newest first → first has parent, last has no parent (root).
        assert!(rows[0].parent_snapshot_id.is_some());
        assert!(rows[2].parent_snapshot_id.is_none());
    }

    #[test]
    fn op_histogram_counts_operations() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let hist = op_histogram(&rows);
        let append = hist.iter().find(|(op, _)| op == "append").map(|(_, n)| *n).unwrap_or(0);
        let over = hist.iter().find(|(op, _)| op == "overwrite").map(|(_, n)| *n).unwrap_or(0);
        assert_eq!(append + over, 3);
    }

    #[test]
    fn total_added_records_sums_chain() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert!(total_added_records(&rows) > 0);
    }

    #[test]
    fn render_includes_op_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        for col in ["snapshot", "parent", "op", "added", "deleted"] {
            assert!(html.contains(&format!(">{col}<")), "missing column {col}");
        }
        assert!(html.contains("append"));
    }
}
