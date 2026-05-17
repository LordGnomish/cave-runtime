// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Partitions sub-page — partition-spec view.
//!
//! A partition spec is the operator-defined mapping from row columns to
//! partition values. Iceberg supports identity / year / month / day /
//! hour / bucket[N] / truncate[N] transforms. This page derives a
//! synthetic spec from each table's columns so the operator can see the
//! shape that ingest jobs are writing into.

use super::tables;
use super::types::{IcebergTable, IcebergViewError, PartitionField, PartitionSpec};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

pub fn list_all(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<PartitionSpec>, IcebergViewError> {
    ctx.authorise(Permission::IcebergRead)?;
    let tbls = tables::list(state, ctx)?;
    Ok(tbls.iter().map(derive_spec).collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<PartitionSpec, IcebergViewError> {
    let t = tables::get(state, ctx, namespace, name)?;
    Ok(derive_spec(&t))
}

fn derive_spec(t: &IcebergTable) -> PartitionSpec {
    // Deterministic fields keyed off namespace+name so two reads agree.
    let mut fields = Vec::new();
    let base_id = ((t.partition_spec_id as u32).saturating_mul(1000)) + 1000;
    fields.push(PartitionField {
        field_id: base_id,
        source_id: 1,
        name: "ts_day".into(),
        transform: "day".into(),
    });
    if t.namespace == "analytics" {
        fields.push(PartitionField {
            field_id: base_id + 1,
            source_id: 2,
            name: "tenant_bucket".into(),
            transform: "bucket[16]".into(),
        });
    }
    if t.row_count > 100_000 {
        fields.push(PartitionField {
            field_id: base_id + 2,
            source_id: 3,
            name: "region".into(),
            transform: "identity".into(),
        });
    }
    PartitionSpec {
        tenant: t.tenant.clone(),
        table_fqn: t.fqn(),
        spec_id: t.partition_spec_id,
        fields,
    }
}

/// True iff `spec` has at least one non-identity transform (i.e. the
/// rows are getting bucketed / hashed in a way that needs a planner).
pub fn has_complex_transform(spec: &PartitionSpec) -> bool {
    spec.fields
        .iter()
        .any(|f| f.transform != "identity" && !f.transform.is_empty())
}

pub fn unique_transforms(specs: &[PartitionSpec]) -> Vec<String> {
    let mut out: Vec<String> = specs
        .iter()
        .flat_map(|s| s.fields.iter().map(|f| f.transform.clone()))
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IcebergViewError> {
    let specs = list_all(state, ctx)?;
    let mut rows = Vec::new();
    for s in &specs {
        for f in &s.fields {
            rows.push(vec![
                escape(&s.table_fqn),
                s.spec_id.to_string(),
                f.field_id.to_string(),
                escape(&f.name),
                escape(&f.transform),
            ]);
        }
    }
    let body = format!(
        r#"<section>{tbl}</section>"#,
        tbl = table(
            &["table", "spec", "field_id", "name", "transform"],
            &rows
        )
    );
    Ok(page_shell_full(
        ctx,
        "/admin/iceberg/partitions",
        &format!("iceberg/partitions · {}", escape(ctx.tenant.as_str())),
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

    fn t(ns: &str, name: &str, rows: u64) -> IcebergTable {
        IcebergTable {
            tenant: TenantId::new("acme").expect("t"),
            namespace: ns.into(),
            name: name.into(),
            location: "s3://x".into(),
            format_version: 2,
            current_snapshot_id: Some(1),
            schema_id: 1,
            last_updated_ms: 0,
            row_count: rows,
            file_count: 1,
            total_data_files_bytes: 0,
            partition_spec_id: 1,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        s.iceberg_tables.write().unwrap().push(t("analytics", "orders", 500_000));
        s.iceberg_tables.write().unwrap().push(t("raw", "audit", 10));
        s
    }

    #[test]
    fn list_all_returns_one_spec_per_table() {
        let s = seeded();
        let specs = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(specs.len(), 2);
    }

    #[test]
    fn list_all_refuses_without_perm() {
        let s = seeded();
        assert!(list_all(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn analytics_namespace_has_bucket_field() {
        let s = seeded();
        let spec = get(&s, &ctx(&[Permission::IcebergRead]), "analytics", "orders").unwrap();
        assert!(spec.fields.iter().any(|f| f.transform.starts_with("bucket[")));
    }

    #[test]
    fn raw_namespace_skips_bucket_field() {
        let s = seeded();
        let spec = get(&s, &ctx(&[Permission::IcebergRead]), "raw", "audit").unwrap();
        assert!(!spec.fields.iter().any(|f| f.transform.starts_with("bucket[")));
    }

    #[test]
    fn large_table_gets_region_partition() {
        let s = seeded();
        let spec = get(&s, &ctx(&[Permission::IcebergRead]), "analytics", "orders").unwrap();
        assert!(spec.fields.iter().any(|f| f.name == "region"));
    }

    #[test]
    fn small_table_has_no_region_partition() {
        let s = seeded();
        let spec = get(&s, &ctx(&[Permission::IcebergRead]), "raw", "audit").unwrap();
        assert!(!spec.fields.iter().any(|f| f.name == "region"));
    }

    #[test]
    fn has_complex_transform_flags_bucket() {
        let s = seeded();
        let spec = get(&s, &ctx(&[Permission::IcebergRead]), "analytics", "orders").unwrap();
        assert!(has_complex_transform(&spec));
    }

    #[test]
    fn unique_transforms_dedups_across_specs() {
        let s = seeded();
        let specs = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let t = unique_transforms(&specs);
        // "day" must appear; "bucket[16]" + "identity" should not duplicate.
        assert!(t.contains(&"day".to_string()));
        let dup = t.iter().filter(|x| x.as_str() == "day").count();
        assert_eq!(dup, 1);
    }

    #[test]
    fn render_emits_transform_column() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert!(html.contains("transform"));
        assert!(html.contains("day"));
    }

    #[test]
    fn get_unknown_table_errors() {
        let s = seeded();
        assert!(matches!(
            get(&s, &ctx(&[Permission::IcebergRead]), "x", "y").unwrap_err(),
            IcebergViewError::TableNotFound(_)
        ));
    }
}
