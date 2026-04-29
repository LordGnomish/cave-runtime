//! Plan executor — turns a `Plan` into a `ResultSet`.

use std::sync::Arc;

use parking_lot::RwLock;

use super::catalog::{ColumnDef, TableSchema};
use super::planner::{Plan, PlanExpr, PlanOp, Projection};
use super::txn::{IsolationLevel, TxnId, TxnManager, TxnState};
use super::value::{ScalarType, ScalarValue};
use super::SqlEngine;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ExecutionError {
    #[error("table not found: {0}")]
    TableNotFound(String),
    #[error("column not found: {0}")]
    ColumnNotFound(String),
    #[error("type mismatch")]
    TypeMismatch,
    #[error("not in transaction")]
    NotInTxn,
    #[error("already in transaction")]
    AlreadyInTxn,
    #[error("storage: {0}")]
    Storage(String),
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("evaluation: {0}")]
    Eval(String),
}

pub type ExecutionResult<T> = Result<T, ExecutionError>;

#[derive(Debug, Clone, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<(String, ScalarType)>,
    pub rows: Vec<Vec<Option<String>>>,
    pub command_tag: String,
    pub txn_after: Option<TxnId>,
}

impl ResultSet {
    pub fn empty(tag: impl Into<String>) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            command_tag: tag.into(),
            txn_after: None,
        }
    }
}

pub fn execute(plan: Plan, engine: &SqlEngine, txn: Option<TxnId>) -> ExecutionResult<ResultSet> {
    match plan {
        Plan::CreateTable {
            schema,
            name,
            columns,
            if_not_exists,
        } => exec_create_table(engine, &schema, &name, columns, if_not_exists),
        Plan::DropTable {
            schema,
            name,
            if_exists,
        } => exec_drop_table(engine, &schema, &name, if_exists),
        Plan::Insert {
            schema,
            table,
            columns,
            rows,
        } => exec_insert(engine, &schema, &table, columns, rows, txn.unwrap_or(TxnId(1))),
        Plan::Select {
            from,
            projection,
            filter,
            order_by,
            limit,
            offset,
        } => exec_select(engine, &from.schema, &from.name, projection, filter, order_by, limit, offset),
        Plan::Update {
            schema,
            table,
            assignments,
            filter,
        } => exec_update(engine, &schema, &table, assignments, filter),
        Plan::Delete {
            schema,
            table,
            filter,
        } => exec_delete(engine, &schema, &table, filter),
        Plan::Begin => {
            if txn.is_some() {
                return Err(ExecutionError::AlreadyInTxn);
            }
            let id = engine.txn_mgr.write().begin(IsolationLevel::ReadCommitted, false);
            Ok(ResultSet {
                txn_after: Some(id),
                ..ResultSet::empty("BEGIN")
            })
        }
        Plan::Commit => match txn {
            Some(id) => {
                engine
                    .txn_mgr
                    .write()
                    .commit(id)
                    .map_err(|e| ExecutionError::Eval(e.to_string()))?;
                Ok(ResultSet::empty("COMMIT"))
            }
            None => Ok(ResultSet::empty("COMMIT")),
        },
        Plan::Rollback => match txn {
            Some(id) => {
                engine
                    .txn_mgr
                    .write()
                    .abort(id)
                    .map_err(|e| ExecutionError::Eval(e.to_string()))?;
                Ok(ResultSet::empty("ROLLBACK"))
            }
            None => Ok(ResultSet::empty("ROLLBACK")),
        },
        Plan::SelectConst(consts) => {
            let columns: Vec<(String, ScalarType)> = consts
                .iter()
                .map(|(name, v)| {
                    let ty = v.type_of().unwrap_or(ScalarType::Text);
                    (name.clone(), ty)
                })
                .collect();
            let row: Vec<Option<String>> = consts.iter().map(|(_, v)| v.to_text()).collect();
            Ok(ResultSet {
                columns,
                rows: vec![row],
                command_tag: "SELECT 1".into(),
                txn_after: None,
            })
        }
        Plan::SetParameter { .. } => Ok(ResultSet::empty("SET")),
        Plan::Empty => Ok(ResultSet::empty("")),
    }
}

