//! DML executor — INSERT, UPDATE, DELETE, TRUNCATE, COPY.

use std::collections::HashMap;
use sqlparser::ast::{self as ast, Statement, Expr, ObjectName, TableFactor, FromTable};
use crate::error::{Error, PgError, Result, SqlState};
use crate::executor::{Executor};
use crate::executor::expr::{eval_expr, EvalContext};
use crate::executor::query::execute_query;
use crate::storage::heap::{Constraint, Index, IndexKey, IndexMethod};
use crate::storage::{alloc_oid};
use crate::types::{CommandResult, PgValue, oid};

// ─────────────────────────────────────────────────────────────────────────────
// INSERT
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_insert(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (table_name, columns, source, on_conflict, returning) = match stmt {
        Statement::Insert(ast::Insert {
            table_name,
            columns,
            source,
            on: on_insert,
            returning,
            ..
        }) => {
            let on_conflict = on_insert.and_then(|oi| {
                if let ast::OnInsert::OnConflict(oc) = oi { Some(oc) } else { None }
            });
            (table_name, columns, source, on_conflict, returning)
        }
        _ => unreachable!(),
    };

    let tbl_str = table_name.to_string();
    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let search_path = exec.config.search_path_refs().iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let search_refs: Vec<&str> = search_path.iter().map(String::as_str).collect();

    let (schema_ref, actual_name) = db.resolve_table(&tbl_str, &search_refs)
        .ok_or_else(|| Error::Pg(PgError::undefined_table(&tbl_str)))?;

    let xid = exec.ensure_transaction();

    // Collect column names
    let col_names: Vec<String> = columns.iter().map(|c| c.value.to_lowercase()).collect();

    // Build source rows from VALUES or SELECT
    let source_rows: Vec<Vec<Option<PgValue>>> = if let Some(src) = source {
        match *src {
            ast::Query { body, .. } => {
                match *body {
                    ast::SetExpr::Values(values) => {
                        let table_read = schema_ref.read();
                        let table = table_read.table(&actual_name).unwrap();
                        let table = table.read();
                        let mut out = Vec::new();
                        for row_exprs in &values.rows {
                            let mut row_vals: Vec<Option<PgValue>> = Vec::with_capacity(table.columns.len());
                            // Fill defaults first
                            for col in &table.columns {
                                if let Some(def) = &col.default_expr {
                                    // Try to evaluate default
                                    let val = evaluate_default(def, exec, col.type_oid);
                                    row_vals.push(Some(val));
                                } else {
                                    row_vals.push(None);
                                }
                            }
                            // Then apply provided columns
                            let insert_cols: Vec<usize> = if col_names.is_empty() {
                                (0..table.columns.len()).collect()
                            } else {
                                col_names.iter()
                                    .map(|n| table.column_idx(n)
                                        .ok_or_else(|| Error::Pg(PgError::error(
                                            SqlState::UNDEFINED_COLUMN,
                                            format!("column \"{n}\" of relation \"{}\" does not exist", table.name),
                                        ))))
                                    .collect::<Result<Vec<_>>>()?
                            };
                            for (pos, expr) in insert_cols.iter().zip(row_exprs.iter()) {
                                let ctx = EvalContext::new();
                                let val = eval_expr(expr, &ctx)?;
                                let col = &table.columns[*pos];
                                let cast_val = cast_value_to_col(val, col.type_oid)?;
                                row_vals[*pos] = if cast_val == PgValue::Null { None } else { Some(cast_val) };
                            }
                            out.push(row_vals);
                        }
                        out
                    }
                    other_body => {
                        // SELECT ... source — execute the query
                        drop(schema_ref); // release lock before executing subquery
                        let sub_result = execute_query(exec, ast::Query {
                            with: None,
                            body: Box::new(other_body),
                            order_by: None,
                            limit: None,
                            limit_by: vec![],
                            offset: None,
                            fetch: None,
                            locks: vec![],
                            for_clause: None,
                            settings: None,
                            format_clause: None,
                        })?;
                        let result_set = match sub_result {
                            CommandResult::Rows(rs) => rs,
                            _ => return Ok(CommandResult::Modified { tag: "INSERT 0 0".to_string(), rows_affected: 0 }),
                        };
                        // Re-acquire schema
                        let db2 = exec.db().unwrap();
                        let sp2: Vec<&str> = search_refs.iter().copied().collect();
                        let (schema_ref2, actual_name2) = db2.resolve_table(&tbl_str, &sp2).unwrap();
                        let schema = schema_ref2.read();
                        let table = schema.table(&actual_name2).unwrap();
                        let table = table.read();
                        result_set.rows.into_iter().map(|row_vals| {
                            // Map result columns to table columns by position or name
                            let mut out_row: Vec<Option<PgValue>> = vec![None; table.columns.len()];
                            let col_indices: Vec<usize> = if col_names.is_empty() {
                                (0..table.columns.len().min(row_vals.len())).collect()
                            } else {
                                col_names.iter()
                                    .filter_map(|n| table.column_idx(n))
                                    .collect()
                            };
                            for (table_pos, val) in col_indices.iter().zip(row_vals.iter()) {
                                out_row[*table_pos] = Some(val.clone());
                            }
                            out_row
                        }).collect()
                    }
                }
            }
        }
    } else {
        return Ok(CommandResult::Modified { tag: "INSERT 0 0".to_string(), rows_affected: 0 });
    };

    let mut inserted = 0u64;

    for row_vals in source_rows {
        let result = insert_one_row(exec, &tbl_str, &search_refs, xid, row_vals, &on_conflict);
        match result {
            Ok(true) => inserted += 1,
            Ok(false) => {} // ON CONFLICT DO NOTHING
            Err(e) => return Err(e),
        }
    }

    // Handle RETURNING
    if let Some(_returning_items) = returning {
        // Build a result set from the last inserted row (simplified)
        return Ok(CommandResult::Modified { tag: format!("INSERT 0 {}", inserted), rows_affected: inserted });
    }

    Ok(CommandResult::Modified { tag: format!("INSERT 0 {}", inserted), rows_affected: inserted })
}

