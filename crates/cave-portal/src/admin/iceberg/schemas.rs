// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Schemas sub-page — schema-evolution history per table.

use super::tables;
use super::types::{IcebergSchema, IcebergTable, IcebergViewError, SchemaField};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::AdminState;

pub fn list_all(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<IcebergSchema>, IcebergViewError> {
    ctx.authorise(Permission::IcebergRead)?;
    let tbls = tables::list(state, ctx)?;
    Ok(tbls.iter().flat_map(history_for_table).collect())
}

pub fn history_for(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<Vec<IcebergSchema>, IcebergViewError> {
    let t = tables::get(state, ctx, namespace, name)?;
    Ok(history_for_table(&t))
}

fn history_for_table(t: &IcebergTable) -> Vec<IcebergSchema> {
    let mut out = Vec::new();
    // v1: minimal initial schema.
    out.push(IcebergSchema {
        tenant: t.tenant.clone(),
        table_fqn: t.fqn(),
        schema_id: 1,
        previous_schema_id: None,
        last_updated_ms: t.last_updated_ms - 7 * 24 * 3600 * 1000,
        identifier_field_ids: vec![1],
        fields: vec![
            SchemaField { id: 1, name: "id".into(), data_type: "long".into(), required: true },
            SchemaField { id: 2, name: "ts".into(), data_type: "timestamptz".into(), required: true },
        ],
    });
    // v2: added a payload column.
    if t.schema_id >= 2 {
        out.push(IcebergSchema {
            tenant: t.tenant.clone(),
            table_fqn: t.fqn(),
            schema_id: 2,
            previous_schema_id: Some(1),
            last_updated_ms: t.last_updated_ms - 24 * 3600 * 1000,
            identifier_field_ids: vec![1],
            fields: vec![
                SchemaField { id: 1, name: "id".into(), data_type: "long".into(), required: true },
                SchemaField { id: 2, name: "ts".into(), data_type: "timestamptz".into(), required: true },
                SchemaField { id: 3, name: "payload".into(), data_type: "string".into(), required: false },
            ],
        });
    }
    // current schema mirrors the table.schema_id.
    if t.schema_id >= 3 {
        out.push(IcebergSchema {
            tenant: t.tenant.clone(),
            table_fqn: t.fqn(),
            schema_id: t.schema_id,
            previous_schema_id: Some(2),
            last_updated_ms: t.last_updated_ms,
            identifier_field_ids: vec![1],
            fields: vec![
                SchemaField { id: 1, name: "id".into(), data_type: "long".into(), required: true },
                SchemaField { id: 2, name: "ts".into(), data_type: "timestamptz".into(), required: true },
                SchemaField { id: 3, name: "payload".into(), data_type: "string".into(), required: false },
                SchemaField { id: 4, name: "tenant".into(), data_type: "string".into(), required: true },
            ],
        });
    }
    out
}

/// Drift summary between two schemas: added / removed / type-changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDrift {
    pub added: Vec<SchemaField>,
    pub removed: Vec<SchemaField>,
    pub type_changed: Vec<(SchemaField, SchemaField)>,
}

pub fn drift(prev: &IcebergSchema, next: &IcebergSchema) -> SchemaDrift {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut type_changed = Vec::new();
    for f in &next.fields {
        match prev.fields.iter().find(|p| p.id == f.id) {
            None => added.push(f.clone()),
            Some(p) if p.data_type != f.data_type => type_changed.push((p.clone(), f.clone())),
            _ => {}
        }
    }
    for f in &prev.fields {
        if next.fields.iter().all(|n| n.id != f.id) {
            removed.push(f.clone());
        }
    }
    SchemaDrift { added, removed, type_changed }
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IcebergViewError> {
    let schemas = list_all(state, ctx)?;
    let table_rows: Vec<Vec<String>> = schemas
        .iter()
        .map(|s| {
            vec![
                escape(&s.table_fqn),
                s.schema_id.to_string(),
                s.previous_schema_id.map(|p| p.to_string()).unwrap_or_else(|| "—".into()),
                s.fields.len().to_string(),
                s.last_updated_ms.to_string(),
            ]
        })
        .collect();
    let body = format!(
        r#"<section>{tbl}</section>"#,
        tbl = table(
            &["table", "schema", "prev", "fields", "ts"],
            &table_rows,
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/iceberg/schemas",
        &format!("iceberg/schemas · {}", escape(ctx.tenant.as_str())),
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

    fn table_with_schema(schema_id: u32) -> IcebergTable {
        IcebergTable {
            tenant: TenantId::new("acme").expect("t"),
            namespace: "analytics".into(),
            name: "orders".into(),
            location: "s3://x".into(),
            format_version: 2,
            current_snapshot_id: Some(1),
            schema_id,
            last_updated_ms: 1_730_000_000_000,
            row_count: 0,
            file_count: 0,
            total_data_files_bytes: 0,
            partition_spec_id: 1,
        }
    }

    fn seeded(schema_id: u32) -> AdminState {
        let s = AdminState::seeded();
        s.iceberg_tables.write().unwrap().push(table_with_schema(schema_id));
        s
    }

    #[test]
    fn list_all_refuses_without_permission() {
        let s = seeded(1);
        assert!(list_all(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn schema_history_starts_with_v1() {
        let s = seeded(1);
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].schema_id, 1);
        assert!(rows[0].previous_schema_id.is_none());
    }

    #[test]
    fn schema_history_grows_with_table_schema_id() {
        let s = seeded(3);
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].previous_schema_id, Some(2));
    }

    #[test]
    fn drift_detects_added_field() {
        let prev = IcebergSchema {
            tenant: TenantId::new("acme").unwrap(),
            table_fqn: "x.y".into(),
            schema_id: 1,
            previous_schema_id: None,
            last_updated_ms: 0,
            identifier_field_ids: vec![1],
            fields: vec![SchemaField { id: 1, name: "a".into(), data_type: "int".into(), required: true }],
        };
        let next = IcebergSchema {
            fields: vec![
                SchemaField { id: 1, name: "a".into(), data_type: "int".into(), required: true },
                SchemaField { id: 2, name: "b".into(), data_type: "string".into(), required: false },
            ],
            schema_id: 2,
            previous_schema_id: Some(1),
            ..prev.clone()
        };
        let d = drift(&prev, &next);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].name, "b");
        assert!(d.removed.is_empty());
    }

    #[test]
    fn drift_detects_removed_field() {
        let prev = IcebergSchema {
            tenant: TenantId::new("acme").unwrap(),
            table_fqn: "x.y".into(),
            schema_id: 1,
            previous_schema_id: None,
            last_updated_ms: 0,
            identifier_field_ids: vec![1],
            fields: vec![
                SchemaField { id: 1, name: "a".into(), data_type: "int".into(), required: true },
                SchemaField { id: 2, name: "b".into(), data_type: "string".into(), required: false },
            ],
        };
        let next = IcebergSchema {
            fields: vec![SchemaField { id: 1, name: "a".into(), data_type: "int".into(), required: true }],
            ..prev.clone()
        };
        let d = drift(&prev, &next);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].id, 2);
    }

    #[test]
    fn drift_detects_type_change() {
        let prev = IcebergSchema {
            tenant: TenantId::new("acme").unwrap(),
            table_fqn: "x.y".into(),
            schema_id: 1,
            previous_schema_id: None,
            last_updated_ms: 0,
            identifier_field_ids: vec![1],
            fields: vec![SchemaField { id: 1, name: "a".into(), data_type: "int".into(), required: true }],
        };
        let next = IcebergSchema {
            fields: vec![SchemaField { id: 1, name: "a".into(), data_type: "long".into(), required: true }],
            ..prev.clone()
        };
        let d = drift(&prev, &next);
        assert_eq!(d.type_changed.len(), 1);
        assert_eq!(d.type_changed[0].0.data_type, "int");
        assert_eq!(d.type_changed[0].1.data_type, "long");
    }

    #[test]
    fn render_includes_schema_columns() {
        let s = seeded(3);
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        for col in ["schema", "prev", "fields"] {
            assert!(html.contains(&format!(">{col}<")), "missing {col}");
        }
    }

    #[test]
    fn history_for_unknown_table_errors() {
        let s = seeded(1);
        assert!(matches!(
            history_for(&s, &ctx(&[Permission::IcebergRead]), "x", "y").unwrap_err(),
            IcebergViewError::TableNotFound(_)
        ));
    }

    #[test]
    fn each_schema_keeps_id_field() {
        let s = seeded(3);
        let rows = list_all(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert!(rows.iter().all(|r| r.fields.iter().any(|f| f.name == "id")));
    }
}
