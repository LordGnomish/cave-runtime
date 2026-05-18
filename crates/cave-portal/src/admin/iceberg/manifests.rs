// SPDX-License-Identifier: AGPL-3.0-or-later
//! Manifests sub-page — per-snapshot manifest file listing.

use super::snapshots;
use super::tables;
use super::types::{IcebergManifest, IcebergSnapshot, IcebergTable, IcebergViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

pub fn list_all(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<IcebergManifest>, IcebergViewError> {
    ctx.authorise(Permission::IcebergRead)?;
    let tbls = tables::list(state, ctx)?;
    let mut out = Vec::new();
    for t in tbls {
        let snaps = snapshots::derive_history_pub(&t);
        for sn in snaps {
            out.extend(derive_manifests(&t, &sn));
        }
    }
    Ok(out)
}

pub fn list_for_snapshot(
    state: &AdminState,
    ctx: &RequestCtx,
    snapshot_id: i64,
) -> Result<Vec<IcebergManifest>, IcebergViewError> {
    let all = list_all(state, ctx)?;
    let manifests: Vec<IcebergManifest> = all
        .into_iter()
        .filter(|m| m.snapshot_id == snapshot_id)
        .collect();
    if manifests.is_empty() {
        return Err(IcebergViewError::SnapshotNotFound(snapshot_id));
    }
    Ok(manifests)
}

/// One data manifest + one delete manifest per snapshot.
fn derive_manifests(t: &IcebergTable, sn: &IcebergSnapshot) -> Vec<IcebergManifest> {
    vec![
        IcebergManifest {
            tenant: t.tenant.clone(),
            table_fqn: t.fqn(),
            snapshot_id: sn.snapshot_id,
            manifest_path: format!("{}/data-{}.avro", t.location, sn.snapshot_id),
            content: "data".into(),
            partition_spec_id: t.partition_spec_id,
            added_files_count: sn.summary_added_files,
            existing_files_count: 0,
            deleted_files_count: 0,
            added_rows_count: sn.summary_added_records,
            existing_rows_count: 0,
            deleted_rows_count: 0,
            min_sequence_number: sn.sequence_number,
            manifest_length_bytes: 4096 + u64::from(sn.summary_added_files) * 512,
        },
        IcebergManifest {
            tenant: t.tenant.clone(),
            table_fqn: t.fqn(),
            snapshot_id: sn.snapshot_id,
            manifest_path: format!("{}/delete-{}.avro", t.location, sn.snapshot_id),
            content: "delete".into(),
            partition_spec_id: t.partition_spec_id,
            added_files_count: 0,
            existing_files_count: 0,
            deleted_files_count: if sn.summary_deleted_records > 0 { 1 } else { 0 },
            added_rows_count: 0,
            existing_rows_count: 0,
            deleted_rows_count: sn.summary_deleted_records,
            min_sequence_number: sn.sequence_number,
            manifest_length_bytes: 2048,
        },
    ]
}

pub fn total_bytes(rows: &[IcebergManifest]) -> u64 {
    rows.iter().map(|m| m.manifest_length_bytes).sum()
}

pub fn data_vs_delete_split(rows: &[IcebergManifest]) -> (usize, usize) {
    let data = rows.iter().filter(|m| m.content == "data").count();
    let delete = rows.iter().filter(|m| m.content == "delete").count();
    (data, delete)
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IcebergViewError> {
    let rows = list_all(state, ctx)?;
    let (data, delete) = data_vs_delete_split(&rows);
    let total_kb = total_bytes(&rows) / 1024;
    let body = format!(
        r#"<section><div class="mb-3 flex gap-3 text-sm">
  <span class="px-2 py-1 rounded bg-gray-200"><strong>{data}</strong> data</span>
  <span class="px-2 py-1 rounded bg-gray-200"><strong>{delete}</strong> delete</span>
  <span class="px-2 py-1 rounded bg-gray-200"><strong>{total_kb}</strong> KB</span>
</div>{tbl}</section>"#,
        data = data,
        delete = delete,
        total_kb = total_kb,
        tbl = table(
            &["table", "snapshot", "content", "added_files", "deleted_rows", "path"],
            &rows
                .iter()
                .map(|m| vec![
                    escape(&m.table_fqn),
                    m.snapshot_id.to_string(),
                    m.content.clone(),
                    m.added_files_count.to_string(),
                    m.deleted_rows_count.to_string(),
                    escape(&m.manifest_path),
                ])
                .collect::<Vec<_>>(),
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/iceberg/manifests",
        &format!("iceberg/manifests · {}", escape(ctx.tenant.as_str())),
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
            schema_id: 2,
            last_updated_ms: 1_730_000_000_000,
            row_count: 100,
            file_count: 5,
            total_data_files_bytes: 1024,
            partition_spec_id: 1,
        });
        s
    }

    #[test]
    fn list_all_refuses_without_permission() {
        let s = seeded();
        assert!(list_all(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_all_emits_two_per_snapshot() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        // 1 table × 3 snapshots × (data + delete) = 6 manifests.
        assert_eq!(rows.len(), 6);
    }

    #[test]
    fn list_for_snapshot_filters() {
        let s = seeded();
        let rows = list_for_snapshot(&s, &ctx(&[Permission::IcebergRead]), 7_000_001).unwrap();
        assert!(rows.iter().all(|r| r.snapshot_id == 7_000_001));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn list_for_unknown_snapshot_errors() {
        let s = seeded();
        assert!(matches!(
            list_for_snapshot(&s, &ctx(&[Permission::IcebergRead]), 999).unwrap_err(),
            IcebergViewError::SnapshotNotFound(_)
        ));
    }

    #[test]
    fn data_manifests_carry_added_files() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let data: Vec<&IcebergManifest> = rows.iter().filter(|m| m.content == "data").collect();
        assert!(data.iter().all(|m| m.added_files_count > 0));
        assert!(data.iter().all(|m| m.added_rows_count > 0));
    }

    #[test]
    fn delete_manifests_carry_deletes_only_when_op_was_overwrite() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let delete: Vec<&IcebergManifest> = rows.iter().filter(|m| m.content == "delete").collect();
        // Snapshot k=1 was overwrite → one delete manifest with deletes.
        assert!(delete.iter().any(|m| m.deleted_rows_count > 0));
    }

    #[test]
    fn manifest_paths_distinct_per_snapshot() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let mut paths: Vec<_> = rows.iter().map(|m| m.manifest_path.clone()).collect();
        let count_before = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), count_before, "duplicate manifest paths");
    }

    #[test]
    fn total_bytes_sums_lengths() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert!(total_bytes(&rows) > 0);
    }

    #[test]
    fn data_vs_delete_split_balances() {
        let s = seeded();
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let (d, x) = data_vs_delete_split(&rows);
        assert_eq!(d, 3);
        assert_eq!(x, 3);
    }

    #[test]
    fn render_includes_split_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert!(html.contains("data"));
        assert!(html.contains("delete"));
        assert!(html.contains("KB"));
    }
}
