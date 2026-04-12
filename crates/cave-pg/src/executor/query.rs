//! SELECT query execution — scans, joins, aggregates, CTEs, window functions.

use std::collections::HashMap;
use sqlparser::ast::{self as ast, Query, Select, SelectItem, SetExpr, TableFactor, TableWithJoins, JoinOperator, JoinConstraint, OrderByExpr, GroupByExpr, Expr, WildcardAdditionalOptions, FunctionArguments};
use crate::error::{Error, PgError, Result, SqlState};
use crate::executor::{Executor, EvalContext};
use crate::executor::expr::{eval_expr, like_match};
use crate::storage::heap::Table;
use crate::storage::mvcc::{IsolationLevel, Snapshot};
use crate::types::{ColumnDesc, CommandResult, Oid, PgValue, ResultSet};
use crate::catalog;

/// A "logical row" during query processing: column names → values.
type Row = HashMap<String, PgValue>;

pub fn execute_query(exec: &mut Executor, query: Query) -> Result<CommandResult> {
    // Handle CTEs first
    let mut cte_results: HashMap<String, Vec<Row>> = HashMap::new();
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            let name = cte.alias.name.value.to_lowercase();
            let cte_query = cte.query.as_ref();
            let result = exec_set_expr(exec, &cte_query.body, &cte_results)?;
            cte_results.insert(name, result);
        }
    }

    let mut rows = exec_set_expr(exec, &query.body, &cte_results)?;

    // ORDER BY
    if let Some(ref order_by) = query.order_by {
        if !order_by.exprs.is_empty() {
            apply_order_by(&mut rows, &order_by.exprs)?;
        }
    }

    // OFFSET
    if let Some(offset_expr) = &query.offset {
        let ctx = EvalContext::new();
        let offset = eval_expr(&offset_expr.value, &ctx)?.to_i64().unwrap_or(0) as usize;
        if offset < rows.len() {
            rows = rows.into_iter().skip(offset).collect();
        } else {
            rows = Vec::new();
        }
    }

    // LIMIT / FETCH FIRST
    let limit: Option<usize> = if let Some(limit_expr) = &query.limit {
        let ctx = EvalContext::new();
        Some(eval_expr(limit_expr, &ctx)?.to_i64().unwrap_or(i64::MAX) as usize)
    } else if let Some(fetch) = &query.fetch {
        let ctx = EvalContext::new();
        let n = if let Some(q) = &fetch.quantity {
            eval_expr(q, &ctx)?.to_i64().unwrap_or(i64::MAX) as usize
        } else {
            1
        };
        Some(n)
    } else {
        None
    };

    if let Some(limit) = limit {
        rows.truncate(limit);
    }

    // Convert rows to ResultSet
    // Get column names from first row (or from SELECT items)
    let col_names: Vec<String> = if let Some(first) = rows.first() {
        first.keys().cloned().collect()
    } else {
        // Get from SELECT item names
        match query.body.as_ref() {
            SetExpr::Select(s) => {
                s.projection.iter().map(|item| match item {
                    SelectItem::UnnamedExpr(e) => expr_name(e),
                    SelectItem::ExprWithAlias { alias, .. } => alias.value.clone(),
                    SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..) => "*".to_string(),
                }).collect()
            }
            _ => vec![],
        }
    };

    let cols: Vec<ColumnDesc> = col_names.iter().map(|name| {
        ColumnDesc::new(name.clone(), crate::types::oid::TEXT)
    }).collect();

    let mut rs = ResultSet::new(cols.clone());
    for row in &rows {
        let values: Vec<PgValue> = col_names.iter().map(|name| {
            row.get(name).cloned().unwrap_or(PgValue::Null)
        }).collect();
        rs.push_row(values);
    }

    Ok(CommandResult::Rows(rs))
}