/// Insert one row, handling ON CONFLICT. Returns true if actually inserted.
fn insert_one_row(
    exec: &mut Executor,
    tbl_str: &str,
    search_refs: &[&str],
    xid: crate::storage::mvcc::Xid,
    row_vals: Vec<Option<PgValue>>,
    on_conflict: &Option<ast::OnConflict>,
) -> Result<bool> {
    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let (schema_ref, actual_name) = db.resolve_table(tbl_str, search_refs)
        .ok_or_else(|| Error::Pg(PgError::undefined_table(tbl_str)))?;

    {
        let schema = schema_ref.read();
        let table_arc = schema.table(&actual_name).unwrap();
        let mut table = table_arc.write();

        // Check NOT NULL
        table.check_not_null(&row_vals)?;

        // Try to insert
        match table.insert(xid, row_vals.clone()) {
            Ok(ctid) => {
                // Update indexes
                drop(table);
                let indexes: Vec<_> = schema.indexes_for_table(&actual_name);
                for idx_arc in indexes {
                    let mut idx = idx_arc.write();
                    let table = schema.table(&actual_name).unwrap();
                    let table = table.read();
                    let key_vals: Vec<&PgValue> = idx.key_columns.iter()
                        .filter_map(|col| {
                            let pos = table.column_idx(col)?;
                            row_vals.get(pos)?.as_ref()
                        })
                        .collect();
                    if key_vals.len() == idx.key_columns.len() {
                        let key = IndexKey::from_values(&key_vals);
                        match idx.insert(key, ctid) {
                            Err(e) => {
                                // Handle unique violation
                                if let Some(oc) = on_conflict {
                                    return handle_on_conflict(exec, tbl_str, search_refs, xid, &row_vals, oc, ctid);
                                }
                                return Err(e);
                            }
                            Ok(()) => {}
                        }
                    }
                }
                return Ok(true);
            }
            Err(e) => {
                if let Some(oc) = on_conflict {
                    // DO NOTHING
                    match &oc.action {
                        ast::OnConflictAction::DoNothing => return Ok(false),
                        _ => return Err(e),
                    }
                }
                return Err(e);
            }
        }
    }
}

