// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/iceberg` — Apache Iceberg catalog views.
//!
//! Mirrors the apache/iceberg v1.5.x REST-catalog and Java-API surface:
//!
//! * [`tables`]      — namespace → table catalog list
//! * [`snapshots`]   — per-table snapshot timeline w/ summary stats
//! * [`partitions`]  — partition-spec view (fields + transforms)
//! * [`schemas`]     — schema-evolution history per table
//! * [`manifests`]   — manifest-file listing (data + delete files)
//!
//! Upstream UI: <https://iceberg.apache.org/docs/latest/>
//!
//! The pages read from in-memory fixtures on [`crate::admin::state::AdminState`].
//! Real-runtime wiring goes through the Iceberg REST API (`/v1/{prefix}/...`)
//! which lives outside this module.

pub mod tables;
pub mod snapshots;
pub mod partitions;
pub mod schemas;
pub mod manifests;
pub mod types;

pub use types::{
    IcebergManifest, IcebergSchema, IcebergSnapshot, IcebergTable, IcebergViewError,
    PartitionField, PartitionSpec, SchemaField,
};

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell, table};
use crate::admin::state::AdminState;

/// Top-of-page summary: total tables, total snapshots, total bytes.
pub fn summary(state: &AdminState, ctx: &RequestCtx) -> Result<Summary, IcebergViewError> {
    let tbls = tables::list(state, ctx)?;
    let snaps = snapshots::list_all(state, ctx)?;
    let total_bytes: u64 = tbls.iter().map(|t| t.total_data_files_bytes).sum();
    Ok(Summary {
        table_count: tbls.len(),
        snapshot_count: snaps.len(),
        total_data_bytes: total_bytes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub table_count: usize,
    pub snapshot_count: usize,
    pub total_data_bytes: u64,
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IcebergViewError> {
    ctx.authorise(Permission::IcebergRead)?;
    let s = summary(state, ctx)?;
    let body = format!(
        r#"<section class="grid grid-cols-3 gap-3 mb-4">
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">tables</div><div class="text-2xl font-bold">{tc}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">snapshots</div><div class="text-2xl font-bold">{sc}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">data size</div><div class="text-2xl font-bold">{mb} MB</div></div>
</section>
<nav class="flex gap-4 mb-3 text-sm">
  <a class="text-blue-700 underline" href="/admin/iceberg/tables?tenant_id={tid}">tables</a>
  <a class="text-blue-700 underline" href="/admin/iceberg/snapshots?tenant_id={tid}">snapshots</a>
  <a class="text-blue-700 underline" href="/admin/iceberg/partitions?tenant_id={tid}">partitions</a>
  <a class="text-blue-700 underline" href="/admin/iceberg/schemas?tenant_id={tid}">schemas</a>
  <a class="text-blue-700 underline" href="/admin/iceberg/manifests?tenant_id={tid}">manifests</a>
</nav>"#,
        tc = s.table_count,
        sc = s.snapshot_count,
        mb = s.total_data_bytes / 1_000_000,
        tid = escape(ctx.tenant.as_str()),
    );
    let table_body = {
        let rows = tables::list(state, ctx)?;
        let table_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|t| {
                vec![
                    t.namespace.clone(),
                    t.name.clone(),
                    t.format_version.to_string(),
                    t.row_count.to_string(),
                    t.file_count.to_string(),
                ]
            })
            .collect();
        table(
            &["namespace", "name", "format", "rows", "files"],
            &table_rows,
        )
    };
    Ok(page_shell(
        &format!("iceberg · {}", escape(ctx.tenant.as_str())),
        &format!("{body}<section>{table_body}</section>"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::iceberg::types::IcebergTable;
    use crate::admin::permission::{Permission, RequestCtx};
    use crate::admin::types::TenantId;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    fn seeded_state() -> AdminState {
        let s = AdminState::seeded();
        let acme = TenantId::new("acme").expect("tenant");
        s.iceberg_tables.write().unwrap().push(IcebergTable {
            tenant: acme.clone(),
            namespace: "analytics".into(),
            name: "orders".into(),
            location: "s3://lake/analytics/orders".into(),
            format_version: 2,
            current_snapshot_id: Some(7_000_001),
            schema_id: 3,
            last_updated_ms: 1_730_000_000_000,
            row_count: 1_000_000,
            file_count: 12,
            total_data_files_bytes: 5_000_000_000,
            partition_spec_id: 1,
        });
        s
    }

    #[test]
    fn summary_counts_tables_and_snapshots() {
        let s = seeded_state();
        let sum = summary(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(sum.table_count, 1);
        assert_eq!(sum.total_data_bytes, 5_000_000_000);
    }

    #[test]
    fn summary_refuses_without_permission() {
        let s = seeded_state();
        assert!(summary(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_links_and_metrics() {
        let s = seeded_state();
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert!(html.contains("iceberg/tables?tenant_id=acme"));
        assert!(html.contains(">1<"), "expected table count cell");
        assert!(html.contains("MB"));
    }
}