fn exec_set_expr(exec: &mut Executor, body: &SetExpr, ctes: &HashMap<String, Vec<Row>>) -> Result<Vec<Row>> {
    match body {
        SetExpr::Select(s) => exec_select(exec, s, ctes),
        SetExpr::Query(q) => exec_set_expr(exec, &q.body, ctes),
        SetExpr::SetOperation { op, left, right, set_quantifier } => {
            let mut left_rows = exec_set_expr(exec, left, ctes)?;
            let right_rows = exec_set_expr(exec, right, ctes)?;
            let all = matches!(set_quantifier, ast::SetQuantifier::All);
            match op {
                ast::SetOperator::Union => {
                    if all {
                        left_rows.extend(right_rows);
                    } else {
                        left_rows.extend(right_rows);
                        left_rows = dedup_rows(left_rows);
                    }
                    Ok(left_rows)
                }
                ast::SetOperator::Intersect => {
                    let result: Vec<Row> = left_rows.into_iter()
                        .filter(|lr| right_rows.iter().any(|rr| rows_equal(lr, rr)))
                        .collect();
                    Ok(if all { result } else { dedup_rows(result) })
                }
                ast::SetOperator::Except => {
                    let result: Vec<Row> = left_rows.into_iter()
                        .filter(|lr| !right_rows.iter().any(|rr| rows_equal(lr, rr)))
                        .collect();
                    Ok(if all { result } else { dedup_rows(result) })
                }
                _ => Err(Error::Pg(PgError::feature_not_supported("set operation"))),
            }
        }
        SetExpr::Values(values) => {
            let mut rows = Vec::new();
            for (row_idx, row_exprs) in values.rows.iter().enumerate() {
                let ctx = EvalContext::new();
                let mut row: Row = HashMap::new();
                for (col_idx, expr) in row_exprs.iter().enumerate() {
                    let val = eval_expr(expr, &ctx)?;
                    row.insert(format!("column{}", col_idx + 1), val);
                }
                rows.push(row);
            }
            Ok(rows)
        }
        _ => Err(Error::Pg(PgError::feature_not_supported("set expression"))),
    }
}

fn exec_select(exec: &mut Executor, select: &Select, ctes: &HashMap<String, Vec<Row>>) -> Result<Vec<Row>> {
    // 1. FROM clause — build initial row stream
    let mut rows = if select.from.is_empty() {
        // SELECT without FROM — single virtual row
        vec![HashMap::new()]
    } else {
        let mut result = Vec::new();
        for (i, table_with_joins) in select.from.iter().enumerate() {
            let from_rows = eval_table_with_joins(exec, table_with_joins, ctes)?;
            if i == 0 {
                result = from_rows;
            } else {
                // Implicit CROSS JOIN
                let mut joined = Vec::new();
                for left_row in &result {
                    for right_row in &from_rows {
                        let mut combined = left_row.clone();
                        combined.extend(right_row.clone());
                        joined.push(combined);
                    }
                }
                result = joined;
            }
        }
        result
    };

    // 2. WHERE clause
    if let Some(selection) = &select.selection {
        rows = rows.into_iter().filter(|row| {
            let ctx = EvalContext::with_row(row.clone());
            eval_expr(selection, &ctx).map(|v| v.is_true()).unwrap_or(false)
        }).collect();
    }

    // 3. GROUP BY + aggregation
    let has_aggregates = select.projection.iter().any(|item| has_agg_in_projection(item))
        || !is_empty_group_by(&select.group_by);

    if has_aggregates {
        rows = apply_group_by_and_agg(exec, select, rows)?;
    } else {
        // 4. HAVING (only valid with GROUP BY, but allow it for non-grouped queries)
        if let Some(having) = &select.having {
            rows = rows.into_iter().filter(|row| {
                let ctx = EvalContext::with_row(row.clone());
                eval_expr(having, &ctx).map(|v| v.is_true()).unwrap_or(false)
            }).collect();
        }

        // 5. SELECT projection
        rows = project_rows(exec, select, rows)?;
    }

    // 6. DISTINCT
    if select.distinct.is_some() {
        rows = dedup_rows(rows);
    }

    Ok(rows)
}

fn eval_table_with_joins(
    exec: &mut Executor,
    table_with_joins: &TableWithJoins,
    ctes: &HashMap<String, Vec<Row>>,
) -> Result<Vec<Row>> {
    let mut rows = eval_table_factor(exec, &table_with_joins.relation, ctes)?;

    for join in &table_with_joins.joins {
        let right_rows = eval_table_factor(exec, &join.relation, ctes)?;
        rows = apply_join(rows, right_rows, &join.join_operator)?;
    }

    Ok(rows)
}

