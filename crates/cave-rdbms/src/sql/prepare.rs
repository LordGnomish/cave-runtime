// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: postgres/postgres@c372fbbd8e911f2412b80a8c39d7079366565d67 src/backend/commands/prepare.c
//! `PREPARE` / `EXECUTE` / `DEALLOCATE` — server-side named prepared statements.
//!
//! Mirrors `src/backend/commands/prepare.c` and the `prepared_queries` HTAB
//! that lives in `src/backend/utils/cache/prepare.c`. The substitutable
//! parameters are positional (`$1`, `$2`, …); execution rebinds them into
//! the parse-tree before re-planning. Cave-rdbms keeps the same surface
//! but stores the rebound SQL string as the parameter substitution result.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// One server-side prepared statement entry.
///
/// PG stores: name, parse-tree, parameter type OIDs, planned plan-list,
/// `query_string`, `creation_time`. We keep the user-relevant subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStatement {
    pub name: String,
    /// The raw SQL with `$N` placeholders (after `PREPARE name AS …`).
    pub query: String,
    /// Declared parameter type names, in `$1, $2, …` order.
    pub param_types: Vec<String>,
}

impl PreparedStatement {
    /// Returns the maximum placeholder index referenced in `query`
    /// (e.g. `SELECT $1, $3` → 3). Used by `EXECUTE` to validate that
    /// the caller supplied enough parameters.
    pub fn max_param_index(&self) -> usize {
        let bytes = self.query.as_bytes();
        let mut max = 0usize;
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                let mut n: usize = 0;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    n = n * 10 + (bytes[j] - b'0') as usize;
                    j += 1;
                }
                if n > max {
                    max = n;
                }
                i = j;
            } else {
                i += 1;
            }
        }
        max
    }

    /// Substitutes positional parameters into the SQL text. Mirrors
    /// `EvaluateParams` in `prepare.c` but keeps the result as text
    /// rather than a bound parse-tree (cave-rdbms re-parses).
    /// Parameters are quoted: integers/booleans bare, everything else
    /// single-quoted with embedded `'` doubled.
    pub fn substitute(&self, params: &[String]) -> Result<String, PrepareError> {
        let needed = self.max_param_index();
        if params.len() < needed {
            return Err(PrepareError::WrongNumberOfArgs {
                statement: self.name.clone(),
                expected: needed,
                got: params.len(),
            });
        }
        let mut out = String::with_capacity(self.query.len() + 16);
        let bytes = self.query.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                let mut n: usize = 0;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    n = n * 10 + (bytes[j] - b'0') as usize;
                    j += 1;
                }
                let raw = &params[n - 1];
                out.push_str(&quote_literal(raw));
                i = j;
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        Ok(out)
    }
}

