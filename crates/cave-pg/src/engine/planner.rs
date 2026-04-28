//! sqlparser AST → engine `Plan`.
//!
//! Subset supported (extends incrementally as needed):
//!
//! * `CREATE TABLE` (with column definitions, NOT NULL, PRIMARY KEY, DEFAULT)
//! * `DROP TABLE [IF EXISTS]`
//! * `INSERT INTO ... VALUES (...), (...)`
//! * `SELECT cols FROM table [WHERE pred] [ORDER BY ...] [LIMIT n]`
//! * `UPDATE table SET col = val [WHERE pred]`
//! * `DELETE FROM table [WHERE pred]`
//! * `BEGIN | COMMIT | ROLLBACK`

use sqlparser::ast::{
    BinaryOperator, ColumnOption, Expr as SqlExpr, OrderByExpr, Query, SelectItem, SetExpr,
    Statement, TableFactor, Value as SqlValue,
};

use super::catalog::{Catalog, ColumnDef};
use super::value::{ScalarType, ScalarValue};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlanError {
    #[error("unsupported statement: {0}")]
    Unsupported(String),
    #[error("invalid column type: {0}")]
    InvalidColumnType(String),
    #[error("table not found: {0}")]
    TableNotFound(String),
    #[error("column not found: {0}")]
    ColumnNotFound(String),
    #[error("invalid literal: {0}")]
    InvalidLiteral(String),
    #[error("type mismatch")]
    TypeMismatch,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    CreateTable {
        schema: String,
        name: String,
        columns: Vec<ColumnDef>,
        if_not_exists: bool,
    },
    DropTable {
        schema: String,
        name: String,
        if_exists: bool,
    },
    Insert {
        schema: String,
        table: String,
        columns: Option<Vec<String>>,
        rows: Vec<Vec<PlanExpr>>,
    },
    Select {
        from: TableRef,
        projection: Vec<Projection>,
        filter: Option<PlanExpr>,
        order_by: Vec<(usize, bool)>, // (column_index, ascending)
        limit: Option<usize>,
        offset: Option<usize>,
    },
    Update {
        schema: String,
        table: String,
        assignments: Vec<(String, PlanExpr)>,
        filter: Option<PlanExpr>,
    },
    Delete {
        schema: String,
        table: String,
        filter: Option<PlanExpr>,
    },
    Begin,
    Commit,
    Rollback,
    /// `SELECT 1`-style constant expressions with no FROM.
    SelectConst(Vec<(String, ScalarValue)>),
    SetParameter {
        name: String,
        value: String,
    },
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRef {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Projection {
    Star,
    Column(String),
    /// Aliased literal — used in `SELECT 1 AS x`.
    Literal { name: String, value: ScalarValue },
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanExpr {
    Column(String),
    Literal(ScalarValue),
    Binary {
        left: Box<PlanExpr>,
        op: PlanOp,
        right: Box<PlanExpr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

pub fn plan_statement(stmt: Statement, catalog: &Catalog) -> Result<Plan, PlanError> {
    match stmt {
        Statement::CreateTable(create) => plan_create_table(create),
        Statement::Drop {
            object_type,
            if_exists,
            mut names,
            ..
        } => {
            if !matches!(object_type, sqlparser::ast::ObjectType::Table) {
                return Err(PlanError::Unsupported("DROP non-table".into()));
            }
            let name = names.remove(0);
            let (schema, table) = split_qualified(&name.0);
            Ok(Plan::DropTable {
                schema,
                name: table,
                if_exists,
            })
        }
        Statement::Insert(ins) => plan_insert(ins),
        Statement::Query(q) => plan_query(*q, catalog),
        Statement::Update {
            table,
            assignments,
            selection,
            ..
        } => plan_update(table, assignments, selection),
        Statement::Delete(del) => plan_delete(del),
        Statement::StartTransaction { .. } => Ok(Plan::Begin),
        Statement::Commit { .. } => Ok(Plan::Commit),
        Statement::Rollback { .. } => Ok(Plan::Rollback),
        Statement::SetVariable { variables, value, .. } => {
            let name = match variables {
                sqlparser::ast::OneOrManyWithParens::One(v) => v.to_string(),
                sqlparser::ast::OneOrManyWithParens::Many(v) => v
                    .first()
                    .map(|v| v.to_string())
                    .ok_or_else(|| PlanError::Unsupported("SET without name".into()))?,
            };
            let value_str = value
                .first()
                .map(|v| v.to_string())
                .ok_or_else(|| PlanError::Unsupported("SET without value".into()))?;
            Ok(Plan::SetParameter {
                name,
                value: value_str,
            })
        }
        other => Err(PlanError::Unsupported(format!("{:?}", other))),
    }
}

fn plan_create_table(create: sqlparser::ast::CreateTable) -> Result<Plan, PlanError> {
    let (schema, name) = split_qualified(&create.name.0);
    let mut columns = Vec::with_capacity(create.columns.len());
    for col in create.columns {
        let ty = ScalarType::parse_sql_type(&col.data_type.to_string())
            .ok_or_else(|| PlanError::InvalidColumnType(col.data_type.to_string()))?;
        let mut nullable = true;
        let mut primary_key = false;
        let mut default_text = None;
        for opt in col.options {
            match opt.option {
                ColumnOption::NotNull => nullable = false,
                ColumnOption::Unique { is_primary, .. } if is_primary => {
                    primary_key = true;
                    nullable = false;
                }
                ColumnOption::Default(expr) => {
                    default_text = Some(expr.to_string());
                }
                _ => {}
            }
        }
        columns.push(ColumnDef {
            name: col.name.value,
            ty,
            nullable,
            primary_key,
            default_text,
        });
    }
    Ok(Plan::CreateTable {
        schema,
        name,
        columns,
        if_not_exists: create.if_not_exists,
    })
}

fn plan_insert(ins: sqlparser::ast::Insert) -> Result<Plan, PlanError> {
    let (schema, table) = split_qualified_table_obj(&ins.table_name);
    let columns = if ins.columns.is_empty() {
        None
    } else {
        Some(ins.columns.iter().map(|c| c.value.clone()).collect())
    };
    let mut rows = Vec::new();
    if let Some(source) = ins.source {
        if let SetExpr::Values(values) = *source.body {
            for row in values.rows {
                let mut cells = Vec::with_capacity(row.len());
                for cell in row {
                    cells.push(plan_expr(cell)?);
                }
                rows.push(cells);
            }
        } else {
            return Err(PlanError::Unsupported("INSERT only supports VALUES".into()));
        }
    }
    Ok(Plan::Insert {
        schema,
        table,
        columns,
        rows,
    })
}

fn plan_query(q: Query, catalog: &Catalog) -> Result<Plan, PlanError> {
    let body = *q.body;
    let select = if let SetExpr::Select(s) = body {
        *s
    } else {
        return Err(PlanError::Unsupported("only SELECT in queries".into()));
    };

    if select.from.is_empty() {
        let mut consts = Vec::new();
        for item in select.projection {
            match item {
                SelectItem::UnnamedExpr(e) => {
                    let value = eval_const(&plan_expr(e.clone())?)?;
                    consts.push((e.to_string(), value));
                }
                SelectItem::ExprWithAlias { expr, alias } => {
                    let value = eval_const(&plan_expr(expr)?)?;
                    consts.push((alias.value, value));
                }
                _ => return Err(PlanError::Unsupported("non-trivial select".into())),
            }
        }
        return Ok(Plan::SelectConst(consts));
    }

    if select.from.len() != 1 {
        return Err(PlanError::Unsupported("joins not implemented".into()));
    }
    let from = match &select.from[0].relation {
        TableFactor::Table { name, .. } => {
            let (schema, table) = split_qualified(&name.0);
            TableRef { schema, name: table }
        }
        _ => return Err(PlanError::Unsupported("subqueries/joins".into())),
    };

    let table_schema = catalog
        .lookup(&from.schema, &from.name)
        .map_err(|_| PlanError::TableNotFound(from.name.clone()))?;

    let mut projection = Vec::with_capacity(select.projection.len());
    for item in select.projection {
        match item {
            SelectItem::Wildcard(_) => projection.push(Projection::Star),
            SelectItem::UnnamedExpr(SqlExpr::Identifier(id)) => {
                projection.push(Projection::Column(id.value));
            }
            SelectItem::UnnamedExpr(e) => {
                let value = eval_const(&plan_expr(e.clone())?)?;
                projection.push(Projection::Literal {
                    name: e.to_string(),
                    value,
                });
            }
            SelectItem::ExprWithAlias { expr, alias } => match expr {
                SqlExpr::Identifier(id) => {
                    let _ = alias;
                    projection.push(Projection::Column(id.value));
                }
                e => {
                    let value = eval_const(&plan_expr(e)?)?;
                    projection.push(Projection::Literal {
                        name: alias.value,
                        value,
                    });
                }
            },
            SelectItem::QualifiedWildcard(_, _) => projection.push(Projection::Star),
        }
    }

    let filter = match select.selection {
        Some(e) => Some(plan_expr(e)?),
        None => None,
    };

    let order_by = match q.order_by {
        Some(ord) => {
            let mut out = Vec::new();
            for OrderByExpr { expr, asc, .. } in ord.exprs {
                if let SqlExpr::Identifier(id) = expr {
                    let idx = table_schema
                        .column_index(&id.value)
                        .ok_or_else(|| PlanError::ColumnNotFound(id.value.clone()))?;
                    out.push((idx, asc.unwrap_or(true)));
                } else {
                    return Err(PlanError::Unsupported("order by non-column".into()));
                }
            }
            out
        }
        None => Vec::new(),
    };

    let limit = match q.limit {
        Some(SqlExpr::Value(SqlValue::Number(n, _))) => Some(n.parse().unwrap_or(0)),
        _ => None,
    };
    let offset = match q.offset {
        Some(off) => match off.value {
            SqlExpr::Value(SqlValue::Number(n, _)) => Some(n.parse().unwrap_or(0)),
            _ => None,
        },
        None => None,
    };

    Ok(Plan::Select {
        from,
        projection,
        filter,
        order_by,
        limit,
        offset,
    })
}

fn plan_update(
    table: sqlparser::ast::TableWithJoins,
    assignments: Vec<sqlparser::ast::Assignment>,
    selection: Option<SqlExpr>,
) -> Result<Plan, PlanError> {
    let (schema, name) = match &table.relation {
        TableFactor::Table { name, .. } => split_qualified(&name.0),
        _ => return Err(PlanError::Unsupported("UPDATE non-table".into())),
    };
    let mut as_pairs = Vec::with_capacity(assignments.len());
    for a in assignments {
        let col_name = match a.target {
            sqlparser::ast::AssignmentTarget::ColumnName(obj) => obj
                .0
                .last()
                .map(|p| p.value.clone())
                .ok_or_else(|| PlanError::Unsupported("UPDATE empty column".into()))?,
            _ => return Err(PlanError::Unsupported("UPDATE compound target".into())),
        };
        as_pairs.push((col_name, plan_expr(a.value)?));
    }
    let filter = match selection {
        Some(e) => Some(plan_expr(e)?),
        None => None,
    };
    Ok(Plan::Update {
        schema,
        table: name,
        assignments: as_pairs,
        filter,
    })
}

fn plan_delete(del: sqlparser::ast::Delete) -> Result<Plan, PlanError> {
    let from_table = match del.from {
        sqlparser::ast::FromTable::WithFromKeyword(v)
        | sqlparser::ast::FromTable::WithoutKeyword(v) => v,
    };
    let first = from_table
        .into_iter()
        .next()
        .ok_or_else(|| PlanError::Unsupported("DELETE without target".into()))?;
    let (schema, table) = match &first.relation {
        TableFactor::Table { name, .. } => split_qualified(&name.0),
        _ => return Err(PlanError::Unsupported("DELETE non-table".into())),
    };
    let filter = match del.selection {
        Some(e) => Some(plan_expr(e)?),
        None => None,
    };
    Ok(Plan::Delete {
        schema,
        table,
        filter,
    })
}

fn plan_expr(expr: SqlExpr) -> Result<PlanExpr, PlanError> {
    match expr {
        SqlExpr::Identifier(id) => Ok(PlanExpr::Column(id.value)),
        SqlExpr::CompoundIdentifier(ids) => Ok(PlanExpr::Column(
            ids.last()
                .ok_or_else(|| PlanError::Unsupported("empty compound id".into()))?
                .value
                .clone(),
        )),
        SqlExpr::Value(v) => Ok(PlanExpr::Literal(value_to_scalar(v)?)),
        SqlExpr::UnaryOp { op, expr } => match op {
            sqlparser::ast::UnaryOperator::Minus => {
                let inner = plan_expr(*expr)?;
                if let PlanExpr::Literal(ScalarValue::Int4(n)) = inner {
                    Ok(PlanExpr::Literal(ScalarValue::Int4(-n)))
                } else if let PlanExpr::Literal(ScalarValue::Int8(n)) = inner {
                    Ok(PlanExpr::Literal(ScalarValue::Int8(-n)))
                } else {
                    Err(PlanError::Unsupported("unary minus on non-int".into()))
                }
            }
            _ => Err(PlanError::Unsupported(format!("unary {:?}", op))),
        },
        SqlExpr::BinaryOp { left, op, right } => {
            let l = plan_expr(*left)?;
            let r = plan_expr(*right)?;
            let pop = match op {
                BinaryOperator::Eq => PlanOp::Eq,
                BinaryOperator::NotEq => PlanOp::NotEq,
                BinaryOperator::Lt => PlanOp::Lt,
                BinaryOperator::LtEq => PlanOp::LtEq,
                BinaryOperator::Gt => PlanOp::Gt,
                BinaryOperator::GtEq => PlanOp::GtEq,
                BinaryOperator::And => PlanOp::And,
                BinaryOperator::Or => PlanOp::Or,
                other => return Err(PlanError::Unsupported(format!("op {:?}", other))),
            };
            Ok(PlanExpr::Binary {
                left: Box::new(l),
                op: pop,
                right: Box::new(r),
            })
        }
        SqlExpr::Nested(e) => plan_expr(*e),
        SqlExpr::IsNull(e) => Ok(PlanExpr::Binary {
            left: Box::new(plan_expr(*e)?),
            op: PlanOp::Eq,
            right: Box::new(PlanExpr::Literal(ScalarValue::Null)),
        }),
        other => Err(PlanError::Unsupported(format!("expr {:?}", other))),
    }
}

fn value_to_scalar(v: SqlValue) -> Result<ScalarValue, PlanError> {
    Ok(match v {
        SqlValue::Number(n, _) => {
            if let Ok(i) = n.parse::<i32>() {
                ScalarValue::Int4(i)
            } else if let Ok(i) = n.parse::<i64>() {
                ScalarValue::Int8(i)
            } else if let Ok(f) = n.parse::<f64>() {
                ScalarValue::Float8(f)
            } else {
                return Err(PlanError::InvalidLiteral(n));
            }
        }
        SqlValue::SingleQuotedString(s) | SqlValue::DoubleQuotedString(s) => ScalarValue::Text(s),
        SqlValue::Boolean(b) => ScalarValue::Bool(b),
        SqlValue::Null => ScalarValue::Null,
        other => return Err(PlanError::InvalidLiteral(format!("{:?}", other))),
    })
}

fn eval_const(expr: &PlanExpr) -> Result<ScalarValue, PlanError> {
    match expr {
        PlanExpr::Literal(v) => Ok(v.clone()),
        PlanExpr::Binary { left, op, right } => {
            let l = eval_const(left)?;
            let r = eval_const(right)?;
            apply_const_op(*op, &l, &r)
        }
        PlanExpr::Column(name) => Err(PlanError::ColumnNotFound(name.clone())),
    }
}

fn apply_const_op(op: PlanOp, l: &ScalarValue, r: &ScalarValue) -> Result<ScalarValue, PlanError> {
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
            _ => return Err(PlanError::TypeMismatch),
        },
    })
}

fn split_qualified(parts: &[sqlparser::ast::Ident]) -> (String, String) {
    if parts.len() == 1 {
        ("public".to_string(), parts[0].value.clone())
    } else {
        (parts[0].value.clone(), parts[1].value.clone())
    }
}

fn split_qualified_table_obj(table_name: &sqlparser::ast::ObjectName) -> (String, String) {
    split_qualified(&table_name.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlparser::dialect::PostgreSqlDialect;
    use sqlparser::parser::Parser;

    fn parse_one(sql: &str) -> Statement {
        let stmts = Parser::parse_sql(&PostgreSqlDialect {}, sql).unwrap();
        stmts.into_iter().next().unwrap()
    }

    fn empty_catalog() -> Catalog {
        Catalog::new()
    }

    #[test]
    fn plan_create_table_simple() {
        let stmt = parse_one("CREATE TABLE users (id INT PRIMARY KEY, name TEXT NOT NULL)");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::CreateTable { columns, .. } => {
                assert_eq!(columns.len(), 2);
                assert_eq!(columns[0].name, "id");
                assert!(columns[0].primary_key);
                assert!(!columns[1].nullable);
            }
            other => panic!("expected CreateTable, got {:?}", other),
        }
    }

    #[test]
    fn plan_create_table_if_not_exists() {
        let stmt = parse_one("CREATE TABLE IF NOT EXISTS t (a INT)");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::CreateTable { if_not_exists, .. } => assert!(if_not_exists),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_drop_table_if_exists() {
        let stmt = parse_one("DROP TABLE IF EXISTS t");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::DropTable { if_exists, .. } => assert!(if_exists),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_insert_with_columns() {
        let stmt = parse_one("INSERT INTO t (a, b) VALUES (1, 'x'), (2, 'y')");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::Insert { columns, rows, .. } => {
                assert_eq!(columns.unwrap(), vec!["a", "b"]);
                assert_eq!(rows.len(), 2);
            }
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_select_const() {
        let stmt = parse_one("SELECT 1");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::SelectConst(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].1, ScalarValue::Int4(1));
            }
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_select_const_with_alias() {
        let stmt = parse_one("SELECT 42 AS the_answer");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::SelectConst(rows) => {
                assert_eq!(rows[0].0, "the_answer");
            }
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_select_from_table_requires_catalog_entry() {
        let stmt = parse_one("SELECT * FROM users");
        let err = plan_statement(stmt, &empty_catalog()).unwrap_err();
        assert!(matches!(err, PlanError::TableNotFound(_)));
    }

    #[test]
    fn plan_select_with_filter() {
        let mut catalog = empty_catalog();
        catalog
            .create_table(
                "public",
                "users",
                vec![ColumnDef {
                    name: "id".into(),
                    ty: ScalarType::Int4,
                    nullable: false,
                    primary_key: true,
                    default_text: None,
                }],
            )
            .unwrap();
        let stmt = parse_one("SELECT * FROM users WHERE id = 1");
        let plan = plan_statement(stmt, &catalog).unwrap();
        match plan {
            Plan::Select { filter, .. } => assert!(filter.is_some()),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_select_with_order_by_and_limit() {
        let mut catalog = empty_catalog();
        catalog
            .create_table(
                "public",
                "users",
                vec![ColumnDef {
                    name: "id".into(),
                    ty: ScalarType::Int4,
                    nullable: false,
                    primary_key: true,
                    default_text: None,
                }],
            )
            .unwrap();
        let stmt = parse_one("SELECT * FROM users ORDER BY id DESC LIMIT 10");
        let plan = plan_statement(stmt, &catalog).unwrap();
        match plan {
            Plan::Select {
                order_by, limit, ..
            } => {
                assert_eq!(order_by, vec![(0, false)]);
                assert_eq!(limit, Some(10));
            }
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_update_simple() {
        let stmt = parse_one("UPDATE t SET a = 99 WHERE id = 1");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::Update { assignments, filter, .. } => {
                assert_eq!(assignments[0].0, "a");
                assert!(filter.is_some());
            }
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_delete_with_where() {
        let stmt = parse_one("DELETE FROM t WHERE id = 1");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::Delete { filter, .. } => assert!(filter.is_some()),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_begin_commit_rollback() {
        let begin = plan_statement(parse_one("BEGIN"), &empty_catalog()).unwrap();
        let commit = plan_statement(parse_one("COMMIT"), &empty_catalog()).unwrap();
        let rollback = plan_statement(parse_one("ROLLBACK"), &empty_catalog()).unwrap();
        assert_eq!(begin, Plan::Begin);
        assert_eq!(commit, Plan::Commit);
        assert_eq!(rollback, Plan::Rollback);
    }

    #[test]
    fn plan_set_parameter() {
        let stmt = parse_one("SET client_encoding TO 'UTF8'");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::SetParameter { name, value } => {
                assert_eq!(name, "client_encoding");
                assert!(value.contains("UTF8"));
            }
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_select_with_negative_literal() {
        let stmt = parse_one("SELECT -42 AS neg");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::SelectConst(rows) => assert_eq!(rows[0].1, ScalarValue::Int4(-42)),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_select_arith_const_compares() {
        let stmt = parse_one("SELECT 1 = 1 AS truth");
        let plan = plan_statement(stmt, &empty_catalog()).unwrap();
        match plan {
            Plan::SelectConst(rows) => assert_eq!(rows[0].1, ScalarValue::Bool(true)),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn plan_qualified_schema_table() {
        let stmt = parse_one("CREATE TABLE acme.users (id INT)");
        let mut c = empty_catalog();
        c.create_schema("acme");
        let plan = plan_statement(stmt, &c).unwrap();
        match plan {
            Plan::CreateTable { schema, name, .. } => {
                assert_eq!(schema, "acme");
                assert_eq!(name, "users");
            }
            other => panic!("got {:?}", other),
        }
    }
}