fn eval_table_factor(
    exec: &mut Executor,
    factor: &TableFactor,
    ctes: &HashMap<String, Vec<Row>>,
) -> Result<Vec<Row>> {
    match factor {
        TableFactor::Table { name, alias, .. } => {
            let table_name = name.to_string();
            let alias_name = alias.as_ref().map(|a| a.name.value.to_lowercase());

            // Check for system catalog queries
            let (schema_name, bare_name) = if let Some((s, t)) = table_name.split_once('.') {
                (s.to_lowercase(), t.to_lowercase())
            } else {
                ("public".to_string(), table_name.to_lowercase())
            };

            // System catalog dispatch
            let catalog_result = if let Some(db) = exec.db() {
                match (schema_name.as_str(), bare_name.as_str()) {
                    ("pg_catalog", _) | (_, "pg_namespace") | (_, "pg_class") |
                    (_, "pg_attribute") | (_, "pg_type") | (_, "pg_index") |
                    (_, "pg_constraint") | (_, "pg_roles") | (_, "pg_authid") |
                    (_, "pg_settings") | (_, "pg_stat_activity") | (_, "pg_stats") => {
                        Some(catalog::query_catalog_table(&schema_name, &bare_name, &exec.engine, &db))
                    }
                    ("information_schema", _) => {
                        Some(catalog::query_catalog_table("information_schema", &bare_name, &exec.engine, &db))
                    }
                    _ => None,
                }
            } else { None };

            if let Some(Some(rs)) = catalog_result {
                return Ok(result_set_to_rows(&rs, alias_name.as_deref().unwrap_or(&bare_name)));
            }

            // Check CTEs
            if let Some(cte_rows) = ctes.get(&bare_name) {
                let prefix = alias_name.as_deref().unwrap_or(&bare_name);
                let rows = cte_rows.iter().map(|row| {
                    row.iter().map(|(k, v)| (format!("{prefix}.{k}"), v.clone())).chain(
                        row.iter().map(|(k, v)| (k.clone(), v.clone()))
                    ).collect()
                }).collect();
                return Ok(rows);
            }

            // Regular table scan
            let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
            let search_path = exec.config.search_path_refs();
            let (schema_ref, actual_name) = db.resolve_table(&table_name, &search_path)
                .or_else(|| db.resolve_view(&table_name, &search_path).map(|(s, n)| (s, n)))
                .ok_or_else(|| Error::Pg(PgError::undefined_table(&table_name)))?;

            // Check if it's a view first
            {
                let schema = schema_ref.read();
                if let Some(view_ref) = schema.view(&actual_name) {
                    let view = view_ref.read();
                    if view.is_materialized {
                        if let Some(cached) = &view.cached_result {
                            let col_names: Vec<String> = view.columns.iter().map(|c| c.name.clone()).collect();
                            let prefix = alias_name.as_deref().unwrap_or(&actual_name);
                            return Ok(cached.iter().map(|row| {
                                col_names.iter().zip(row.iter()).flat_map(|(col, val)| {
                                    let v = val.clone().unwrap_or(PgValue::Null);
                                    vec![(format!("{prefix}.{col}"), v.clone()), (col.clone(), v)]
                                }).collect()
                            }).collect());
                        }
                    }
                    let query = view.query.clone();
                    drop(view);
                    drop(schema);
                    // Execute view's defining query
                    let view_stmts = crate::executor::parse_sql(&query)?;
                    if let Some(stmt) = view_stmts.into_iter().next() {
                        if let sqlparser::ast::Statement::Query(q) = stmt {
                            return exec_set_expr(exec, &q.body, ctes);
                        }
                    }
                    return Ok(vec![]);
                }
            }

            let schema = schema_ref.read();
            let table_ref = schema.table(&actual_name)
                .ok_or_else(|| Error::Pg(PgError::undefined_table(&actual_name)))?;
            let table = table_ref.read();
            let prefix = alias_name.as_deref().unwrap_or(&actual_name);

            let xid = exec.current_xid();
            let clog = db.txn_manager.clog.as_ref();
            let snapshot = if xid == crate::storage::mvcc::XID_INVALID {
                // No transaction — read everything committed
                crate::storage::mvcc::Snapshot::new(
                    0,
                    db.txn_manager.xid_counter_value(),
                    Default::default(),
                )
            } else {
                match exec.config.default_transaction_isolation {
                    IsolationLevel::ReadCommitted => db.txn_manager.statement_snapshot(xid),
                    _ => db.txn_manager.transaction_snapshot(xid)
                        .unwrap_or_else(|| db.txn_manager.statement_snapshot(xid)),
                }
            };

            let col_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
            let rows: Vec<Row> = table.scan(&snapshot, clog).map(|tuple| {
                let mut row: Row = HashMap::new();
                for (col_name, val) in col_names.iter().zip(tuple.data.iter()) {
                    let v = val.clone().unwrap_or(PgValue::Null);
                    // Both qualified (table.col) and unqualified (col) access
                    row.insert(format!("{prefix}.{col_name}"), v.clone());
                    row.insert(col_name.clone(), v.clone());
                }
                // Include ctid
                row.insert(format!("{prefix}.ctid"), PgValue::Int8(tuple.ctid as i64));
                row.insert("ctid".to_string(), PgValue::Int8(tuple.ctid as i64));
                row
            }).collect();

            Ok(rows)
        }

        TableFactor::Derived { subquery, alias, .. } => {
            let rows = exec_set_expr(exec, &subquery.body, ctes)?;
            let alias_name = alias.as_ref().map(|a| a.name.value.to_lowercase()).unwrap_or_else(|| "subquery".to_string());
            Ok(rows.into_iter().map(|row| {
                row.into_iter().flat_map(|(k, v)| {
                    vec![(format!("{alias_name}.{k}"), v.clone()), (k, v)]
                }).collect()
            }).collect())
        }

        TableFactor::Function { name, args, alias, .. } => {
            let fn_name = name.to_string().to_lowercase();
            let ctx = EvalContext::new();
            let arg_vals: Vec<PgValue> = args.iter().map(|a| match a {
                ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(e)) => eval_expr(e, &ctx),
                _ => Ok(PgValue::Null),
            }).collect::<Result<_>>()?;

            let result = crate::functions::call(&fn_name, arg_vals)?;
            let alias_name = alias.as_ref().map(|a| a.name.value.to_lowercase()).unwrap_or(fn_name.clone());

            match result {
                PgValue::Array { elements, .. } => {
                    Ok(elements.into_iter().map(|v| {
                        let mut row: Row = HashMap::new();
                        row.insert(alias_name.clone(), v);
                        row
                    }).collect())
                }
                PgValue::Null => Ok(vec![]),
                v => {
                    let mut row: Row = HashMap::new();
                    row.insert(alias_name, v);
                    Ok(vec![row])
                }
            }
        }

        TableFactor::TableFunction { expr, alias } => {
            let ctx = EvalContext::new();
            let result = eval_expr(expr, &ctx)?;
            let alias_name = alias.as_ref().map(|a| a.name.value.to_lowercase()).unwrap_or_else(|| "func".to_string());
            match result {
                PgValue::Array { elements, .. } => {
                    Ok(elements.into_iter().map(|v| {
                        let mut row: Row = HashMap::new();
                        row.insert(alias_name.clone(), v);
                        row
                    }).collect())
                }
                _ => Ok(vec![]),
            }
        }

        TableFactor::Pivot { .. } | TableFactor::Unpivot { .. } => {
            Err(Error::Pg(PgError::feature_not_supported("PIVOT/UNPIVOT")))
        }

        TableFactor::MatchRecognize { .. } => {
            Err(Error::Pg(PgError::feature_not_supported("MATCH_RECOGNIZE")))
        }

        TableFactor::UNNEST { array_exprs, alias, .. } => {
            let ctx = EvalContext::new();
            let mut all_arrays: Vec<Vec<PgValue>> = Vec::new();
            for expr in array_exprs {
                let v = eval_expr(expr, &ctx)?;
                match v {
                    PgValue::Array { elements, .. } => all_arrays.push(elements),
                    PgValue::Null => all_arrays.push(vec![]),
                    v => all_arrays.push(vec![v]),
                }
            }
            let max_len = all_arrays.iter().map(|a| a.len()).max().unwrap_or(0);
            let alias_name = alias.as_ref().map(|a| a.name.value.to_lowercase()).unwrap_or_else(|| "unnest".to_string());
            Ok((0..max_len).map(|i| {
                let mut row: Row = HashMap::new();
                for (j, arr) in all_arrays.iter().enumerate() {
                    let v = arr.get(i).cloned().unwrap_or(PgValue::Null);
                    row.insert(if j == 0 { alias_name.clone() } else { format!("{alias_name}{j}") }, v);
                }
                row
            }).collect())
        }

        _ => Err(Error::Pg(PgError::feature_not_supported("table factor"))),
    }
}