fn exec_create_table(
    engine: &SqlEngine,
    schema: &str,
    name: &str,
    columns: Vec<ColumnDef>,
    if_not_exists: bool,
) -> ExecutionResult<ResultSet> {
    let qual = qualify(schema, name);
    let column_count = columns.len();
    {
        let mut cat = engine.catalog.write();
        match cat.create_table(schema, name, columns) {
            Ok(_) => {}
            Err(super::catalog::CatalogError::TableExists(_)) if if_not_exists => {
                return Ok(ResultSet::empty("CREATE TABLE"));
            }
            Err(e) => return Err(ExecutionError::Catalog(e.to_string())),
        }
    }
    engine.storage.write().create_table(&qual, column_count);
    Ok(ResultSet::empty("CREATE TABLE"))
}

fn exec_drop_table(
    engine: &SqlEngine,
    schema: &str,
    name: &str,
    if_exists: bool,
) -> ExecutionResult<ResultSet> {
    let qual = qualify(schema, name);
    let mut cat = engine.catalog.write();
    match cat.drop_table(schema, name) {
        Ok(()) => {
            let _ = engine.storage.write().drop_table(&qual);
            Ok(ResultSet::empty("DROP TABLE"))
        }
        Err(super::catalog::CatalogError::TableNotFound(_)) if if_exists => {
            Ok(ResultSet::empty("DROP TABLE"))
        }
        Err(e) => Err(ExecutionError::Catalog(e.to_string())),
    }
}

fn exec_insert(
    engine: &SqlEngine,
    schema: &str,
    name: &str,
    columns: Option<Vec<String>>,
    rows: Vec<Vec<PlanExpr>>,
    txn: TxnId,
) -> ExecutionResult<ResultSet> {
    let qual = qualify(schema, name);
    let table_schema = engine
        .catalog
        .read()
        .lookup(schema, name)
        .map(|s| s.clone())
        .map_err(|_| ExecutionError::TableNotFound(name.into()))?;
    let target_indices: Vec<usize> = match &columns {
        Some(cols) => cols
            .iter()
            .map(|c| {
                table_schema
                    .column_index(c)
                    .ok_or_else(|| ExecutionError::ColumnNotFound(c.clone()))
            })
            .collect::<ExecutionResult<Vec<_>>>()?,
        None => (0..table_schema.columns.len()).collect(),
    };

    let mut inserted = 0u64;
    for row_exprs in rows {
        let mut row_values = vec![ScalarValue::Null; table_schema.columns.len()];
        if row_exprs.len() != target_indices.len() {
            return Err(ExecutionError::TypeMismatch);
        }
        for (target_idx, expr) in target_indices.iter().zip(row_exprs.into_iter()) {
            let v = evaluate_const(&expr, &table_schema)?;
            let coerced = coerce(&v, table_schema.columns[*target_idx].ty)?;
            row_values[*target_idx] = coerced;
        }
        engine
            .storage
            .write()
            .insert(&qual, row_values, txn.0)
            .map_err(|e| ExecutionError::Storage(e.to_string()))?;
        inserted += 1;
    }
    Ok(ResultSet::empty(format!("INSERT 0 {}", inserted)))
}

