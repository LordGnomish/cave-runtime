// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Mini SQL parser — subset for the DataFusion MVP.
//!
//! Upstream: `crates/datafusion-sql/src/parser.rs` (which wraps
//! `sqlparser-rs`). The MVP ships a hand-rolled tokenizer + recursive
//! descent parser for the smallest useful subset:
//!
//! ```text
//! SELECT [DISTINCT] expr_list FROM tbl [WHERE expr] [GROUP BY expr_list]
//!   [ORDER BY expr_list] [LIMIT n] [OFFSET n]
//! ```
//!
//! Joins, subqueries, CTEs, window functions, and DDL are deferred
//! (`[[scope_cuts]] full-sql-grammar`).

use crate::error::{Error, Result};
use crate::logical_expr::{BinaryOp, LogicalExpr};
use crate::row::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStatement {
    pub distinct: bool,
    pub select_list: Vec<LogicalExpr>,
    pub from: Option<String>,
    pub where_clause: Option<LogicalExpr>,
    pub group_by: Vec<LogicalExpr>,
    pub order_by: Vec<(LogicalExpr, bool /* ascending */)>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub fn parse_sql(sql: &str) -> Result<SelectStatement> {
    let tokens = tokenize(sql)?;
    let mut p = Parser { tokens, pos: 0 };
    p.parse_select()
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    QuotedString(String),
    Number(String),
    Punct(char),
    Star,
    Op(String), // <=, >=, <>, !=
    Comma,
    Lparen,
    Rparen,
    Semicolon,
    Eof,
}

fn tokenize(s: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == ',' {
            out.push(Token::Comma);
            i += 1;
        } else if c == '(' {
            out.push(Token::Lparen);
            i += 1;
        } else if c == ')' {
            out.push(Token::Rparen);
            i += 1;
        } else if c == ';' {
            out.push(Token::Semicolon);
            i += 1;
        } else if c == '*' {
            out.push(Token::Star);
            i += 1;
        } else if c == '\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] as char != '\'' {
                i += 1;
            }
            if i >= bytes.len() {
                return Err(Error::SqlParse("unterminated string".into()));
            }
            let s = std::str::from_utf8(&bytes[start..i])
                .map_err(|_| Error::SqlParse("bad utf8 in string literal".into()))?
                .to_string();
            i += 1;
            out.push(Token::QuotedString(s));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && {
                let ch = bytes[i] as char;
                ch.is_ascii_digit() || ch == '.'
            } {
                i += 1;
            }
            out.push(Token::Number(s[start..i].to_string()));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() && {
                let ch = bytes[i] as char;
                ch.is_ascii_alphanumeric() || ch == '_'
            } {
                i += 1;
            }
            out.push(Token::Ident(s[start..i].to_string()));
        } else if c == '<' || c == '>' || c == '=' || c == '!' {
            // Try to combine into <=, >=, <>, !=
            let next = bytes.get(i + 1).map(|b| *b as char);
            let two = matches!(
                (c, next),
                ('<', Some('='))
                    | ('>', Some('='))
                    | ('<', Some('>'))
                    | ('!', Some('='))
            );
            if two {
                let op: String = format!("{}{}", c, next.unwrap());
                i += 2;
                out.push(Token::Op(op));
            } else {
                i += 1;
                out.push(Token::Op(c.to_string()));
            }
        } else if c == '+' || c == '-' || c == '/' || c == '%' {
            out.push(Token::Op(c.to_string()));
            i += 1;
        } else if c == '.' {
            out.push(Token::Punct('.'));
            i += 1;
        } else {
            return Err(Error::SqlParse(format!("unexpected char: {}", c)));
        }
    }
    out.push(Token::Eof);
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn eat(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        self.pos += 1;
        t
    }

    fn matches_ident(&mut self, want: &str) -> bool {
        if let Token::Ident(s) = self.peek() {
            if s.eq_ignore_ascii_case(want) {
                self.pos += 1;
                return true;
            }
        }
        false
    }

    fn expect_ident(&mut self, want: &str) -> Result<()> {
        if self.matches_ident(want) {
            Ok(())
        } else {
            Err(Error::SqlParse(format!("expected {:?}, got {:?}", want, self.peek())))
        }
    }

    fn parse_select(&mut self) -> Result<SelectStatement> {
        self.expect_ident("SELECT")?;
        let distinct = self.matches_ident("DISTINCT");
        let select_list = self.parse_select_list()?;
        let mut stmt = SelectStatement {
            distinct,
            select_list,
            from: None,
            where_clause: None,
            group_by: vec![],
            order_by: vec![],
            limit: None,
            offset: None,
        };
        if self.matches_ident("FROM") {
            if let Token::Ident(n) = self.eat() {
                stmt.from = Some(n);
            } else {
                return Err(Error::SqlParse("expected table name after FROM".into()));
            }
        }
        if self.matches_ident("WHERE") {
            stmt.where_clause = Some(self.parse_expr(0)?);
        }
        if self.matches_ident("GROUP") {
            self.expect_ident("BY")?;
            stmt.group_by = self.parse_expr_list()?;
        }
        if self.matches_ident("ORDER") {
            self.expect_ident("BY")?;
            loop {
                let e = self.parse_expr(0)?;
                let asc = if self.matches_ident("DESC") {
                    false
                } else {
                    let _ = self.matches_ident("ASC");
                    true
                };
                stmt.order_by.push((e, asc));
                if !matches!(self.peek(), Token::Comma) {
                    break;
                }
                self.eat();
            }
        }
        if self.matches_ident("LIMIT") {
            if let Token::Number(n) = self.eat() {
                stmt.limit = Some(n.parse().map_err(|_| Error::SqlParse(format!("LIMIT: {}", n)))?);
            } else {
                return Err(Error::SqlParse("expected number after LIMIT".into()));
            }
        }
        if self.matches_ident("OFFSET") {
            if let Token::Number(n) = self.eat() {
                stmt.offset =
                    Some(n.parse().map_err(|_| Error::SqlParse(format!("OFFSET: {}", n)))?);
            } else {
                return Err(Error::SqlParse("expected number after OFFSET".into()));
            }
        }
        Ok(stmt)
    }

    fn parse_select_list(&mut self) -> Result<Vec<LogicalExpr>> {
        if let Token::Star = self.peek() {
            // * → unfold at planner-time; carry as a sentinel column "*".
            self.eat();
            return Ok(vec![LogicalExpr::Column { name: "*".into() }]);
        }
        self.parse_expr_list()
    }

    fn parse_expr_list(&mut self) -> Result<Vec<LogicalExpr>> {
        let mut out = vec![self.parse_expr(0)?];
        while matches!(self.peek(), Token::Comma) {
            self.eat();
            out.push(self.parse_expr(0)?);
        }
        Ok(out)
    }

    /// Operator precedence — higher binds tighter. Matches DataFusion's
    /// Postgres-compatible parser:
    /// `OR(1) < AND(2) < cmp(3) < +/-(4) < */%(5) < unary(6)`.
    fn parse_expr(&mut self, min_prec: u8) -> Result<LogicalExpr> {
        let mut lhs = self.parse_primary()?;
        loop {
            let op_opt: Option<(BinaryOp, u8)> = match self.peek() {
                Token::Op(s) => match s.as_str() {
                    "+" => Some((BinaryOp::Plus, 4)),
                    "-" => Some((BinaryOp::Minus, 4)),
                    "*" => Some((BinaryOp::Multiply, 5)),
                    "/" => Some((BinaryOp::Divide, 5)),
                    "%" => Some((BinaryOp::Modulo, 5)),
                    "=" => Some((BinaryOp::Eq, 3)),
                    "!=" | "<>" => Some((BinaryOp::NotEq, 3)),
                    "<" => Some((BinaryOp::Lt, 3)),
                    "<=" => Some((BinaryOp::LtEq, 3)),
                    ">" => Some((BinaryOp::Gt, 3)),
                    ">=" => Some((BinaryOp::GtEq, 3)),
                    _ => None,
                },
                Token::Ident(s) if s.eq_ignore_ascii_case("AND") => Some((BinaryOp::And, 2)),
                Token::Ident(s) if s.eq_ignore_ascii_case("OR") => Some((BinaryOp::Or, 1)),
                _ => None,
            };
            let Some((op, prec)) = op_opt else { break };
            if prec < min_prec {
                break;
            }
            self.eat();
            let rhs = self.parse_expr(prec + 1)?;
            lhs = LogicalExpr::BinaryOp {
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_primary(&mut self) -> Result<LogicalExpr> {
        let tok = self.eat();
        match tok {
            Token::Number(s) => {
                if let Ok(n) = s.parse::<i64>() {
                    Ok(LogicalExpr::lit(n))
                } else if let Ok(f) = s.parse::<f64>() {
                    Ok(LogicalExpr::lit(f))
                } else {
                    Err(Error::SqlParse(format!("bad number: {}", s)))
                }
            }
            Token::QuotedString(s) => Ok(LogicalExpr::Literal { value: Value::Utf8(s) }),
            Token::Lparen => {
                let e = self.parse_expr(0)?;
                if !matches!(self.peek(), Token::Rparen) {
                    return Err(Error::SqlParse("expected )".into()));
                }
                self.eat();
                Ok(e)
            }
            Token::Ident(name) => {
                // Function call? `name(...)`
                if matches!(self.peek(), Token::Lparen) {
                    self.eat();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Token::Rparen) {
                        args.push(self.parse_expr(0)?);
                        while matches!(self.peek(), Token::Comma) {
                            self.eat();
                            args.push(self.parse_expr(0)?);
                        }
                    }
                    if !matches!(self.peek(), Token::Rparen) {
                        return Err(Error::SqlParse("expected )".into()));
                    }
                    self.eat();
                    return Ok(LogicalExpr::Function { name, args });
                }
                // `AS alias`?
                let mut col = LogicalExpr::Column { name };
                if self.matches_ident("AS") {
                    if let Token::Ident(alias) = self.eat() {
                        col = col.alias(alias);
                    } else {
                        return Err(Error::SqlParse("expected alias after AS".into()));
                    }
                }
                Ok(col)
            }
            other => Err(Error::SqlParse(format!("unexpected token: {:?}", other))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_select_star() {
        let s = parse_sql("SELECT * FROM users").unwrap();
        assert_eq!(s.from.as_deref(), Some("users"));
        assert_eq!(s.select_list.len(), 1);
    }

    #[test]
    fn parses_where_and_group_by_and_limit() {
        let s = parse_sql("SELECT a, b FROM t WHERE a > 5 GROUP BY a LIMIT 10").unwrap();
        assert_eq!(s.select_list.len(), 2);
        assert!(s.where_clause.is_some());
        assert_eq!(s.group_by.len(), 1);
        assert_eq!(s.limit, Some(10));
    }

    #[test]
    fn parses_function_call() {
        let s = parse_sql("SELECT count(a), sum(b) FROM t").unwrap();
        assert!(matches!(&s.select_list[0], LogicalExpr::Function { name, .. } if name == "count"));
    }

    #[test]
    fn parses_order_by_desc() {
        let s = parse_sql("SELECT a FROM t ORDER BY a DESC, b").unwrap();
        assert_eq!(s.order_by.len(), 2);
        assert!(!s.order_by[0].1);
        assert!(s.order_by[1].1);
    }

    #[test]
    fn precedence_keeps_and_above_or() {
        // a OR b AND c => a OR (b AND c)
        let s = parse_sql("SELECT 1 FROM t WHERE a OR b AND c").unwrap();
        match s.where_clause.unwrap() {
            LogicalExpr::BinaryOp { op: BinaryOp::Or, right, .. } => match *right {
                LogicalExpr::BinaryOp { op: BinaryOp::And, .. } => {}
                _ => panic!("expected AND inside OR"),
            },
            _ => panic!("expected OR top"),
        }
    }

    #[test]
    fn alias_assigns_output_name() {
        let s = parse_sql("SELECT a AS x FROM t").unwrap();
        assert_eq!(s.select_list[0].output_name(), "x");
    }

    #[test]
    fn string_literal_parses() {
        let s = parse_sql("SELECT a FROM t WHERE name = 'Alice'").unwrap();
        let where_e = s.where_clause.unwrap();
        if let LogicalExpr::BinaryOp { right, .. } = where_e {
            if let LogicalExpr::Literal { value: Value::Utf8(s) } = *right {
                assert_eq!(s, "Alice");
                return;
            }
        }
        panic!("expected string literal in WHERE");
    }

    #[test]
    fn distinct_flag_parses() {
        let s = parse_sql("SELECT DISTINCT a FROM t").unwrap();
        assert!(s.distinct);
    }

    #[test]
    fn offset_after_limit() {
        let s = parse_sql("SELECT a FROM t LIMIT 5 OFFSET 10").unwrap();
        assert_eq!(s.limit, Some(5));
        assert_eq!(s.offset, Some(10));
    }
}
