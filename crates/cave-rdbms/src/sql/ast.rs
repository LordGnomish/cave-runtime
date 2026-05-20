// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Abstract syntax tree for SQL.

#[derive(Debug, Clone, PartialEq)]
pub struct Ast {
    pub statement: Statement,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Select(SelectStmt),
    Insert(InsertStmt),
    Update(UpdateStmt),
    Delete(DeleteStmt),
    CreateTable(CreateTableStmt),
    DropTable(DropTableStmt),
    CreateIndex(CreateIndexStmt),
    DropIndex(DropIndexStmt),
    CreateSchema(CreateSchemaStmt),
    AlterTable(AlterTableStmt),
    Begin,
    Commit,
    Rollback,
    Savepoint(String),
    RollbackTo(String),
    Explain(Box<Statement>),
    Set { key: String, value: String },
    Show(String),
    Copy { table: String, stdin: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStmt {
    pub distinct: bool,
    pub columns: Vec<SelectColumn>,
    pub from: Option<Box<FromClause>>,
    pub where_clause: Option<Box<Expr>>,
    pub group_by: Option<Vec<Expr>>,
    pub having: Option<Box<Expr>>,
    pub order_by: Option<Vec<OrderBy>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumn {
    Star,
    TableStar(String),
    Expr(Expr, Option<String>), // expr, alias
}

#[derive(Debug, Clone, PartialEq)]
pub enum FromClause {
    Table(String, Option<String>), // table, alias
    Join {
        left: Box<FromClause>,
        kind: JoinKind,
        right: Box<FromClause>,
        on: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub expr: Expr,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsertStmt {
    pub table: String,
    pub columns: Option<Vec<String>>,
    pub values: Vec<Vec<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStmt {
    pub table: String,
    pub assignments: Vec<(String, Expr)>,
    pub where_clause: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteStmt {
    pub table: String,
    pub where_clause: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTableStmt {
    pub table: String,
    pub columns: Vec<ColumnDef>,
    pub constraints: Vec<TableConstraint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub type_name: String,
    pub not_null: bool,
    pub default: Option<Box<Expr>>,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableConstraint {
    PrimaryKey(Vec<String>),
    Unique(Vec<String>),
    ForeignKey {
        columns: Vec<String>,
        ref_table: String,
        ref_columns: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropTableStmt {
    pub table: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateIndexStmt {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropIndexStmt {
    pub name: String,
    pub if_exists: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateSchemaStmt {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlterTableStmt {
    AddColumn {
        table: String,
        column: ColumnDef,
    },
    DropColumn {
        table: String,
        column: String,
    },
    RenameColumn {
        table: String,
        old_name: String,
        new_name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal),
    Identifier(String),
    QualifiedIdentifier(String, String), // table.column
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    FunctionCall {
        name: String,
        args: Vec<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        type_name: String,
    },
    Case {
        operand: Option<Box<Expr>>,
        whens: Vec<(Expr, Expr)>, // (condition, result)
        else_expr: Option<Box<Expr>>,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
        not: bool,
    },
    InSubquery {
        expr: Box<Expr>,
        subquery: Box<Ast>,
        not: bool,
    },
    Subquery(Box<Ast>),
    IsNull {
        expr: Box<Expr>,
        not: bool,
    },
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        not: bool,
    },
    Alias(Box<Expr>, String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOp {
    // Comparison
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Logical
    And,
    Or,
    // String
    Like,
    ILike,
    // Other
    Concat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Minus,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Integer(i64),
    Float(f64),
    String(String),
    Boolean(bool),
    Date(String),
    Timestamp(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ast_select_creation() {
        let select = SelectStmt {
            distinct: false,
            columns: vec![SelectColumn::Star],
            from: None,
            where_clause: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: Some(10),
            offset: None,
        };
        assert_eq!(select.limit, Some(10));
    }

    #[test]
    fn test_binary_op_types() {
        let ops = vec![BinaryOp::Eq, BinaryOp::Add, BinaryOp::And, BinaryOp::Like];
        assert_eq!(ops.len(), 4);
    }

    #[test]
    fn test_create_table_stmt() {
        let col = ColumnDef {
            name: "id".to_string(),
            type_name: "int".to_string(),
            not_null: true,
            default: None,
            primary_key: true,
        };
        let stmt = CreateTableStmt {
            table: "users".to_string(),
            columns: vec![col],
            constraints: vec![],
        };
        assert_eq!(stmt.table, "users");
    }
}