/// Apply PG's literal quoting rules (`quote_literal()` in `quote.c`):
/// integers/booleans stay bare, everything else gets single-quoted with
/// embedded `'` doubled.
fn quote_literal(raw: &str) -> String {
    // Boolean / integer literals — bare.
    let trimmed = raw.trim();
    if trimmed == "true" || trimmed == "false" || trimmed == "TRUE" || trimmed == "FALSE" {
        return trimmed.to_string();
    }
    if !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
        && trimmed.chars().any(|c| c.is_ascii_digit())
    {
        return trimmed.to_string();
    }
    let escaped = raw.replace('\'', "''");
    format!("'{}'", escaped)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrepareError {
    /// `PREPARE name AS …` when `name` already exists. PG raises
    /// `ERRCODE_DUPLICATE_PSTATEMENT`.
    #[error("prepared statement {0:?} already exists")]
    AlreadyExists(String),
    /// `EXECUTE` / `DEALLOCATE` on a name we have no record of.
    /// PG: `ERRCODE_UNDEFINED_PSTATEMENT`.
    #[error("prepared statement {0:?} does not exist")]
    NotFound(String),
    /// `EXECUTE foo(…)` with mismatching arg count.
    /// PG: `ERRCODE_PROTOCOL_VIOLATION` "wrong number of parameters".
    #[error("EXECUTE {statement} expects {expected} parameters, got {got}")]
    WrongNumberOfArgs {
        statement: String,
        expected: usize,
        got: usize,
    },
}

/// Per-session cache. PG stores this in process-private memory keyed by
/// the user's session; here we put it behind a `Mutex` so the cache is
/// safe to share across the async wire-protocol handler.
#[derive(Default)]
pub struct PreparedStatementCache {
    inner: Mutex<HashMap<String, PreparedStatement>>,
}

impl PreparedStatementCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, PreparedStatement>> {
        self.inner.lock().expect("PreparedStatementCache mutex poisoned")
    }

    /// `PREPARE name (typelist) AS …` — refuses to clobber an
    /// existing entry, matching `StorePreparedStatement(..., false)`.
    pub fn prepare(
        &self,
        name: impl Into<String>,
        query: impl Into<String>,
        param_types: Vec<String>,
    ) -> Result<(), PrepareError> {
        let name = name.into();
        let mut map = self.lock();
        if map.contains_key(&name) {
            return Err(PrepareError::AlreadyExists(name));
        }
        map.insert(
            name.clone(),
            PreparedStatement {
                name,
                query: query.into(),
                param_types,
            },
        );
        Ok(())
    }

    /// `EXECUTE name (args…)` — returns the substituted SQL ready for
    /// re-planning. Caller plans + executes it like a normal statement.
    pub fn execute(&self, name: &str, args: &[String]) -> Result<String, PrepareError> {
        let map = self.lock();
        let stmt = map.get(name).ok_or_else(|| PrepareError::NotFound(name.to_string()))?;
        stmt.substitute(args)
    }

    /// `DEALLOCATE name` — drops the entry. Refuses to free unknown name.
    pub fn deallocate(&self, name: &str) -> Result<(), PrepareError> {
        let mut map = self.lock();
        if map.remove(name).is_none() {
            return Err(PrepareError::NotFound(name.to_string()));
        }
        Ok(())
    }

    /// `DEALLOCATE ALL` — wipes the cache (PG `DropAllPreparedStatements`).
    pub fn deallocate_all(&self) {
        self.lock().clear();
    }

    /// Read-only inspection — the source of the `pg_prepared_statements`
    /// view. Sorted by name for stable rendering.
    pub fn list(&self) -> Vec<PreparedStatement> {
        let mut v: Vec<_> = self.lock().values().cloned().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> PreparedStatementCache {
        PreparedStatementCache::new()
    }

    #[test]
    fn test_prepare_stores_named_statement() {
        let c = cache();
        c.prepare("p1", "SELECT $1", vec!["int4".into()]).unwrap();
        assert_eq!(c.len(), 1);
        let ps = &c.list()[0];
        assert_eq!(ps.name, "p1");
        assert_eq!(ps.query, "SELECT $1");
        assert_eq!(ps.param_types, vec!["int4"]);
    }

    #[test]
    fn test_prepare_rejects_duplicate() {
        let c = cache();
        c.prepare("p1", "SELECT $1", vec!["int4".into()]).unwrap();
        let err = c
            .prepare("p1", "SELECT $1 + $1", vec!["int4".into()])
            .unwrap_err();
        assert_eq!(err, PrepareError::AlreadyExists("p1".into()));
    }

    #[test]
    fn test_max_param_index_finds_highest_dollar_n() {
        let ps = PreparedStatement {
            name: "x".into(),
            query: "SELECT $1, $3, $2 FROM t WHERE id = $3".into(),
            param_types: vec!["int4".into(), "text".into(), "int4".into()],
        };
        assert_eq!(ps.max_param_index(), 3);
    }

    #[test]
    fn test_execute_substitutes_integers_bare() {
        let c = cache();
        c.prepare("ins", "INSERT INTO t VALUES ($1, $2)", vec!["int4".into(), "int4".into()])
            .unwrap();
        let sql = c.execute("ins", &["42".into(), "-7".into()]).unwrap();
        assert_eq!(sql, "INSERT INTO t VALUES (42, -7)");
    }

    #[test]
    fn test_execute_substitutes_text_with_single_quotes_and_escapes() {
        let c = cache();
        c.prepare("ins", "INSERT INTO t VALUES ($1)", vec!["text".into()]).unwrap();
        // Embedded single-quote → doubled.
        let sql = c.execute("ins", &["O'Reilly".into()]).unwrap();
        assert_eq!(sql, "INSERT INTO t VALUES ('O''Reilly')");
    }

    #[test]
    fn test_execute_substitutes_booleans_bare() {
        let c = cache();
        c.prepare("u", "UPDATE t SET active = $1", vec!["bool".into()]).unwrap();
        assert_eq!(
            c.execute("u", &["true".into()]).unwrap(),
            "UPDATE t SET active = true"
        );
        assert_eq!(
            c.execute("u", &["FALSE".into()]).unwrap(),
            "UPDATE t SET active = FALSE"
        );
    }

    #[test]
    fn test_execute_wrong_arg_count_errors() {
        let c = cache();
        c.prepare("p", "SELECT $1, $2", vec!["int4".into(), "int4".into()])
            .unwrap();
        let err = c.execute("p", &["1".into()]).unwrap_err();
        assert!(matches!(
            err,
            PrepareError::WrongNumberOfArgs {
                expected: 2,
                got: 1,
                ..
            }
        ));
    }

    #[test]
    fn test_execute_unknown_name_errors() {
        let c = cache();
        let err = c.execute("nope", &[]).unwrap_err();
        assert_eq!(err, PrepareError::NotFound("nope".into()));
    }

    #[test]
    fn test_deallocate_drops_entry() {
        let c = cache();
        c.prepare("p", "SELECT 1", vec![]).unwrap();
        c.deallocate("p").unwrap();
        assert!(c.is_empty());
        assert_eq!(c.deallocate("p").unwrap_err(), PrepareError::NotFound("p".into()));
    }

    #[test]
    fn test_deallocate_all_wipes_cache() {
        let c = cache();
        c.prepare("a", "SELECT 1", vec![]).unwrap();
        c.prepare("b", "SELECT 2", vec![]).unwrap();
        c.deallocate_all();
        assert!(c.is_empty());
    }

    #[test]
    fn test_list_sorted_by_name() {
        let c = cache();
        c.prepare("zeta", "SELECT 1", vec![]).unwrap();
        c.prepare("alpha", "SELECT 2", vec![]).unwrap();
        c.prepare("mid", "SELECT 3", vec![]).unwrap();
        let names: Vec<String> = c.list().into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn test_quote_literal_handles_numeric_with_decimal() {
        let c = cache();
        c.prepare("p", "SELECT $1", vec!["numeric".into()]).unwrap();
        assert_eq!(c.execute("p", &["3.14".into()]).unwrap(), "SELECT 3.14");
    }

    #[test]
    fn test_quote_literal_quotes_alphabetic_strings() {
        let c = cache();
        c.prepare("p", "SELECT $1", vec!["text".into()]).unwrap();
        assert_eq!(c.execute("p", &["hello".into()]).unwrap(), "SELECT 'hello'");
    }
}