fn handle_on_conflict(
    exec: &mut Executor,
    tbl_str: &str,
    search_refs: &[&str],
    xid: crate::storage::mvcc::Xid,
    row_vals: &[Option<PgValue>],
    on_conflict: &ast::OnConflict,
    ctid: u64,
) -> Result<bool> {
    match &on_conflict.action {
        ast::OnConflictAction::DoNothing => Ok(false),
        ast::OnConflictAction::DoUpdate(update) => {
            // ON CONFLICT DO UPDATE SET col = expr, ...
            let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
            let (schema_ref, actual_name) = db.resolve_table(tbl_str, search_refs).unwrap();
            let schema = schema_ref.read();
            let table_arc = schema.table(&actual_name).unwrap();
            let mut table = table_arc.write();

            // Find the conflicting tuple and update it
            let existing_ctid = table.tuples.iter()
                .filter(|t| t.xmax == 0)
                .map(|t| t.ctid)
                .last()
                .unwrap_or(0);

            if existing_ctid > 0 {
                let existing_vals: Vec<Option<PgValue>> = table.tuples.iter()
                    .find(|t| t.ctid == existing_ctid)
                    .map(|t| t.data.clone())
                    .unwrap_or_default();

                let mut new_vals = existing_vals.clone();

                let excluded_row: HashMap<String, PgValue> = table.columns.iter().enumerate()
                    .map(|(i, col)| {
                        let val = row_vals.get(i).and_then(|v| v.clone()).unwrap_or(PgValue::Null);
                        (col.name.clone(), val)
                    })
                    .collect();

                for assignment in &update.assignments {
                    let col_name = assignment.target.to_string().to_lowercase();
                    if let Some(idx) = table.column_idx(&col_name) {
                        let mut ctx = EvalContext::with_row(excluded_row.clone());
                        let val = eval_expr(&assignment.value, &ctx)?;
                        new_vals[idx] = if val == PgValue::Null { None } else { Some(val) };
                    }
                }

                table.update_tuple(existing_ctid, xid, new_vals)?;
            }
            Ok(true)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// UPDATE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_update(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (table, assignments, selection, _from, _returning) = match stmt {
        Statement::Update { table, assignments, selection, from, returning, .. } => {
            (table, assignments, selection, from, returning)
        }
        _ => unreachable!(),
    };

    let tbl_str = match &table.relation {
        TableFactor::Table { name, .. } => name.to_string(),
        other => other.to_string(),
    };

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let search_path = exec.config.search_path_refs().iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let search_refs: Vec<&str> = search_path.iter().map(String::as_str).collect();

    let (schema_ref, actual_name) = db.resolve_table(&tbl_str, &search_refs)
        .ok_or_else(|| Error::Pg(PgError::undefined_table(&tbl_str)))?;

    let xid = exec.ensure_transaction();
    let snapshot = db.txn_manager.statement_snapshot(xid);
    let clog = db.txn_manager.clog.as_ref();

    let schema = schema_ref.read();
    let table_arc = schema.table(&actual_name).unwrap();

    // Collect ctids to update and new values
    let to_update: Vec<(u64, Vec<Option<PgValue>>)> = {
        let table = table_arc.read();
        table.scan(&snapshot, clog).filter_map(|tuple| {
            // Build row context for WHERE and SET evaluation
            let row: HashMap<String, PgValue> = table.columns.iter().enumerate()
                .map(|(i, col)| {
                    let val = tuple.data.get(i).and_then(|v| v.clone()).unwrap_or(PgValue::Null);
                    (col.name.clone(), val)
                })
                .collect();

            // Check WHERE
            if let Some(where_expr) = &selection {
                let ctx = EvalContext::with_row(row.clone());
                match eval_expr(where_expr, &ctx) {
                    Ok(PgValue::Bool(true)) => {}
                    Ok(PgValue::Null) | Ok(PgValue::Bool(false)) => return None,
                    Ok(_) => return None,
                    Err(_) => return None,
                }
            }

            // Compute new values
            let mut new_vals = tuple.data.clone();
            for assign in &assignments {
                let col_name = assign.target.to_string().to_lowercase();
                if let Some(idx) = table.column_idx(&col_name) {
                    let ctx = EvalContext::with_row(row.clone());
                    if let Ok(val) = eval_expr(&assign.value, &ctx) {
                        new_vals[idx] = if val == PgValue::Null { None } else { Some(val) };
                    }
                }
            }
            Some((tuple.ctid, new_vals))
        }).collect()
    };

    let count = to_update.len() as u64;

    {
        let mut table = table_arc.write();
        for (ctid, new_vals) in to_update {
            table.check_not_null(&new_vals)?;
            table.update_tuple(ctid, xid, new_vals)?;
        }
    }

    Ok(CommandResult::Modified { tag: format!("UPDATE {}", count), rows_affected: count })
}

// ─────────────────────────────────────────────────────────────────────────────
// DELETE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_delete(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (from, selection, _returning) = match stmt {
        Statement::Delete(ast::Delete { from, selection, returning, .. }) => (from, selection, returning),
        _ => unreachable!(),
    };

    let tables = match &from {
        FromTable::WithFromKeyword(tbls) | FromTable::WithoutKeyword(tbls) => tbls,
    };
    let tbl_str = match tables.first() {
        Some(ast::TableWithJoins { relation: TableFactor::Table { name, .. }, .. }) => name.to_string(),
        _ => return Err(Error::Pg(PgError::syntax_error("invalid DELETE target"))),
    };

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let search_path = exec.config.search_path_refs().iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let search_refs: Vec<&str> = search_path.iter().map(String::as_str).collect();

    let (schema_ref, actual_name) = db.resolve_table(&tbl_str, &search_refs)
        .ok_or_else(|| Error::Pg(PgError::undefined_table(&tbl_str)))?;

    let xid = exec.ensure_transaction();
    let snapshot = db.txn_manager.statement_snapshot(xid);
    let clog = db.txn_manager.clog.as_ref();

    let schema = schema_ref.read();
    let table_arc = schema.table(&actual_name).unwrap();

    let to_delete: Vec<u64> = {
        let table = table_arc.read();
        table.scan(&snapshot, clog).filter_map(|tuple| {
            if let Some(where_expr) = &selection {
                let row: HashMap<String, PgValue> = table.columns.iter().enumerate()
                    .map(|(i, col)| {
                        let val = tuple.data.get(i).and_then(|v| v.clone()).unwrap_or(PgValue::Null);
                        (col.name.clone(), val)
                    })
                    .collect();
                let ctx = EvalContext::with_row(row);
                match eval_expr(where_expr, &ctx) {
                    Ok(PgValue::Bool(true)) => Some(tuple.ctid),
                    _ => None,
                }
            } else {
                Some(tuple.ctid)
            }
        }).collect()
    };

    let count = to_delete.len() as u64;

    {
        let mut table = table_arc.write();
        for ctid in to_delete {
            table.delete_tuple(ctid, xid)?;
        }
    }

    Ok(CommandResult::Modified { tag: format!("DELETE {}", count), rows_affected: count })
}

