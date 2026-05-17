// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Recursive-descent SQL parser.

use crate::sql::ast::*;
use crate::sql::lexer::{Lexer, Token};

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        Parser {
            tokens,
            position: 0,
        }
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or(&Token::Eof)
    }

    #[allow(dead_code)]
    fn peek(&self) -> &Token {
        self.tokens.get(self.position + 1).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        if std::mem::discriminant(self.current()) == std::mem::discriminant(&expected) {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected {:?}, got {:?}", expected, self.current()))
        }
    }

    pub fn parse(&mut self) -> Result<Ast, String> {
        let statement = self.parse_statement()?;
        Ok(Ast { statement })
    }

    fn parse_statement(&mut self) -> Result<Statement, String> {
        match self.current() {
            Token::Select => self.parse_select(),
            Token::Insert => self.parse_insert(),
            Token::Update => self.parse_update(),
            Token::Delete => self.parse_delete(),
            Token::Create => self.parse_create(),
            Token::Drop => self.parse_drop(),
            Token::Alter => self.parse_alter(),
            Token::Begin => {
                self.advance();
                Ok(Statement::Begin)
            }
            Token::Commit => {
                self.advance();
                Ok(Statement::Commit)
            }
            Token::Rollback => {
                self.advance();
                if matches!(self.current(), Token::To) {
                    self.advance();
                    if let Token::Savepoint = self.current() {
                        self.advance();
                    }
                    let name = self.parse_ident()?;
                    Ok(Statement::RollbackTo(name))
                } else {
                    Ok(Statement::Rollback)
                }
            }
            Token::Savepoint => {
                self.advance();
                let name = self.parse_ident()?;
                Ok(Statement::Savepoint(name))
            }
            Token::Explain => {
                self.advance();
                let stmt = self.parse_statement()?;
                Ok(Statement::Explain(Box::new(stmt)))
            }
            Token::Show => {
                self.advance();
                let name = self.parse_ident()?;
                Ok(Statement::Show(name))
            }
            Token::Copy => {
                self.advance();
                let table = self.parse_ident()?;
                self.expect(Token::From)?;
                self.expect(Token::Stdin)?;
                Ok(Statement::Copy {
                    table,
                    stdin: true,
                })
            }
            _ => Err(format!("unexpected token: {:?}", self.current())),
        }
    }

    fn parse_select(&mut self) -> Result<Statement, String> {
        self.expect(Token::Select)?;
        let distinct = if matches!(self.current(), Token::Distinct) {
            self.advance();
            true
        } else {
            false
        };
        let columns = self.parse_select_columns()?;
        let from = if matches!(self.current(), Token::From) {
            self.advance();
            Some(Box::new(self.parse_from()?))
        } else {
            None
        };
        let where_clause = if matches!(self.current(), Token::Where) {
            self.advance();
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        let group_by = if matches!(self.current(), Token::Group) {
            self.advance();
            self.expect(Token::By)?;
            let mut exprs = vec![self.parse_expr()?];
            while matches!(self.current(), Token::Comma) {
                self.advance();
                exprs.push(self.parse_expr()?);
            }
            Some(exprs)
        } else {
            None
        };
        let having = if matches!(self.current(), Token::Having) {
            self.advance();
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        let order_by = if matches!(self.current(), Token::Order) {
            self.advance();
            self.expect(Token::By)?;
            let mut items = vec![self.parse_order_by()?];
            while matches!(self.current(), Token::Comma) {
                self.advance();
                items.push(self.parse_order_by()?);
            }
            Some(items)
        } else {
            None
        };
        let limit = if matches!(self.current(), Token::Limit) {
            self.advance();
            if let Token::Integer(n) = self.current() {
                let n = *n;
                self.advance();
                Some(n)
            } else {
                return Err("expected integer for LIMIT".to_string());
            }
        } else {
            None
        };
        let offset = if matches!(self.current(), Token::Offset) {
            self.advance();
            if let Token::Integer(n) = self.current() {
                let n = *n;
                self.advance();
                Some(n)
            } else {
                return Err("expected integer for OFFSET".to_string());
            }
        } else {
            None
        };
        Ok(Statement::Select(SelectStmt {
            distinct,
            columns,
            from,
            where_clause,
            group_by,
            having,
            order_by,
            limit,
            offset,
        }))
    }

    fn parse_select_columns(&mut self) -> Result<Vec<SelectColumn>, String> {
        let mut columns = Vec::new();
        if matches!(self.current(), Token::Star) {
            self.advance();
            columns.push(SelectColumn::Star);
        } else {
            columns.push(self.parse_select_column()?);
            while matches!(self.current(), Token::Comma) {
                self.advance();
                columns.push(self.parse_select_column()?);
            }
        }
        Ok(columns)
    }

    fn parse_select_column(&mut self) -> Result<SelectColumn, String> {
        if matches!(self.current(), Token::Star) {
            self.advance();
            return Ok(SelectColumn::Star);
        }
        let expr = self.parse_expr()?;
        let alias = if matches!(self.current(), Token::As) {
            self.advance();
            Some(self.parse_ident()?)
        } else if matches!(self.current(), Token::Identifier(_)) {
            Some(self.parse_ident()?)
        } else {
            None
        };
        Ok(SelectColumn::Expr(expr, alias))
    }

    fn parse_from(&mut self) -> Result<FromClause, String> {
        let table = self.parse_ident()?;
        let alias = if matches!(self.current(), Token::As) {
            self.advance();
            Some(self.parse_ident()?)
        } else if matches!(self.current(), Token::Identifier(_)) {
            Some(self.parse_ident()?)
        } else {
            None
        };
        let mut from = FromClause::Table(table, alias);
        while matches!(
            self.current(),
            Token::Join | Token::Inner | Token::Left | Token::Right | Token::Full
        ) {
            let kind = match self.current() {
                Token::Inner => {
                    self.advance();
                    JoinKind::Inner
                }
                Token::Left => {
                    self.advance();
                    JoinKind::Left
                }
                Token::Right => {
                    self.advance();
                    JoinKind::Right
                }
                Token::Full => {
                    self.advance();
                    JoinKind::Full
                }
                _ => JoinKind::Inner,
            };
            self.expect(Token::Join)?;
            let right_table = self.parse_ident()?;
            let right_alias = if matches!(self.current(), Token::As) {
                self.advance();
                Some(self.parse_ident()?)
            } else if matches!(self.current(), Token::Identifier(_)) {
                Some(self.parse_ident()?)
            } else {
                None
            };
            let right = Box::new(FromClause::Table(right_table, right_alias));
            let on = if matches!(self.current(), Token::On) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            from = FromClause::Join {
                left: Box::new(from),
                kind,
                right,
                on,
            };
        }
        Ok(from)
    }

    fn parse_order_by(&mut self) -> Result<OrderBy, String> {
        let expr = self.parse_expr()?;
        let descending = if matches!(self.current(), Token::Identifier(s) if s.to_uppercase() == "DESC") {
            self.advance();
            true
        } else if matches!(self.current(), Token::Identifier(s) if s.to_uppercase() == "ASC") {
            self.advance();
            false
        } else {
            false
        };
        Ok(OrderBy { expr, descending })
    }

    fn parse_insert(&mut self) -> Result<Statement, String> {
        self.expect(Token::Insert)?;
        self.expect(Token::Into)?;
        let table = self.parse_ident()?;
        let columns = if matches!(self.current(), Token::LeftParen) {
            self.advance();
            let mut cols = vec![self.parse_ident()?];
            while matches!(self.current(), Token::Comma) {
                self.advance();
                cols.push(self.parse_ident()?);
            }
            self.expect(Token::RightParen)?;
            Some(cols)
        } else {
            None
        };
        self.expect(Token::Values)?;
        let mut values = Vec::new();
        self.expect(Token::LeftParen)?;
        let mut row = vec![self.parse_expr()?];
        while matches!(self.current(), Token::Comma) {
            self.advance();
            row.push(self.parse_expr()?);
        }
        self.expect(Token::RightParen)?;
        values.push(row);
        while matches!(self.current(), Token::Comma) {
            self.advance();
            self.expect(Token::LeftParen)?;
            let mut row = vec![self.parse_expr()?];
            while matches!(self.current(), Token::Comma) {
                self.advance();
                row.push(self.parse_expr()?);
            }
            self.expect(Token::RightParen)?;
            values.push(row);
        }
        Ok(Statement::Insert(InsertStmt {
            table,
            columns,
            values,
        }))
    }

    fn parse_update(&mut self) -> Result<Statement, String> {
        self.expect(Token::Update)?;
        let table = self.parse_ident()?;
        self.expect(Token::Set)?;
        let mut assignments = vec![self.parse_assignment()?];
        while matches!(self.current(), Token::Comma) {
            self.advance();
            assignments.push(self.parse_assignment()?);
        }
        let where_clause = if matches!(self.current(), Token::Where) {
            self.advance();
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        Ok(Statement::Update(UpdateStmt {
            table,
            assignments,
            where_clause,
        }))
    }

    fn parse_assignment(&mut self) -> Result<(String, Expr), String> {
        let col = self.parse_ident()?;
        self.expect(Token::Equal)?;
        let expr = self.parse_expr()?;
        Ok((col, expr))
    }

    fn parse_delete(&mut self) -> Result<Statement, String> {
        self.expect(Token::Delete)?;
        self.expect(Token::From)?;
        let table = self.parse_ident()?;
        let where_clause = if matches!(self.current(), Token::Where) {
            self.advance();
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        Ok(Statement::Delete(DeleteStmt { table, where_clause }))
    }

    fn parse_create(&mut self) -> Result<Statement, String> {
        self.expect(Token::Create)?;
        if matches!(self.current(), Token::Table) {
            self.parse_create_table()
        } else if matches!(self.current(), Token::Index) {
            self.parse_create_index()
        } else if matches!(self.current(), Token::Schema) {
            self.advance();
            let name = self.parse_ident()?;
            Ok(Statement::CreateSchema(CreateSchemaStmt { name }))
        } else {
            Err("expected TABLE, INDEX, or SCHEMA".to_string())
        }
    }

    fn parse_create_table(&mut self) -> Result<Statement, String> {
        self.expect(Token::Table)?;
        let table = self.parse_ident()?;
        self.expect(Token::LeftParen)?;
        let mut columns = Vec::new();
        let mut constraints = Vec::new();
        loop {
            if matches!(self.current(), Token::Primary) {
                self.advance();
                self.expect(Token::Key)?;
                self.expect(Token::LeftParen)?;
                let mut cols = vec![self.parse_ident()?];
                while matches!(self.current(), Token::Comma) {
                    self.advance();
                    cols.push(self.parse_ident()?);
                }
                self.expect(Token::RightParen)?;
                constraints.push(TableConstraint::PrimaryKey(cols));
            } else if matches!(self.current(), Token::Identifier(_)) {
                let name = self.parse_ident()?;
                let type_name = self.parse_type()?;
                let not_null = if matches!(self.current(), Token::Not) {
                    self.advance();
                    self.expect(Token::Null)?;
                    true
                } else {
                    false
                };
                let default = if matches!(self.current(), Token::Default) {
                    self.advance();
                    Some(Box::new(self.parse_expr()?))
                } else {
                    None
                };
                let primary_key = if matches!(self.current(), Token::Primary) {
                    self.advance();
                    self.expect(Token::Key)?;
                    true
                } else {
                    false
                };
                columns.push(ColumnDef {
                    name,
                    type_name,
                    not_null,
                    default,
                    primary_key,
                });
            } else {
                return Err(format!("unexpected token: {:?}", self.current()));
            }
            if matches!(self.current(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(Token::RightParen)?;
        Ok(Statement::CreateTable(CreateTableStmt {
            table,
            columns,
            constraints,
        }))
    }

    fn parse_create_index(&mut self) -> Result<Statement, String> {
        self.expect(Token::Index)?;
        let name = self.parse_ident()?;
        self.expect(Token::On)?;
        let table = self.parse_ident()?;
        self.expect(Token::LeftParen)?;
        let mut columns = vec![self.parse_ident()?];
        while matches!(self.current(), Token::Comma) {
            self.advance();
            columns.push(self.parse_ident()?);
        }
        self.expect(Token::RightParen)?;
        Ok(Statement::CreateIndex(CreateIndexStmt {
            name,
            table,
            columns,
            unique: false,
        }))
    }

    fn parse_drop(&mut self) -> Result<Statement, String> {
        self.expect(Token::Drop)?;
        if matches!(self.current(), Token::Table) {
            self.advance();
            let if_exists = if matches!(self.current(), Token::If) {
                self.advance();
                self.expect(Token::Exists)?;
                true
            } else {
                false
            };
            let table = self.parse_ident()?;
            Ok(Statement::DropTable(DropTableStmt { table, if_exists }))
        } else if matches!(self.current(), Token::Index) {
            self.advance();
            let if_exists = if matches!(self.current(), Token::If) {
                self.advance();
                self.expect(Token::Exists)?;
                true
            } else {
                false
            };
            let name = self.parse_ident()?;
            Ok(Statement::DropIndex(DropIndexStmt { name, if_exists }))
        } else {
            Err("expected TABLE or INDEX".to_string())
        }
    }

    fn parse_alter(&mut self) -> Result<Statement, String> {
        self.expect(Token::Alter)?;
        self.expect(Token::Table)?;
        let table = self.parse_ident()?;
        if matches!(self.current(), Token::Add) {
            self.advance();
            self.expect(Token::Column)?;
            let name = self.parse_ident()?;
            let type_name = self.parse_type()?;
            let not_null = if matches!(self.current(), Token::Not) {
                self.advance();
                self.expect(Token::Null)?;
                true
            } else {
                false
            };
            let default = if matches!(self.current(), Token::Default) {
                self.advance();
                Some(Box::new(self.parse_expr()?))
            } else {
                None
            };
            let column = ColumnDef {
                name,
                type_name,
                not_null,
                default,
                primary_key: false,
            };
            Ok(Statement::AlterTable(AlterTableStmt::AddColumn { table, column }))
        } else {
            Err("expected ADD COLUMN".to_string())
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while matches!(self.current(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        while matches!(self.current(), Token::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        while let Some(op) = self.current_comparison_op() {
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn current_comparison_op(&self) -> Option<BinaryOp> {
        match self.current() {
            Token::Equal => Some(BinaryOp::Eq),
            Token::NotEqual => Some(BinaryOp::Ne),
            Token::Less => Some(BinaryOp::Lt),
            Token::Greater => Some(BinaryOp::Gt),
            Token::LessEqual => Some(BinaryOp::Le),
            Token::GreaterEqual => Some(BinaryOp::Ge),
            Token::Like => Some(BinaryOp::Like),
            Token::ILike => Some(BinaryOp::ILike),
            _ => None,
        }
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        while let Some(op) = self.current_additive_op() {
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn current_additive_op(&self) -> Option<BinaryOp> {
        match self.current() {
            Token::Plus => Some(BinaryOp::Add),
            Token::Minus => Some(BinaryOp::Sub),
            _ => None,
        }
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        while let Some(op) = self.current_multiplicative_op() {
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn current_multiplicative_op(&self) -> Option<BinaryOp> {
        match self.current() {
            Token::Star => Some(BinaryOp::Mul),
            Token::Slash => Some(BinaryOp::Div),
            Token::Percent => Some(BinaryOp::Mod),
            _ => None,
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.current() {
            Token::Not => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                })
            }
            Token::Minus => {
                self.advance();
                let operand = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Minus,
                    operand: Box::new(operand),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if matches!(self.current(), Token::Is) {
                self.advance();
                let not = if matches!(self.current(), Token::Not) {
                    self.advance();
                    true
                } else {
                    false
                };
                self.expect(Token::Null)?;
                expr = Expr::IsNull {
                    expr: Box::new(expr),
                    not,
                };
            } else if matches!(self.current(), Token::In) {
                self.advance();
                self.expect(Token::LeftParen)?;
                if matches!(self.current(), Token::Select) {
                    let subquery = self.parse()?;
                    self.expect(Token::RightParen)?;
                    expr = Expr::InSubquery {
                        expr: Box::new(expr),
                        subquery: Box::new(subquery),
                        not: false,
                    };
                } else {
                    let mut list = vec![self.parse_expr()?];
                    while matches!(self.current(), Token::Comma) {
                        self.advance();
                        list.push(self.parse_expr()?);
                    }
                    self.expect(Token::RightParen)?;
                    expr = Expr::InList {
                        expr: Box::new(expr),
                        list,
                        not: false,
                    };
                }
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.current() {
            Token::LeftParen => {
                self.advance();
                if matches!(self.current(), Token::Select) {
                    let subquery = self.parse()?;
                    self.expect(Token::RightParen)?;
                    Ok(Expr::Subquery(Box::new(subquery)))
                } else {
                    let expr = self.parse_expr()?;
                    self.expect(Token::RightParen)?;
                    Ok(expr)
                }
            }
            Token::Null => {
                self.advance();
                Ok(Expr::Literal(Literal::Null))
            }
            Token::Integer(n) => {
                let n = *n;
                self.advance();
                Ok(Expr::Literal(Literal::Integer(n)))
            }
            Token::Float(f) => {
                let f = *f;
                self.advance();
                Ok(Expr::Literal(Literal::Float(f)))
            }
            Token::String(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expr::Literal(Literal::String(s)))
            }
            Token::Identifier(s) | Token::QuotedIdentifier(s) => {
                let name = s.clone();
                self.advance();
                if matches!(self.current(), Token::LeftParen) {
                    self.advance();
                    let mut args = Vec::new();
                    if !matches!(self.current(), Token::RightParen) {
                        args.push(self.parse_expr()?);
                        while matches!(self.current(), Token::Comma) {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(Token::RightParen)?;
                    Ok(Expr::FunctionCall { name, args })
                } else if matches!(self.current(), Token::Dot) {
                    self.advance();
                    let col = self.parse_ident()?;
                    Ok(Expr::QualifiedIdentifier(name, col))
                } else {
                    Ok(Expr::Identifier(name))
                }
            }
            Token::Cast => {
                self.advance();
                self.expect(Token::LeftParen)?;
                let expr = self.parse_expr()?;
                self.expect(Token::As)?;
                let type_name = self.parse_type()?;
                self.expect(Token::RightParen)?;
                Ok(Expr::Cast {
                    expr: Box::new(expr),
                    type_name,
                })
            }
            _ => Err(format!("unexpected token in expression: {:?}", self.current())),
        }
    }

    fn parse_ident(&mut self) -> Result<String, String> {
        match self.current() {
            Token::Identifier(s) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            Token::QuotedIdentifier(s) => {
                let s = s.clone();
                self.advance();
                Ok(s)
            }
            _ => Err(format!("expected identifier, got {:?}", self.current())),
        }
    }

    fn parse_type(&mut self) -> Result<String, String> {
        let type_str = self.parse_ident()?;
        if matches!(self.current(), Token::LeftParen) {
            self.advance();
            while !matches!(self.current(), Token::RightParen) {
                self.advance();
            }
            self.expect(Token::RightParen)?;
        }
        Ok(type_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_simple_select() {
        let mut parser = Parser::new("SELECT * FROM users");
        let ast = parser.parse().unwrap();
        assert!(matches!(ast.statement, Statement::Select(_)));
    }

    #[test]
    fn test_parser_insert() {
        let mut parser = Parser::new("INSERT INTO users (id, name) VALUES (1, 'alice')");
        let ast = parser.parse().unwrap();
        assert!(matches!(ast.statement, Statement::Insert(_)));
    }

    #[test]
    fn test_parser_update() {
        let mut parser = Parser::new("UPDATE users SET name = 'bob' WHERE id = 1");
        let ast = parser.parse().unwrap();
        assert!(matches!(ast.statement, Statement::Update(_)));
    }

    #[test]
    fn test_parser_delete() {
        let mut parser = Parser::new("DELETE FROM users WHERE id = 1");
        let ast = parser.parse().unwrap();
        assert!(matches!(ast.statement, Statement::Delete(_)));
    }

    #[test]
    fn test_parser_create_table() {
        let mut parser = Parser::new("CREATE TABLE users (id int PRIMARY KEY, name text)");
        let ast = parser.parse().unwrap();
        assert!(matches!(ast.statement, Statement::CreateTable(_)));
    }

    #[test]
    fn test_parser_begin_commit_rollback() {
        let mut parser = Parser::new("BEGIN");
        assert!(matches!(parser.parse().unwrap().statement, Statement::Begin));

        let mut parser = Parser::new("COMMIT");
        assert!(matches!(parser.parse().unwrap().statement, Statement::Commit));

        let mut parser = Parser::new("ROLLBACK");
        assert!(matches!(parser.parse().unwrap().statement, Statement::Rollback));
    }
}
