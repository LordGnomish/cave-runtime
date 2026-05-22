// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! INSERT statement execution.

use crate::sql::ast::{Expr, InsertStmt, Literal, OnConflictAction, SelectColumn};
use crate::storage::schema::{Database, Row, Table};
use crate::types::SqlValue;

pub fn execute_insert(insert: &InsertStmt, db: &mut Database) -> Result<u64, String> {
    let (count, _rows) = execute_insert_inner(insert, db, false)?;
    Ok(count)
}

/// INSERT … RETURNING — Postgres-compat extension that returns the affected
/// rows projected through the RETURNING list.
///
/// Maps to `ExecInsert` + `ExecProcessReturning` in postgres'
/// `src/backend/executor/nodeModifyTable.c`. Returns (affected_count, rows).
pub fn execute_insert_returning(
    insert: &InsertStmt,
    db: &mut Database,
) -> Result<(u64, Vec<Row>), String> {
    execute_insert_inner(insert, db, true)
}

fn execute_insert_inner(
    insert: &InsertStmt,
    db: &mut Database,
    capture_returning: bool,
) -> Result<(u64, Vec<Row>), String> {
    let schema = db.schemas.get_mut("public").ok_or("no public schema")?;
    let table = schema
        .tables
        .get_mut(&insert.table)
        .ok_or(format!("table {} not found", insert.table))?;

    // ── Conflict target resolution ───────────────────────────────────────────
    let conflict_cols: Vec<usize> = match &insert.on_conflict {
        Some(action) => {
            let target = match action {
                OnConflictAction::DoNothing { target } => target.as_ref(),
                OnConflictAction::DoUpdate { target, .. } => target.as_ref(),
            };
            if let Some(cols) = target {
                cols.iter()
                    .map(|n| {
                        table
                            .column_index(n)
                            .ok_or_else(|| format!("ON CONFLICT column {} not found", n))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                // Default: every column flagged primary_key.
                table
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.primary_key)
                    .map(|(i, _)| i)
                    .collect()
            }
        }
        None => Vec::new(),
    };

    let mut affected = 0u64;
    let mut returning_rows: Vec<Row> = Vec::new();
    for row_values in &insert.values {
        let mut new_row = Vec::with_capacity(row_values.len());
        for val_expr in row_values {
            new_row.push(literal_to_value(val_expr)?);
        }

        // Conflict detection — find an existing row whose conflict-key matches.
        let conflict_idx = if !conflict_cols.is_empty() {
            find_conflict_row(table, &new_row, &conflict_cols)
        } else {
            None
        };

        match (conflict_idx, &insert.on_conflict) {
            (Some(idx), Some(OnConflictAction::DoNothing { .. })) => {
                if capture_returning && insert.returning.is_some() {
                    // RETURNING on a no-op INSERT returns nothing for that row.
                }
                let _ = idx;
            }
            (Some(idx), Some(OnConflictAction::DoUpdate { assignments, .. })) => {
                for (col_name, expr) in assignments {
                    if let Some(ci) = table.column_index(col_name) {
                        let v = literal_to_value(expr)?;
                        if let Some(row) = table.rows.get_mut(idx) {
                            if ci < row.len() {
                                row[ci] = v;
                            }
                        }
                    }
                }
                affected += 1;
                if capture_returning {
                    if let Some(row) = table.rows.get(idx) {
                        returning_rows.push(
                            project_returning(insert.returning.as_deref(), table, row)?,
                        );
                    }
                }
            }
            _ => {
                table.rows.push(new_row.clone());
                affected += 1;
                if capture_returning {
                    returning_rows.push(project_returning(
                        insert.returning.as_deref(),
                        table,
                        &new_row,
                    )?);
                }
            }
        }
    }
    Ok((affected, returning_rows))
}

fn find_conflict_row(table: &Table, new_row: &[SqlValue], cols: &[usize]) -> Option<usize> {
    table.rows.iter().position(|existing| {
        cols.iter().all(|&i| {
            existing
                .get(i)
                .zip(new_row.get(i))
                .is_some_and(|(a, b)| values_equal(a, b))
        })
    })
}

fn values_equal(a: &SqlValue, b: &SqlValue) -> bool {
    match (a, b) {
        (SqlValue::Null, SqlValue::Null) => false, // PG semantics: NULL never conflicts
        (SqlValue::Int4(x), SqlValue::Int4(y)) => x == y,
        (SqlValue::Int8(x), SqlValue::Int8(y)) => x == y,
        (SqlValue::Text(x), SqlValue::Text(y)) => x == y,
        (SqlValue::Bool(x), SqlValue::Bool(y)) => x == y,
        (SqlValue::Numeric(x), SqlValue::Numeric(y)) => (x - y).abs() < f64::EPSILON,
        (SqlValue::Date(x), SqlValue::Date(y)) => x == y,
        (SqlValue::Timestamp(x), SqlValue::Timestamp(y)) => x == y,
        _ => false,
    }
}

pub(crate) fn literal_to_value(expr: &Expr) -> Result<SqlValue, String> {
    match expr {
        Expr::Literal(lit) => Ok(match lit {
            Literal::Null => SqlValue::Null,
            Literal::Integer(n) => SqlValue::Int4(*n as i32),
            Literal::Float(f) => SqlValue::Numeric(*f),
            Literal::String(s) => SqlValue::Text(s.clone()),
            Literal::Boolean(b) => SqlValue::Bool(*b),
            Literal::Date(s) => SqlValue::Date(s.clone()),
            Literal::Timestamp(s) => SqlValue::Timestamp(s.clone()),
        }),
        _ => Err("complex expressions not yet supported in this context".to_string()),
    }
}

pub(crate) fn project_returning(
    returning: Option<&[SelectColumn]>,
    table: &Table,
    row: &Row,
) -> Result<Row, String> {
    let Some(cols) = returning else {
        return Ok(row.clone());
    };
    let mut out = Vec::with_capacity(cols.len());
    for col in cols {
        match col {
            SelectColumn::Star => return Ok(row.clone()),
            SelectColumn::TableStar(_) => return Ok(row.clone()),
            SelectColumn::Expr(Expr::Identifier(name), _) => {
                let idx = table
                    .column_index(name)
                    .ok_or_else(|| format!("RETURNING column {} not found", name))?;
                out.push(row.get(idx).cloned().unwrap_or(SqlValue::Null));
            }
            SelectColumn::Expr(Expr::Literal(lit), _) => out.push(literal_to_value(&Expr::Literal(lit.clone()))?),
            _ => return Err("only column references and literals allowed in RETURNING".to_string()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql::ast::Expr;
    use crate::storage::schema::ColumnDef;

    fn make_users_table(db: &mut Database) {
        let schema = db.schemas.get_mut("public").unwrap();
        let cols = vec![
            ColumnDef {
                name: "id".to_string(),
                type_name: "int".to_string(),
                not_null: true,
                primary_key: true,
            },
            ColumnDef {
                name: "name".to_string(),
                type_name: "text".to_string(),
                not_null: false,
                primary_key: false,
            },
        ];
        let table = crate::storage::schema::Table::new("users", cols);
        schema.tables.insert("users".to_string(), table);
    }

    #[test]
    fn test_insert_simple() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        let insert = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(1)),
                Expr::Literal(Literal::String("alice".into())),
            ]],
            on_conflict: None,
            returning: None,
        };
        let result = execute_insert(&insert, &mut db);
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_insert_returning_star() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        let insert = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(1)),
                Expr::Literal(Literal::String("alice".into())),
            ]],
            on_conflict: None,
            returning: Some(vec![SelectColumn::Star]),
        };
        let (count, rows) = execute_insert_returning(&insert, &mut db).unwrap();
        assert_eq!(count, 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 2);
        assert!(matches!(rows[0][0], SqlValue::Int4(1)));
    }

    #[test]
    fn test_insert_returning_specific_column() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        let insert = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(42)),
                Expr::Literal(Literal::String("bob".into())),
            ]],
            on_conflict: None,
            returning: Some(vec![SelectColumn::Expr(
                Expr::Identifier("id".to_string()),
                None,
            )]),
        };
        let (count, rows) = execute_insert_returning(&insert, &mut db).unwrap();
        assert_eq!(count, 1);
        assert_eq!(rows[0].len(), 1);
        assert!(matches!(rows[0][0], SqlValue::Int4(42)));
    }

    #[test]
    fn test_on_conflict_do_nothing() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        let insert1 = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(1)),
                Expr::Literal(Literal::String("alice".into())),
            ]],
            on_conflict: None,
            returning: None,
        };
        execute_insert(&insert1, &mut db).unwrap();

        // Conflict by PK id — DO NOTHING.
        let insert2 = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(1)),
                Expr::Literal(Literal::String("ALICE-DUP".into())),
            ]],
            on_conflict: Some(OnConflictAction::DoNothing {
                target: Some(vec!["id".to_string()]),
            }),
            returning: None,
        };
        let n = execute_insert(&insert2, &mut db).unwrap();
        assert_eq!(n, 0);
        let table = db.schemas["public"].tables.get("users").unwrap();
        assert_eq!(table.rows.len(), 1);
        // Original name should be preserved.
        assert!(matches!(&table.rows[0][1], SqlValue::Text(s) if s == "alice"));
    }

    #[test]
    fn test_on_conflict_do_update() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        let insert1 = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(7)),
                Expr::Literal(Literal::String("old".into())),
            ]],
            on_conflict: None,
            returning: None,
        };
        execute_insert(&insert1, &mut db).unwrap();

        let insert2 = InsertStmt {
            table: "users".to_string(),
            columns: None,
            values: vec![vec![
                Expr::Literal(Literal::Integer(7)),
                Expr::Literal(Literal::String("new".into())),
            ]],
            on_conflict: Some(OnConflictAction::DoUpdate {
                target: Some(vec!["id".to_string()]),
                assignments: vec![(
                    "name".to_string(),
                    Expr::Literal(Literal::String("new".into())),
                )],
            }),
            returning: None,
        };
        let n = execute_insert(&insert2, &mut db).unwrap();
        assert_eq!(n, 1);
        let table = db.schemas["public"].tables.get("users").unwrap();
        assert_eq!(table.rows.len(), 1);
        assert!(matches!(&table.rows[0][1], SqlValue::Text(s) if s == "new"));
    }

    #[test]
    fn test_on_conflict_default_target_is_primary_key() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        execute_insert(
            &InsertStmt {
                table: "users".to_string(),
                columns: None,
                values: vec![vec![
                    Expr::Literal(Literal::Integer(3)),
                    Expr::Literal(Literal::String("a".into())),
                ]],
                on_conflict: None,
                returning: None,
            },
            &mut db,
        )
        .unwrap();

        // No target column listed — default to PK columns.
        let n = execute_insert(
            &InsertStmt {
                table: "users".to_string(),
                columns: None,
                values: vec![vec![
                    Expr::Literal(Literal::Integer(3)),
                    Expr::Literal(Literal::String("b".into())),
                ]],
                on_conflict: Some(OnConflictAction::DoNothing { target: None }),
                returning: None,
            },
            &mut db,
        )
        .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_returning_after_on_conflict_do_update() {
        let mut db = Database::new("test");
        make_users_table(&mut db);

        execute_insert(
            &InsertStmt {
                table: "users".to_string(),
                columns: None,
                values: vec![vec![
                    Expr::Literal(Literal::Integer(5)),
                    Expr::Literal(Literal::String("old".into())),
                ]],
                on_conflict: None,
                returning: None,
            },
            &mut db,
        )
        .unwrap();

        let (count, rows) = execute_insert_returning(
            &InsertStmt {
                table: "users".to_string(),
                columns: None,
                values: vec![vec![
                    Expr::Literal(Literal::Integer(5)),
                    Expr::Literal(Literal::String("fresh".into())),
                ]],
                on_conflict: Some(OnConflictAction::DoUpdate {
                    target: Some(vec!["id".to_string()]),
                    assignments: vec![(
                        "name".to_string(),
                        Expr::Literal(Literal::String("fresh".into())),
                    )],
                }),
                returning: Some(vec![SelectColumn::Star]),
            },
            &mut db,
        )
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0][1], SqlValue::Text(s) if s == "fresh"));
    }
}