fn apply_join(left: Vec<Row>, right: Vec<Row>, join_op: &JoinOperator) -> Result<Vec<Row>> {
    match join_op {
        JoinOperator::CrossJoin => {
            let mut result = Vec::with_capacity(left.len() * right.len());
            for lr in &left {
                for rr in &right {
                    let mut row = lr.clone();
                    row.extend(rr.clone());
                    result.push(row);
                }
            }
            Ok(result)
        }
        JoinOperator::Inner(constraint) |
        JoinOperator::LeftOuter(constraint) |
        JoinOperator::RightOuter(constraint) |
        JoinOperator::FullOuter(constraint) => {
            let is_left = matches!(join_op, JoinOperator::LeftOuter(_) | JoinOperator::FullOuter(_));
            let is_right = matches!(join_op, JoinOperator::RightOuter(_) | JoinOperator::FullOuter(_));
            let mut result = Vec::new();
            let mut right_matched = vec![false; right.len()];

            for lr in &left {
                let mut found = false;
                for (ri, rr) in right.iter().enumerate() {
                    let mut combined = lr.clone();
                    combined.extend(rr.clone());
                    let matches = match constraint {
                        JoinConstraint::On(cond) => {
                            let ctx = EvalContext::with_row(combined.clone());
                            eval_expr(cond, &ctx).map(|v| v.is_true()).unwrap_or(false)
                        }
                        JoinConstraint::Using(cols) => {
                            cols.iter().all(|col| {
                                lr.get(&col.value.to_lowercase()) == rr.get(&col.value.to_lowercase())
                            })
                        }
                        JoinConstraint::Natural | JoinConstraint::None => true,
                    };
                    if matches {
                        result.push(combined);
                        right_matched[ri] = true;
                        found = true;
                    }
                }
                if !found && is_left {
                    // Add left row with NULLs for right columns
                    let mut row = lr.clone();
                    for (k, _) in right.first().unwrap_or(&HashMap::new()) {
                        row.entry(k.clone()).or_insert(PgValue::Null);
                    }
                    result.push(row);
                }
            }
            if is_right {
                for (ri, rr) in right.iter().enumerate() {
                    if !right_matched[ri] {
                        let mut row = rr.clone();
                        for (k, _) in left.first().unwrap_or(&HashMap::new()) {
                            row.entry(k.clone()).or_insert(PgValue::Null);
                        }
                        result.push(row);
                    }
                }
            }
            Ok(result)
        }
        _ => Err(Error::Pg(PgError::feature_not_supported("join type"))),
    }
}