// ─────────────────────────────────────────────────────────────────────────────
// TRUNCATE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_truncate(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let table_names = match stmt {
        Statement::Truncate { table_names, .. } => table_names,
        _ => unreachable!(),
    };

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let search_path = exec.config.search_path_refs().iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let search_refs: Vec<&str> = search_path.iter().map(String::as_str).collect();

    for tbl_ref in &table_names {
        let tbl_str = tbl_ref.name.to_string();
        if let Some((schema_ref, actual_name)) = db.resolve_table(&tbl_str, &search_refs) {
            let schema = schema_ref.read();
            if let Some(table_arc) = schema.table(&actual_name) {
                table_arc.write().truncate();
            }
        }
    }

    Ok(CommandResult::Transaction("TRUNCATE TABLE".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// COPY
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_copy(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    // COPY is handled at the protocol level in server.rs.
    // Here we just return a stub that triggers the protocol-level COPY state.
    Ok(CommandResult::Copy { direction: "OUT".to_string(), rows: 0 })
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Evaluate a default expression string for a column.
fn evaluate_default(default_expr: &str, exec: &Executor, type_oid: crate::types::Oid) -> PgValue {
    // Handle common defaults
    match default_expr.to_lowercase().trim() {
        "now()" | "current_timestamp" | "current_timestamp()" => {
            PgValue::TimestampTz(chrono::Utc::now())
        }
        "true" => PgValue::Bool(true),
        "false" => PgValue::Bool(false),
        s if s.starts_with("nextval(") => {
            // Sequence default — will be resolved by executor
            PgValue::Text(format!("__nextval__{}", &s[8..s.len()-1]))
        }
        _ => PgValue::Null,
    }
}

/// Cast a PgValue to the target column OID, best-effort.
fn cast_value_to_col(val: PgValue, type_oid: crate::types::Oid) -> Result<PgValue> {
    if val == PgValue::Null {
        return Ok(PgValue::Null);
    }
    // Attempt type cast using the types module
    val.cast_to(type_oid).map_err(|e| Error::Pg(PgError::error(
        SqlState::DATATYPE_MISMATCH,
        e.to_string(),
    )))
}
