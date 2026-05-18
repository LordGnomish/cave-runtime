// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tables sub-page — catalog listing grouped by namespace.

use super::types::{IcebergTable, IcebergViewError};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{scope, AdminState};

pub fn list(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<IcebergTable>, IcebergViewError> {
    ctx.authorise(Permission::IcebergRead)?;
    let mut rows: Vec<IcebergTable> =
        scope(&state.iceberg_tables.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect();
    rows.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    Ok(rows)
}

pub fn list_namespace(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
) -> Result<Vec<IcebergTable>, IcebergViewError> {
    let all = list(state, ctx)?;
    Ok(all.into_iter().filter(|t| t.namespace == namespace).collect())
}

pub fn get(
    state: &AdminState,
    ctx: &RequestCtx,
    namespace: &str,
    name: &str,
) -> Result<IcebergTable, IcebergViewError> {
    list(state, ctx)?
        .into_iter()
        .find(|t| t.namespace == namespace && t.name == name)
        .ok_or_else(|| IcebergViewError::TableNotFound(format!("{namespace}.{name}")))
}

pub fn namespaces(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<String>, IcebergViewError> {
    let mut ns: Vec<String> = list(state, ctx)?.into_iter().map(|t| t.namespace).collect();
    ns.sort();
    ns.dedup();
    Ok(ns)
}

pub fn count_by_namespace(rows: &[IcebergTable]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        *acc.entry(r.namespace.clone()).or_insert(0) += 1;
    }
    acc.into_iter().collect()
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, IcebergViewError> {
    let rows = list(state, ctx)?;
    let by_ns = count_by_namespace(&rows);
    let chips: String = by_ns
        .iter()
        .map(|(ns, n)| {
            format!(
                r#"<span class="px-2 py-1 mr-2 rounded bg-gray-200 text-sm">{ns} <strong>×{n}</strong></span>"#,
                ns = escape(ns),
                n = n
            )
        })
        .collect();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|t| {
            vec![
                escape(&t.namespace),
                escape(&t.name),
                t.format_version.to_string(),
                t.current_snapshot_id
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "—".into()),
                t.row_count.to_string(),
                t.file_count.to_string(),
                escape(&t.location),
            ]
        })
        .collect();
    let body = format!(
        r#"<section><div class="mb-3 text-sm text-gray-600">Upstream: <a class="text-blue-700 underline" href="https://iceberg.apache.org/docs/latest/">Apache Iceberg</a></div><div class="mb-3">{chips}</div>{tbl}</section>"#,
        chips = chips,
        tbl = table(
            &[
                "namespace",
                "name",
                "format",
                "snapshot",
                "rows",
                "files",
                "location",
            ],
            &table_rows
        ),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/iceberg/tables",
        &format!("iceberg/tables · {}", escape(ctx.tenant.as_str())),
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

    fn t(tenant: &str, ns: &str, name: &str, rows: u64) -> IcebergTable {
        IcebergTable {
            tenant: TenantId::new(tenant).expect("tenant"),
            namespace: ns.into(),
            name: name.into(),
            location: format!("s3://lake/{ns}/{name}"),
            format_version: 2,
            current_snapshot_id: Some(1),
            schema_id: 1,
            last_updated_ms: 0,
            row_count: rows,
            file_count: 1,
            total_data_files_bytes: 1024,
            partition_spec_id: 1,
        }
    }

    fn seeded() -> AdminState {
        let s = AdminState::seeded();
        let mut g = s.iceberg_tables.write().unwrap();
        g.push(t("acme", "analytics", "orders", 100));
        g.push(t("acme", "analytics", "events", 200));
        g.push(t("acme", "raw", "audit", 50));
        g.push(t("evil", "analytics", "secret", 999));
        drop(g);
        s
    }

    #[test]
    fn list_filters_to_caller_tenant() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_sorts_by_namespace_then_name() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        // analytics.events, analytics.orders, raw.audit
        let labels: Vec<String> = rows.iter().map(|t| t.fqn()).collect();
        assert_eq!(
            labels,
            vec!["analytics.events", "analytics.orders", "raw.audit"]
        );
    }

    #[test]
    fn list_refuses_without_permission() {
        let s = seeded();
        assert!(list(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_namespace_filters() {
        let s = seeded();
        let rows = list_namespace(&s, &ctx(&[Permission::IcebergRead]), "raw").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "audit");
    }

    #[test]
    fn get_returns_table_or_error() {
        let s = seeded();
        let c = ctx(&[Permission::IcebergRead]);
        assert!(get(&s, &c, "analytics", "orders").is_ok());
        let err = get(&s, &c, "analytics", "missing").unwrap_err();
        assert!(matches!(err, IcebergViewError::TableNotFound(_)));
    }

    #[test]
    fn namespaces_deduped_and_sorted() {
        let s = seeded();
        let ns = namespaces(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        assert_eq!(ns, vec!["analytics".to_string(), "raw".into()]);
    }

    #[test]
    fn count_by_namespace_summary() {
        let s = seeded();
        let rows = list(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        let counts = count_by_namespace(&rows);
        assert_eq!(counts, vec![("analytics".into(), 2), ("raw".into(), 1)]);
    }

    #[test]
    fn render_includes_namespace_chips_and_columns() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        for col in ["namespace", "name", "format", "snapshot", "rows", "files"] {
            assert!(html.contains(&format!(">{col}<")), "missing column: {col}");
        }
        assert!(html.contains("analytics"));
        assert!(html.contains("raw"));
    }

    #[test]
    fn render_excludes_evil_rows() {
        let s = seeded();
        let html = render(&s, &ctx(&[Permission::IcebergRead])).unwrap();
        // Check rendered cell text only — the sidebar links to
        // /admin/secrets (OpenBao), which legitimately injects the
        // substring `secret` outside the data area.
        assert!(!html.contains(">secret"), "leaked foreign tenant row");
    }
}