fn apply_group_by_and_agg(exec: &mut Executor, select: &Select, rows: Vec<Row>) -> Result<Vec<Row>> {
    // Determine group keys
    let group_keys: Vec<&Expr> = match &select.group_by {
        GroupByExpr::All(_) => vec![],
        GroupByExpr::Expressions(exprs, _) => exprs.iter().collect(),
    };

    // Group rows
    let mut groups: Vec<(Vec<PgValue>, Vec<Row>)> = Vec::new();

    if group_keys.is_empty() && !has_group_by(select) {
        // Single aggregate group over all rows
        groups.push((vec![], rows));
    } else {
        for row in rows {
            let ctx = EvalContext::with_row(row.clone());
            let key: Vec<PgValue> = group_keys.iter()
                .map(|e| eval_expr(e, &ctx).unwrap_or(PgValue::Null))
                .collect();

            if let Some(group) = groups.iter_mut().find(|(k, _)| k == &key) {
                group.1.push(row);
            } else {
                groups.push((key, vec![row]));
            }
        }
    }

    // Apply HAVING filter and compute projections
    let mut result = Vec::new();
    for (key_vals, group_rows) in groups {
        // Build aggregate context
        let agg_ctx = compute_aggregates(&group_rows, &select.projection);

        // Check HAVING
        if let Some(having) = &select.having {
            let ctx = EvalContext::with_row(agg_ctx.clone());
            if !eval_expr(having, &ctx).map(|v| v.is_true()).unwrap_or(false) {
                continue;
            }
        }

        // Project
        let first_row = group_rows.first().cloned().unwrap_or_default();
        let mut merged = first_row;
        merged.extend(agg_ctx);

        let projected = project_row(exec, &select.projection, &merged)?;
        result.push(projected);
    }

    Ok(result)
}

fn compute_aggregates(rows: &[Row], projection: &[SelectItem]) -> Row {
    let mut result: Row = HashMap::new();

    // Collect all aggregate function calls in projection
    for item in projection {
        match item {
            SelectItem::ExprWithAlias { expr, alias } => {
                if let Some(agg_val) = try_compute_agg(rows, expr) {
                    result.insert(alias.value.to_lowercase(), agg_val);
                    result.insert(expr_name(expr), result.get(&alias.value.to_lowercase()).cloned().unwrap_or(PgValue::Null));
                }
            }
            SelectItem::UnnamedExpr(expr) => {
                if let Some(agg_val) = try_compute_agg(rows, expr) {
                    result.insert(expr_name(expr), agg_val);
                }
            }
            _ => {}
        }
    }
    result
}

fn fn_args(args: &FunctionArguments) -> &[ast::FunctionArg] {
    match args {
        FunctionArguments::List(list) => &list.args,
        _ => &[],
    }
}