fn exec_select(
    engine: &SqlEngine,
    schema: &str,
    name: &str,
    projection: Vec<Projection>,
    filter: Option<PlanExpr>,
    order_by: Vec<(usize, bool)>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> ExecutionResult<ResultSet> {
    let qual = qualify(schema, name);
    let table_schema = engine
        .catalog
        .read()
        .lookup(schema, name)
        .map(|s| s.clone())
        .map_err(|_| ExecutionError::TableNotFound(name.into()))?;

    let mut rows: Vec<(u64, Vec<ScalarValue>)> = engine
        .storage
        .read()
        .scan(&qual)
        .map_err(|e| ExecutionError::Storage(e.to_string()))?
        .into_iter()
        .map(|(id, row)| (id, row.values))
        .collect();

    if let Some(ref pred) = filter {
        rows.retain(|(_, vals)| {
            evaluate_pred(pred, &table_schema, vals).unwrap_or(ScalarValue::Bool(false))
                == ScalarValue::Bool(true)
        });
    }

    if !order_by.is_empty() {
        rows.sort_by(|(_, a), (_, b)| {
            for (idx, ascending) in &order_by {
                let cmp = a[*idx].cmp_loose(&b[*idx]);
                let cmp = if *ascending { cmp } else { cmp.reverse() };
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    let off = offset.unwrap_or(0);
    let lim = limit.unwrap_or(usize::MAX);
    let selected: Vec<(u64, Vec<ScalarValue>)> = rows.into_iter().skip(off).take(lim).collect();
    let row_count = selected.len();

    // Build column metadata + row text representation per projection.
    let (output_columns, output_indices) = resolve_projection(&projection, &table_schema)?;

    let mut output_rows: Vec<Vec<Option<String>>> = Vec::with_capacity(selected.len());
    for (_, vals) in &selected {
        let mut out = Vec::with_capacity(output_indices.len());
        for source in &output_indices {
            match source {
                ProjectionSource::Column(idx) => out.push(vals[*idx].to_text()),
                ProjectionSource::Literal(v) => out.push(v.to_text()),
            }
        }
        output_rows.push(out);
    }

    Ok(ResultSet {
        columns: output_columns,
        rows: output_rows,
        command_tag: format!("SELECT {}", row_count),
        txn_after: None,
    })
}

fn exec_update(
    engine: &SqlEngine,
    schema: &str,
    name: &str,
    assignments: Vec<(String, PlanExpr)>,
    filter: Option<PlanExpr>,
) -> ExecutionResult<ResultSet> {
    let qual = qualify(schema, name);
    let table_schema = engine
        .catalog
        .read()
        .lookup(schema, name)
        .map(|s| s.clone())
        .map_err(|_| ExecutionError::TableNotFound(name.into()))?;
    let target_indices: Vec<(usize, ScalarValue)> = assignments
        .iter()
        .map(|(col, expr)| {
            let idx = table_schema
                .column_index(col)
                .ok_or_else(|| ExecutionError::ColumnNotFound(col.clone()))?;
            let v = evaluate_const(expr, &table_schema)?;
            let v = coerce(&v, table_schema.columns[idx].ty)?;
            Ok((idx, v))
        })
        .collect::<ExecutionResult<Vec<_>>>()?;

    let row_ids: Vec<u64> = {
        let snapshot = engine
            .storage
            .read()
            .scan(&qual)
            .map_err(|e| ExecutionError::Storage(e.to_string()))?;
        snapshot
            .into_iter()
            .filter(|(_, row)| match &filter {
                None => true,
                Some(pred) => evaluate_pred(pred, &table_schema, &row.values)
                    .unwrap_or(ScalarValue::Bool(false))
                    == ScalarValue::Bool(true),
            })
            .map(|(id, _)| id)
            .collect()
    };

    let mut updated = 0u64;
    for id in row_ids {
        let mut storage = engine.storage.write();
        let row = storage
            .get_row(&qual, id)
            .map_err(|e| ExecutionError::Storage(e.to_string()))?;
        let mut new_values = row.values.clone();
        for (idx, v) in &target_indices {
            new_values[*idx] = v.clone();
        }
        storage
            .update(&qual, id, new_values)
            .map_err(|e| ExecutionError::Storage(e.to_string()))?;
        updated += 1;
    }
    Ok(ResultSet::empty(format!("UPDATE {}", updated)))
}

fn exec_delete(
    engine: &SqlEngine,
    schema: &str,
    name: &str,
    filter: Option<PlanExpr>,
) -> ExecutionResult<ResultSet> {
    let qual = qualify(schema, name);
    let table_schema = engine
        .catalog
        .read()
        .lookup(schema, name)
        .map(|s| s.clone())
        .map_err(|_| ExecutionError::TableNotFound(name.into()))?;

    let row_ids: Vec<u64> = {
        let snapshot = engine
            .storage
            .read()
            .scan(&qual)
            .map_err(|e| ExecutionError::Storage(e.to_string()))?;
        snapshot
            .into_iter()
            .filter(|(_, row)| match &filter {
                None => true,
                Some(pred) => evaluate_pred(pred, &table_schema, &row.values)
                    .unwrap_or(ScalarValue::Bool(false))
                    == ScalarValue::Bool(true),
            })
            .map(|(id, _)| id)
            .collect()
    };

    let mut deleted = 0u64;
    for id in row_ids {
        engine
            .storage
            .write()
            .delete(&qual, id, /*xmax*/ 1)
            .map_err(|e| ExecutionError::Storage(e.to_string()))?;
        deleted += 1;
    }
    Ok(ResultSet::empty(format!("DELETE {}", deleted)))
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn qualify(schema: &str, name: &str) -> String {
    format!("{}.{}", schema, name)
}

#[derive(Debug)]
enum ProjectionSource {
    Column(usize),
    Literal(ScalarValue),
}

fn resolve_projection(
    projection: &[Projection],
    table: &TableSchema,
) -> ExecutionResult<(Vec<(String, ScalarType)>, Vec<ProjectionSource>)> {
    let mut columns = Vec::new();
    let mut sources = Vec::new();
    for p in projection {
        match p {
            Projection::Star => {
                for (i, col) in table.columns.iter().enumerate() {
                    columns.push((col.name.clone(), col.ty));
                    sources.push(ProjectionSource::Column(i));
                }
            }
            Projection::Column(name) => {
                let idx = table
                    .column_index(name)
                    .ok_or_else(|| ExecutionError::ColumnNotFound(name.clone()))?;
                columns.push((name.clone(), table.columns[idx].ty));
                sources.push(ProjectionSource::Column(idx));
            }
            Projection::Literal { name, value } => {
                let ty = value.type_of().unwrap_or(ScalarType::Text);
                columns.push((name.clone(), ty));
                sources.push(ProjectionSource::Literal(value.clone()));
            }
        }
    }
    Ok((columns, sources))
}

fn evaluate_const(expr: &PlanExpr, table: &TableSchema) -> ExecutionResult<ScalarValue> {
    match expr {
        PlanExpr::Literal(v) => Ok(v.clone()),
        PlanExpr::Column(name) => Err(ExecutionError::ColumnNotFound(name.clone())),
        PlanExpr::Binary { left, op, right } => {
            let l = evaluate_const(left, table)?;
            let r = evaluate_const(right, table)?;
            apply_op(*op, &l, &r)
        }
    }
}

fn evaluate_pred(
    pred: &PlanExpr,
    table: &TableSchema,
    row: &[ScalarValue],
) -> ExecutionResult<ScalarValue> {
    match pred {
        PlanExpr::Literal(v) => Ok(v.clone()),
        PlanExpr::Column(name) => {
            let idx = table
                .column_index(name)
                .ok_or_else(|| ExecutionError::ColumnNotFound(name.clone()))?;
            Ok(row[idx].clone())
        }
        PlanExpr::Binary { left, op, right } => {
            let l = evaluate_pred(left, table, row)?;
            let r = evaluate_pred(right, table, row)?;
            apply_op(*op, &l, &r)
        }
    }
}

fn apply_op(op: PlanOp, l: &ScalarValue, r: &ScalarValue) -> ExecutionResult<ScalarValue> {
    use std::cmp::Ordering;
    let cmp = l.cmp_loose(r);
    Ok(match op {
        PlanOp::Eq => ScalarValue::Bool(cmp == Ordering::Equal),
        PlanOp::NotEq => ScalarValue::Bool(cmp != Ordering::Equal),
        PlanOp::Lt => ScalarValue::Bool(cmp == Ordering::Less),
        PlanOp::LtEq => ScalarValue::Bool(cmp != Ordering::Greater),
        PlanOp::Gt => ScalarValue::Bool(cmp == Ordering::Greater),
        PlanOp::GtEq => ScalarValue::Bool(cmp != Ordering::Less),
        PlanOp::And | PlanOp::Or => match (l, r) {
            (ScalarValue::Bool(a), ScalarValue::Bool(b)) => match op {
                PlanOp::And => ScalarValue::Bool(*a && *b),
                PlanOp::Or => ScalarValue::Bool(*a || *b),
                _ => unreachable!(),
            },
            _ => return Err(ExecutionError::TypeMismatch),
        },
    })
}

fn coerce(v: &ScalarValue, ty: ScalarType) -> ExecutionResult<ScalarValue> {
    if matches!(v, ScalarValue::Null) {
        return Ok(ScalarValue::Null);
    }
    match (v, ty) {
        (ScalarValue::Int4(n), ScalarType::Int8) => Ok(ScalarValue::Int8(*n as i64)),
        (ScalarValue::Int8(n), ScalarType::Int4) => {
            Ok(ScalarValue::Int4(*n as i32))
        }
        (ScalarValue::Int4(n), ScalarType::Int2) => Ok(ScalarValue::Int2(*n as i16)),
        (ScalarValue::Text(s), ScalarType::Text | ScalarType::Varchar | ScalarType::Json | ScalarType::Jsonb) => {
            Ok(ScalarValue::Text(s.clone()))
        }
        (v, ty) if v.type_of() == Some(ty) => Ok(v.clone()),
        _ => Err(ExecutionError::TypeMismatch),
    }
}

// Re-export so the top-level `engine::execute_sql` can pass it through.
pub use ResultSet as PublicResultSet;

// Suppress unused import warnings for items intentionally re-exported.
#[allow(dead_code)]
fn _keep_arc_rwlock<T>(_: Arc<RwLock<T>>) {}
#[allow(dead_code)]
fn _keep_txn(_: TxnState) {}
#[allow(dead_code)]
fn _keep_txn_mgr(_: TxnManager) {}

#[cfg(test)]
mod tests {
    use super::super::execute_sql;
    use super::*;
    use crate::engine::SqlEngine;

    fn run(engine: &SqlEngine, sql: &str) -> ResultSet {
        execute_sql(engine, sql, None).unwrap()
    }

    #[test]
    fn create_then_insert_then_select() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT, name TEXT)");
        run(&e, "INSERT INTO t (id, name) VALUES (1, 'alice'), (2, 'bob')");
        let r = run(&e, "SELECT * FROM t ORDER BY id");
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.command_tag, "SELECT 2");
        assert_eq!(r.rows[0][0].as_deref(), Some("1"));
        assert_eq!(r.rows[0][1].as_deref(), Some("alice"));
    }

    #[test]
    fn select_const_returns_one_row() {
        let e = SqlEngine::new();
        let r = run(&e, "SELECT 42");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0].as_deref(), Some("42"));
    }

    #[test]
    fn select_with_where_filters() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        run(&e, "INSERT INTO t VALUES (1), (2), (3)");
        let r = run(&e, "SELECT * FROM t WHERE id = 2");
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0][0].as_deref(), Some("2"));
    }

    #[test]
    fn select_with_limit_caps_rows() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        for i in 0..10 {
            run(&e, &format!("INSERT INTO t VALUES ({})", i));
        }
        let r = run(&e, "SELECT * FROM t ORDER BY id LIMIT 3");
        assert_eq!(r.rows.len(), 3);
    }

    #[test]
    fn select_with_order_by_desc() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        run(&e, "INSERT INTO t VALUES (1), (2), (3)");
        let r = run(&e, "SELECT * FROM t ORDER BY id DESC");
        assert_eq!(r.rows[0][0].as_deref(), Some("3"));
        assert_eq!(r.rows[2][0].as_deref(), Some("1"));
    }

    #[test]
    fn update_modifies_matching_rows() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT, name TEXT)");
        run(&e, "INSERT INTO t VALUES (1, 'a'), (2, 'b')");
        run(&e, "UPDATE t SET name = 'X' WHERE id = 1");
        let r = run(&e, "SELECT name FROM t WHERE id = 1");
        assert_eq!(r.rows[0][0].as_deref(), Some("X"));
    }

    #[test]
    fn delete_removes_matching_rows() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        run(&e, "INSERT INTO t VALUES (1), (2), (3)");
        run(&e, "DELETE FROM t WHERE id = 2");
        let r = run(&e, "SELECT * FROM t ORDER BY id");
        assert_eq!(r.rows.len(), 2);
    }

    #[test]
    fn drop_table_then_select_errors() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        run(&e, "DROP TABLE t");
        let res = execute_sql(&e, "SELECT * FROM t", None);
        assert!(res.is_err());
    }

    #[test]
    fn drop_if_exists_no_error() {
        let e = SqlEngine::new();
        let r = run(&e, "DROP TABLE IF EXISTS missing");
        assert_eq!(r.command_tag, "DROP TABLE");
    }

    #[test]
    fn insert_into_unknown_table_errors() {
        let e = SqlEngine::new();
        let res = execute_sql(&e, "INSERT INTO nope VALUES (1)", None);
        assert!(res.is_err());
    }

    #[test]
    fn select_columns_subset() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (a INT, b INT, c INT)");
        run(&e, "INSERT INTO t VALUES (1, 2, 3)");
        let r = run(&e, "SELECT b, a FROM t");
        assert_eq!(r.columns[0].0, "b");
        assert_eq!(r.columns[1].0, "a");
        assert_eq!(r.rows[0][0].as_deref(), Some("2"));
        assert_eq!(r.rows[0][1].as_deref(), Some("1"));
    }

    #[test]
    fn select_with_offset_skips_rows() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        for i in 1..=5 {
            run(&e, &format!("INSERT INTO t VALUES ({})", i));
        }
        let r = run(&e, "SELECT * FROM t ORDER BY id LIMIT 2 OFFSET 2");
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0][0].as_deref(), Some("3"));
    }

    #[test]
    fn create_if_not_exists_idempotent() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        let r = run(&e, "CREATE TABLE IF NOT EXISTS t (id INT)");
        assert_eq!(r.command_tag, "CREATE TABLE");
    }

    #[test]
    fn delete_without_where_clears_table() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT)");
        run(&e, "INSERT INTO t VALUES (1), (2)");
        run(&e, "DELETE FROM t");
        let r = run(&e, "SELECT * FROM t");
        assert_eq!(r.rows.len(), 0);
    }

    #[test]
    fn boolean_filter_combines_and_or() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (a INT, b INT)");
        run(&e, "INSERT INTO t VALUES (1, 10), (2, 20), (3, 30)");
        let r = run(&e, "SELECT * FROM t WHERE a = 1 OR b = 30 ORDER BY a");
        assert_eq!(r.rows.len(), 2);
    }

    #[test]
    fn update_all_rows_when_no_where() {
        let e = SqlEngine::new();
        run(&e, "CREATE TABLE t (id INT, x INT)");
        run(&e, "INSERT INTO t VALUES (1, 1), (2, 2)");
        run(&e, "UPDATE t SET x = 99");
        let r = run(&e, "SELECT x FROM t");
        for row in &r.rows {
            assert_eq!(row[0].as_deref(), Some("99"));
        }
    }

    #[test]
    fn begin_returns_txn_id() {
        let e = SqlEngine::new();
        let r = run(&e, "BEGIN");
        assert!(r.txn_after.is_some());
    }
}
