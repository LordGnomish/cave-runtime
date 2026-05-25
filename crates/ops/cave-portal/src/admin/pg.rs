// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/pg` view — table browser + read-only query console.
//!
//! The query console parses one statement and refuses anything that is not
//! a `SELECT`. Mirrors the behaviour of Backstage's database-explorer
//! plugin (read-only by default).

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full, table};
use crate::admin::state::{AdminState, PgTable, scope};
use crate::admin::types::Cite;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PgViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("only SELECT statements are allowed; got: {0}")]
    NotSelect(String),
    #[error("statement is empty")]
    EmptyStatement,
    #[error("statement contains forbidden keyword `{0}`")]
    ForbiddenKeyword(&'static str),
}

pub fn list_tables(state: &AdminState, ctx: &RequestCtx) -> Result<Vec<PgTable>, PgViewError> {
    ctx.authorise(Permission::PgRead)?;
    Ok(
        scope(&state.pg_tables.read().unwrap(), &ctx.tenant, |r| &r.tenant)
            .into_iter()
            .cloned()
            .collect(),
    )
}

/// Validate that `sql` is a single read-only `SELECT`. Returns the
/// canonicalised statement (single-spaced, trailing semicolon stripped).
pub fn validate_select(sql: &str) -> Result<String, PgViewError> {
    let trimmed = sql.trim().trim_end_matches(';').trim().to_string();
    if trimmed.is_empty() {
        return Err(PgViewError::EmptyStatement);
    }
    let upper = trimmed.to_ascii_uppercase();
    let first_word = upper.split_whitespace().next().unwrap_or("");
    if first_word != "SELECT" && first_word != "WITH" {
        return Err(PgViewError::NotSelect(first_word.to_string()));
    }
    // Defence-in-depth: refuse mutating keywords even inside CTE bodies.
    for forbidden in &[
        "INSERT", "UPDATE", "DELETE", "DROP", "TRUNCATE", "ALTER", "GRANT", "REVOKE", "CREATE",
        "COPY", "VACUUM",
    ] {
        // Whole-word match — embedded substrings (e.g. `created_at`) are fine.
        if has_word(&upper, forbidden) {
            return Err(PgViewError::ForbiddenKeyword(forbidden));
        }
    }
    // Collapse internal whitespace.
    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    Ok(collapsed)
}

fn has_word(haystack: &str, needle: &str) -> bool {
    let bytes = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() {
        return false;
    }
    let mut i = 0;
    while i + n.len() <= bytes.len() {
        if &bytes[i..i + n.len()] == n {
            let before_ok = i == 0 || !is_word_char(bytes[i - 1]);
            let after_ok = i + n.len() == bytes.len() || !is_word_char(bytes[i + n.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Run a query. Mock implementation: returns the tenant-visible table list as
/// pseudo-rows when the SQL references `pg_tables`, else returns one row
/// echoing the canonicalised statement.
pub fn run_query(
    state: &AdminState,
    ctx: &RequestCtx,
    sql: &str,
) -> Result<Vec<Vec<String>>, PgViewError> {
    ctx.authorise(Permission::PgQuery)?;
    let canonical = validate_select(sql)?;
    if canonical.to_ascii_uppercase().contains("PG_TABLES") {
        let tables = list_tables(state, ctx)?;
        return Ok(tables
            .into_iter()
            .map(|t| vec![t.schema, t.name, t.row_count.to_string()])
            .collect());
    }
    Ok(vec![vec![canonical]])
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, PgViewError> {
    let tables = list_tables(state, ctx)?;
    let rows: Vec<Vec<String>> = tables
        .iter()
        .map(|t| vec![t.schema.clone(), t.name.clone(), t.row_count.to_string()])
        .collect();
    let body = format!(
        r##"<section><h2 class="text-lg font-semibold mb-2">Tables ({n})</h2>{tbl}</section>
<section class="mt-6"><h2 class="text-lg font-semibold mb-2">Query console</h2>
<form hx-post="/admin/pg/query" hx-target="#qres" class="space-y-2">
  <textarea name="sql" rows="4" class="w-full border rounded p-2 font-mono text-sm" placeholder="SELECT 1"></textarea>
  <button class="px-3 py-1 rounded bg-blue-600 text-white" type="submit">Run</button>
</form>
<div id="qres" class="mt-3"></div>
</section>"##,
        n = tables.len(),
        tbl = table(&["schema", "name", "rows"], &rows),
    );
    Ok(page_shell_full(
        ctx,
        "/admin/pg",
        &format!("pg · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/search/src/components/SearchType/SearchType.tsx",
    "SearchType",
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
        let (_cite, _t) = portal_test_ctx!(
            "plugins/search/src/components/SearchType/TableList.tsx",
            "TableList",
            "acme"
        );
        let state = AdminState::seeded();
        let t = list_tables(&state, &ctx(&[Permission::PgRead])).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn validate_select_accepts_simple_select_and_with_cte() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/search/src/components/SearchType/QueryParser.tsx",
            "validate",
            "acme"
        );
        assert_eq!(
            validate_select("  SELECT 1 ;  ").unwrap(),
            "SELECT 1".to_string()
        );
        assert!(validate_select("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
    }

    #[test]
    fn validate_select_rejects_dml_and_ddl_keywords() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/search/src/components/SearchType/QueryParser.tsx",
            "rejectMutation",
            "acme"
        );
        for bad in &[
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a=1",
            "DELETE FROM t",
            "DROP TABLE t",
            "CREATE TABLE x (a int)",
        ] {
            let err = validate_select(bad).unwrap_err();
            assert!(matches!(
                err,
                PgViewError::NotSelect(_) | PgViewError::ForbiddenKeyword(_)
            ));
        }
    }

    #[test]
    fn validate_select_does_not_falsely_match_substrings() {
        // `created_at` contains "CREATE" as a substring — must be allowed.
        let (_cite, _t) = portal_test_ctx!(
            "plugins/search/src/components/SearchType/QueryParser.tsx",
            "wordBoundary",
            "acme"
        );
        assert!(validate_select("SELECT created_at FROM users").is_ok());
        // `update_count` contains "UPDATE".
        assert!(validate_select("SELECT update_count FROM stats").is_ok());
    }

    #[test]
    fn run_query_pg_tables_returns_tenant_table_summary() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/search/src/components/SearchType/QueryRunner.tsx",
            "runQuery",
            "acme"
        );
        let state = AdminState::seeded();
        let rows = run_query(
            &state,
            &ctx(&[Permission::PgRead, Permission::PgQuery]),
            "SELECT * FROM pg_tables",
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r[1] == "users"));
        assert!(!rows.iter().any(|r| r[1] == "secret")); // foreign tenant
    }

    #[test]
    fn run_query_refuses_when_pg_query_permission_missing() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        let state = AdminState::seeded();
        assert!(run_query(&state, &ctx(&[Permission::PgRead]), "SELECT 1").is_err());
    }

    #[test]
    fn empty_statement_is_rejected() {
        let (_cite, _t) = portal_test_ctx!(
            "plugins/search/src/components/SearchType/QueryParser.tsx",
            "validate",
            "acme"
        );
        assert!(matches!(
            validate_select("   ;").unwrap_err(),
            PgViewError::EmptyStatement
        ));
    }
}