fn try_compute_agg(rows: &[Row], expr: &Expr) -> Option<PgValue> {
    match expr {
        Expr::Function(f) => {
            let fn_name = f.name.to_string().to_lowercase();
            let fargs = fn_args(&f.args);
            let arg_expr = fargs.first().map(|a| match a {
                ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(e)) => Some(e),
                ast::FunctionArg::Named { arg: ast::FunctionArgExpr::Expr(e), .. } => Some(e),
                _ => None,
            }).flatten();

            let is_wildcard = fargs.first().map(|a| matches!(a,
                ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Wildcard)
            )).unwrap_or(false);

            match fn_name.as_str() {
                "count" => {
                    if is_wildcard {
                        return Some(PgValue::Int8(rows.len() as i64));
                    }
                    if let Some(arg) = arg_expr {
                        let count = rows.iter().filter(|row| {
                            let ctx = EvalContext::with_row((*row).clone());
                            !eval_expr(arg, &ctx).unwrap_or(PgValue::Null).is_null()
                        }).count();
                        return Some(PgValue::Int8(count as i64));
                    }
                    Some(PgValue::Int8(rows.len() as i64))
                }
                "sum" => {
                    if let Some(arg) = arg_expr {
                        let mut sum = 0f64;
                        let mut has_value = false;
                        for row in rows {
                            let ctx = EvalContext::with_row(row.clone());
                            if let Ok(v) = eval_expr(arg, &ctx) {
                                if let Some(f) = v.to_f64() {
                                    sum += f;
                                    has_value = true;
                                }
                            }
                        }
                        return Some(if has_value { PgValue::Float8(sum) } else { PgValue::Null });
                    }
                    None
                }
                "avg" => {
                    if let Some(arg) = arg_expr {
                        let mut sum = 0f64;
                        let mut count = 0i64;
                        for row in rows {
                            let ctx = EvalContext::with_row(row.clone());
                            if let Ok(v) = eval_expr(arg, &ctx) {
                                if let Some(f) = v.to_f64() {
                                    sum += f;
                                    count += 1;
                                }
                            }
                        }
                        return Some(if count > 0 { PgValue::Float8(sum / count as f64) } else { PgValue::Null });
                    }
                    None
                }
                "min" => {
                    if let Some(arg) = arg_expr {
                        let mut min_val: Option<PgValue> = None;
                        for row in rows {
                            let ctx = EvalContext::with_row(row.clone());
                            if let Ok(v) = eval_expr(arg, &ctx) {
                                if !v.is_null() {
                                    min_val = Some(match &min_val {
                                        None => v,
                                        Some(curr) => if v.compare(curr) == Some(std::cmp::Ordering::Less) { v } else { curr.clone() },
                                    });
                                }
                            }
                        }
                        return Some(min_val.unwrap_or(PgValue::Null));
                    }
                    None
                }
                "max" => {
                    if let Some(arg) = arg_expr {
                        let mut max_val: Option<PgValue> = None;
                        for row in rows {
                            let ctx = EvalContext::with_row(row.clone());
                            if let Ok(v) = eval_expr(arg, &ctx) {
                                if !v.is_null() {
                                    max_val = Some(match &max_val {
                                        None => v,
                                        Some(curr) => if v.compare(curr) == Some(std::cmp::Ordering::Greater) { v } else { curr.clone() },
                                    });
                                }
                            }
                        }
                        return Some(max_val.unwrap_or(PgValue::Null));
                    }
                    None
                }
                "string_agg" => {
                    if fargs.len() >= 2 {
                        let arg = match &fargs[0] {
                            ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(e)) => e,
                            _ => return None,
                        };
                        let delim = match &fargs[1] {
                            ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(e)) => {
                                let ctx = EvalContext::new();
                                eval_expr(e, &ctx).unwrap_or(PgValue::Text(",".to_string())).to_text()
                            }
                            _ => ",".to_string(),
                        };
                        let parts: Vec<String> = rows.iter().filter_map(|row| {
                            let ctx = EvalContext::with_row(row.clone());
                            eval_expr(arg, &ctx).ok().filter(|v| !v.is_null()).map(|v| v.to_text())
                        }).collect();
                        return Some(PgValue::Text(parts.join(&delim)));
                    }
                    None
                }
                "array_agg" => {
                    if let Some(arg) = arg_expr {
                        let elements: Vec<PgValue> = rows.iter().filter_map(|row| {
                            let ctx = EvalContext::with_row(row.clone());
                            eval_expr(arg, &ctx).ok()
                        }).collect();
                        let elem_oid = elements.first().map(|v| v.oid()).unwrap_or(crate::types::oid::TEXT);
                        return Some(PgValue::Array { element_oid: elem_oid, elements });
                    }
                    None
                }
                "json_agg" | "jsonb_agg" => {
                    if let Some(arg) = arg_expr {
                        let elements: Vec<serde_json::Value> = rows.iter().filter_map(|row| {
                            let ctx = EvalContext::with_row(row.clone());
                            eval_expr(arg, &ctx).ok().map(|v| match v {
                                PgValue::Json(j) | PgValue::Jsonb(j) => j,
                                other => serde_json::Value::String(other.to_text()),
                            })
                        }).collect();
                        return Some(PgValue::Jsonb(serde_json::Value::Array(elements)));
                    }
                    None
                }
                "bool_and" | "every" => {
                    if let Some(arg) = arg_expr {
                        let result = rows.iter().all(|row| {
                            let ctx = EvalContext::with_row(row.clone());
                            eval_expr(arg, &ctx).map(|v| v.is_true()).unwrap_or(false)
                        });
                        return Some(PgValue::Bool(result));
                    }
                    None
                }
                "bool_or" => {
                    if let Some(arg) = arg_expr {
                        let result = rows.iter().any(|row| {
                            let ctx = EvalContext::with_row(row.clone());
                            eval_expr(arg, &ctx).map(|v| v.is_true()).unwrap_or(false)
                        });
                        return Some(PgValue::Bool(result));
                    }
                    None
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn project_rows(exec: &mut Executor, select: &Select, rows: Vec<Row>) -> Result<Vec<Row>> {
    rows.into_iter().map(|row| project_row(exec, &select.projection, &row)).collect()
}

fn project_row(exec: &mut Executor, projection: &[SelectItem], row: &Row) -> Result<Row> {
    let mut result: Row = HashMap::new();
    for item in projection {
        match item {
            SelectItem::UnnamedExpr(expr) => {
                let ctx = EvalContext::with_row(row.clone());
                let val = eval_expr_with_sequences(expr, &ctx, exec)?;
                let name = expr_name(expr);
                result.insert(name, val);
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                let ctx = EvalContext::with_row(row.clone());
                let val = eval_expr_with_sequences(expr, &ctx, exec)?;
                result.insert(alias.value.to_lowercase(), val);
            }
            SelectItem::Wildcard(_) => {
                // Include all columns from row
                for (k, v) in row {
                    if !k.contains('.') {
                        result.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
            SelectItem::QualifiedWildcard(name, _) => {
                let prefix = name.to_string().to_lowercase();
                for (k, v) in row {
                    if let Some(col) = k.strip_prefix(&format!("{prefix}.")) {
                        result.insert(col.to_string(), v.clone());
                    }
                }
            }
        }
    }
    Ok(result)
}

fn eval_expr_with_sequences(expr: &Expr, ctx: &EvalContext, exec: &mut Executor) -> Result<PgValue> {
    let v = eval_expr(expr, ctx)?;
    // Resolve sequence placeholders
    match &v {
        PgValue::Text(s) => {
            if let Some(seq_name) = s.strip_prefix("__nextval__") {
                let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no database")))?;
                let search_path = exec.config.search_path_refs();
                if let Some((schema_ref, actual_name)) = db.resolve_sequence(seq_name, &search_path) {
                    let seq = schema_ref.read().sequence(&actual_name).unwrap();
                    let next = seq.nextval()?;
                    exec.sequence_currval.insert(seq_name.to_string(), next);
                    return Ok(PgValue::Int8(next));
                }
                return Err(Error::Pg(PgError::error(SqlState::UNDEFINED_TABLE, format!("sequence \"{seq_name}\" does not exist"))));
            }
            if let Some(seq_name) = s.strip_prefix("__currval__") {
                if let Some(&curr) = exec.sequence_currval.get(seq_name) {
                    return Ok(PgValue::Int8(curr));
                }
                return Err(Error::Pg(PgError::error(SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
                    format!("currval of sequence \"{seq_name}\" is not yet defined in this session"))));
            }
            Ok(v)
        }
        _ => Ok(v),
    }
}

fn apply_order_by(rows: &mut Vec<Row>, order_by: &[OrderByExpr]) -> Result<()> {
    rows.sort_by(|a, b| {
        for ord in order_by {
            let ctx_a = EvalContext::with_row(a.clone());
            let ctx_b = EvalContext::with_row(b.clone());
            let av = eval_expr(&ord.expr, &ctx_a).unwrap_or(PgValue::Null);
            let bv = eval_expr(&ord.expr, &ctx_b).unwrap_or(PgValue::Null);

            // NULL ordering
            let (an, bn) = (av.is_null(), bv.is_null());
            let nulls_first = ord.nulls_first.unwrap_or(false);
            if an && bn { continue; }
            if an { return if nulls_first { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }; }
            if bn { return if nulls_first { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Less }; }

            if let Some(ord_result) = av.compare(&bv) {
                let result = if ord.asc.unwrap_or(true) { ord_result } else { ord_result.reverse() };
                if result != std::cmp::Ordering::Equal { return result; }
            }
        }
        std::cmp::Ordering::Equal
    });
    Ok(())
}

fn dedup_rows(rows: Vec<Row>) -> Vec<Row> {
    let mut seen: Vec<Vec<String>> = Vec::new();
    let mut result = Vec::new();
    for row in rows {
        let key: Vec<String> = {
            let mut keys: Vec<&String> = row.keys().collect();
            keys.sort();
            keys.iter().map(|k| format!("{}={}", k, row.get(*k).map(|v| v.to_text()).unwrap_or_default())).collect()
        };
        if !seen.contains(&key) {
            seen.push(key);
            result.push(row);
        }
    }
    result
}

fn rows_equal(a: &Row, b: &Row) -> bool {
    let a_key: Vec<String> = {
        let mut keys: Vec<&String> = a.keys().collect();
        keys.sort();
        keys.iter().map(|k| format!("{}={}", k, a.get(*k).map(|v| v.to_text()).unwrap_or_default())).collect()
    };
    let b_key: Vec<String> = {
        let mut keys: Vec<&String> = b.keys().collect();
        keys.sort();
        keys.iter().map(|k| format!("{}={}", k, b.get(*k).map(|v| v.to_text()).unwrap_or_default())).collect()
    };
    a_key == b_key
}

fn expr_name(expr: &Expr) -> String {
    match expr {
        Expr::Identifier(i) => i.value.to_lowercase(),
        Expr::CompoundIdentifier(parts) => parts.last().map(|p| p.value.to_lowercase()).unwrap_or_default(),
        Expr::Function(f) => f.name.to_string().to_lowercase(),
        Expr::Value(_) => "?column?".to_string(),
        Expr::BinaryOp { .. } => "?column?".to_string(),
        Expr::Cast { data_type, .. } => data_type.to_string().to_lowercase(),
        _ => "?column?".to_string(),
    }
}

fn has_agg_in_projection(item: &SelectItem) -> bool {
    match item {
        SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => expr_has_agg(e),
        _ => false,
    }
}

fn expr_has_agg(expr: &Expr) -> bool {
    match expr {
        Expr::Function(f) => {
            let name = f.name.to_string().to_lowercase();
            matches!(name.as_str(), "count" | "sum" | "avg" | "min" | "max" | "string_agg" | "array_agg" | "json_agg" | "jsonb_agg" | "bool_and" | "bool_or" | "every")
        }
        Expr::BinaryOp { left, right, .. } => expr_has_agg(left) || expr_has_agg(right),
        Expr::Case { conditions, results, else_result, .. } => {
            conditions.iter().any(expr_has_agg) || results.iter().any(expr_has_agg) ||
            else_result.as_ref().map(|e| expr_has_agg(e)).unwrap_or(false)
        }
        _ => false,
    }
}

fn has_group_by(select: &Select) -> bool {
    match &select.group_by {
        GroupByExpr::All(_) => true,
        GroupByExpr::Expressions(exprs, _) => !exprs.is_empty(),
    }
}

fn is_empty_group_by(group_by: &GroupByExpr) -> bool {
    match group_by {
        GroupByExpr::All(_) => false,
        GroupByExpr::Expressions(exprs, _) => exprs.is_empty(),
    }
}

fn result_set_to_rows(rs: &ResultSet, table_alias: &str) -> Vec<Row> {
    let col_names: Vec<String> = rs.columns.iter().map(|c| c.name.clone()).collect();
    rs.rows.iter().map(|row| {
        let mut map: Row = HashMap::new();
        for (col, val) in col_names.iter().zip(row.iter()) {
            let v = val.clone();
            map.insert(format!("{table_alias}.{col}"), v.clone());
            map.insert(col.clone(), v);
        }
        map
    }).collect()
}

// Extension trait for TransactionManager to expose xid counter
trait TxnManagerExt {
    fn xid_counter_value(&self) -> crate::storage::mvcc::Xid;
}

impl TxnManagerExt for crate::storage::mvcc::TransactionManager {
    fn xid_counter_value(&self) -> crate::storage::mvcc::Xid {
        use std::sync::atomic::Ordering;
        // Access the atomic counter through the public API
        self.statement_snapshot(0).xmax
    }
}

