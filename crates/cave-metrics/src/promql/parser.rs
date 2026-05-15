// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Recursive-descent PromQL parser.
//! Operator precedence (highest → lowest):
//!   ^  (right-associative)
//!   unary -
//!   * / %
//!   + -
//!   == != < <= > >=
//!   and unless
//!   or

use crate::error::{MetricsError, Result};
use crate::model::LabelMatcher;
use crate::promql::ast::*;
use crate::promql::lexer::{Lexer, Token};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut lex = Lexer::new(input);
        Self { tokens: lex.tokenize(), pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek2(&self) -> &Token {
        self.tokens.get(self.pos + 1).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() { self.pos += 1; }
        t
    }

    fn eat(&mut self, expected: &Token) -> Result<()> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            self.advance();
            Ok(())
        } else {
            Err(MetricsError::Parse(format!("expected {:?}, got {:?}", expected, self.peek())))
        }
    }

    fn eat_ident(&mut self) -> Result<String> {
        match self.advance().clone() {
            Token::Ident(s) => Ok(s),
            other => Err(MetricsError::Parse(format!("expected identifier, got {:?}", other))),
        }
    }

    /// Parse a complete expression.
    pub fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    // or
    fn parse_or(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let matching = self.parse_vector_matching()?;
            let rhs = self.parse_and()?;
            lhs = Expr::Binary(BinaryExpr {
                op: BinaryOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs),
                matching, return_bool: false,
            });
        }
        Ok(lhs)
    }

    // and / unless
    fn parse_and(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Token::And    => BinaryOp::And,
                Token::Unless => BinaryOp::Unless,
                _ => break,
            };
            self.advance();
            let matching = self.parse_vector_matching()?;
            let rhs = self.parse_comparison()?;
            lhs = Expr::Binary(BinaryExpr {
                op, lhs: Box::new(lhs), rhs: Box::new(rhs), matching, return_bool: false,
            });
        }
        Ok(lhs)
    }

    // == != < <= > >=
    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_add()?;
        loop {
            let (op, return_bool) = match self.peek() {
                Token::Eq    => (BinaryOp::Eql, false),
                Token::Ne    => (BinaryOp::Neq, false),
                Token::Lt    => (BinaryOp::Lss, false),
                Token::Le    => (BinaryOp::Lte, false),
                Token::Gt    => (BinaryOp::Gtr, false),
                Token::Ge    => (BinaryOp::Gte, false),
                _ => break,
            };
            self.advance();
            let return_bool2 = if matches!(self.peek(), Token::Bool) {
                self.advance();
                true
            } else { return_bool };
            let matching = self.parse_vector_matching()?;
            let rhs = self.parse_add()?;
            lhs = Expr::Binary(BinaryExpr {
                op, lhs: Box::new(lhs), rhs: Box::new(rhs), matching, return_bool: return_bool2,
            });
        }
        Ok(lhs)
    }

    // + -
    fn parse_add(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Token::Plus  => BinaryOp::Add,
                Token::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let matching = self.parse_vector_matching()?;
            let rhs = self.parse_mul()?;
            lhs = Expr::Binary(BinaryExpr {
                op, lhs: Box::new(lhs), rhs: Box::new(rhs), matching, return_bool: false,
            });
        }
        Ok(lhs)
    }

    // * / % atan2
    fn parse_mul(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_pow()?;
        loop {
            let op = match self.peek() {
                Token::Star    => BinaryOp::Mul,
                Token::Slash   => BinaryOp::Div,
                Token::Percent => BinaryOp::Mod,
                Token::Atan2   => BinaryOp::Atan2,
                _ => break,
            };
            self.advance();
            let matching = self.parse_vector_matching()?;
            let rhs = self.parse_pow()?;
            lhs = Expr::Binary(BinaryExpr {
                op, lhs: Box::new(lhs), rhs: Box::new(rhs), matching, return_bool: false,
            });
        }
        Ok(lhs)
    }

    // ^ (right-associative)
    fn parse_pow(&mut self) -> Result<Expr> {
        let base = self.parse_unary()?;
        if matches!(self.peek(), Token::Caret) {
            self.advance();
            let matching = self.parse_vector_matching()?;
            let exp = self.parse_pow()?; // right-associative
            Ok(Expr::Binary(BinaryExpr {
                op: BinaryOp::Pow, lhs: Box::new(base), rhs: Box::new(exp),
                matching, return_bool: false,
            }))
        } else {
            Ok(base)
        }
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if matches!(self.peek(), Token::Minus) {
            self.advance();
            let inner = self.parse_postfix()?;
            return Ok(Expr::Unary(UnaryExpr { expr: Box::new(inner) }));
        }
        if matches!(self.peek(), Token::Plus) {
            self.advance();
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;

        // offset modifier
        if matches!(self.peek(), Token::Offset) {
            self.advance();
            let dur = self.parse_duration()?;
            expr = apply_offset(expr, dur);
        }

        // @ modifier
        if matches!(self.peek(), Token::At) {
            self.advance();
            let ts = match self.advance().clone() {
                Token::Number(n) => (n * 1000.0) as i64,
                _ => return Err(MetricsError::Parse("expected timestamp after @".into())),
            };
            expr = apply_at(expr, ts);
        }

        // subquery: [range:step]
        if matches!(self.peek(), Token::LBracket) {
            if let Expr::VectorSelector(_) = &expr {
                // Check if it's matrix or subquery
                let saved = self.pos;
                self.advance(); // [
                if let Ok(range) = self.parse_duration() {
                    if matches!(self.peek(), Token::Colon) {
                        self.advance(); // :
                        let step = if matches!(self.peek(), Token::RBracket) {
                            0
                        } else {
                            self.parse_duration()?
                        };
                        self.eat(&Token::RBracket)?;
                        let offset = if matches!(self.peek(), Token::Offset) {
                            self.advance();
                            Some(self.parse_duration()?)
                        } else { None };
                        let at = None;
                        return Ok(Expr::Subquery(Subquery {
                            expr: Box::new(expr), range_ms: range, step_ms: step, offset, at,
                        }));
                    } else if matches!(self.peek(), Token::RBracket) {
                        self.advance(); // ]
                        // Matrix selector
                        return Ok(self.make_matrix(expr, range)?);
                    } else {
                        self.pos = saved;
                    }
                } else {
                    self.pos = saved;
                }
            }
        }

        Ok(expr)
    }

    fn make_matrix(&self, expr: Expr, range_ms: i64) -> Result<Expr> {
        match expr {
            Expr::VectorSelector(vs) => Ok(Expr::MatrixSelector(MatrixSelector {
                selector: vs,
                range_ms,
                offset: None,
                at: None,
            })),
            _ => Err(MetricsError::Parse("matrix selector requires a vector selector".into())),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        match self.peek().clone() {
            Token::Number(n) => { self.advance(); Ok(Expr::NumberLiteral(n)) }
            Token::StringLit(s) => { self.advance(); Ok(Expr::StringLiteral(s)) }

            Token::LParen => {
                self.advance();
                let inner = self.parse_expr()?;
                self.eat(&Token::RParen)?;
                Ok(inner)
            }

            // Aggregation operators
            Token::Sum | Token::Min | Token::Max | Token::Avg | Token::Count
            | Token::Stddev | Token::Stdvar | Token::Topk | Token::Bottomk
            | Token::Quantile | Token::CountValues | Token::Group => {
                self.parse_aggregate()
            }

            // Named function or metric
            Token::Ident(_) => {
                // Look ahead: if next is `(`, it's a function call
                if matches!(self.peek2(), Token::LParen) {
                    self.parse_call()
                } else {
                    // Vector selector (metric name)
                    self.parse_vector_selector()
                }
            }

            // Bare label selector: `{job="foo"}`
            Token::LBrace => self.parse_vector_selector(),

            _ => Err(MetricsError::Parse(format!("unexpected token: {:?}", self.peek()))),
        }
    }

    fn parse_aggregate(&mut self) -> Result<Expr> {
        let op = match self.advance().clone() {
            Token::Sum        => AggregateOp::Sum,
            Token::Min        => AggregateOp::Min,
            Token::Max        => AggregateOp::Max,
            Token::Avg        => AggregateOp::Avg,
            Token::Count      => AggregateOp::Count,
            Token::Stddev     => AggregateOp::Stddev,
            Token::Stdvar     => AggregateOp::Stdvar,
            Token::Topk       => AggregateOp::Topk,
            Token::Bottomk    => AggregateOp::Bottomk,
            Token::Quantile   => AggregateOp::Quantile,
            Token::CountValues => AggregateOp::CountValues,
            Token::Group      => AggregateOp::Group,
            other => return Err(MetricsError::Parse(format!("expected aggregate op, got {:?}", other))),
        };

        // Optional grouping before args: `sum by(job) (...)`
        let grouping_before = self.try_parse_grouping()?;

        self.eat(&Token::LParen)?;

        // Param for topk/bottomk/quantile/count_values
        let param = if matches!(op, AggregateOp::Topk | AggregateOp::Bottomk | AggregateOp::Quantile | AggregateOp::CountValues) {
            let p = self.parse_expr()?;
            self.eat(&Token::Comma)?;
            Some(Box::new(p))
        } else {
            None
        };

        let inner = self.parse_expr()?;
        self.eat(&Token::RParen)?;

        // Optional grouping after args: `sum(...) by(job)`
        let grouping = if grouping_before.labels.is_empty() && !grouping_before.without {
            self.try_parse_grouping()?
        } else {
            grouping_before
        };

        Ok(Expr::Aggregate(AggregateExpr {
            op,
            expr: Box::new(inner),
            param,
            grouping,
        }))
    }

    fn try_parse_grouping(&mut self) -> Result<Grouping> {
        match self.peek() {
            Token::By | Token::Without => {
                let without = matches!(self.peek(), Token::Without);
                self.advance();
                self.eat(&Token::LParen)?;
                let mut labels = Vec::new();
                while !matches!(self.peek(), Token::RParen | Token::Eof) {
                    match self.advance().clone() {
                        Token::Ident(s) => labels.push(s),
                        _ => {}
                    }
                    if matches!(self.peek(), Token::Comma) { self.advance(); }
                }
                self.eat(&Token::RParen)?;
                Ok(Grouping { without, labels })
            }
            _ => Ok(Grouping::default()),
        }
    }

    fn parse_call(&mut self) -> Result<Expr> {
        let name = self.eat_ident()?;
        self.eat(&Token::LParen)?;
        let mut args = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            args.push(self.parse_expr()?);
            if matches!(self.peek(), Token::Comma) { self.advance(); }
        }
        self.eat(&Token::RParen)?;
        Ok(Expr::Call(CallExpr { func: name, args }))
    }

    fn parse_vector_selector(&mut self) -> Result<Expr> {
        let name = if let Token::Ident(n) = self.peek().clone() {
            self.advance();
            Some(n)
        } else {
            None
        };

        let mut matchers = Vec::new();

        // Add __name__ matcher if we have a metric name
        if let Some(ref n) = name {
            if !n.is_empty() {
                matchers.push(LabelMatcher::equal("__name__", n.as_str()));
            }
        }

        // Optional label selector block
        if matches!(self.peek(), Token::LBrace) {
            self.advance();
            while !matches!(self.peek(), Token::RBrace | Token::Eof) {
                let label_name = match self.advance().clone() {
                    Token::Ident(s) => s,
                    _ => break,
                };
                let op_tok = self.advance().clone();
                let value  = match self.advance().clone() {
                    Token::StringLit(s) => s,
                    Token::Ident(s)     => s,
                    _ => String::new(),
                };
                let matcher = match op_tok {
                    Token::EqMatch  => LabelMatcher::equal(&label_name, &value),
                    Token::Ne       => LabelMatcher::not_equal(&label_name, &value),
                    Token::ReMatch  => LabelMatcher::regex(&label_name, &value)?,
                    Token::NreMatch => LabelMatcher::not_regex(&label_name, &value)?,
                    _ => LabelMatcher::equal(&label_name, &value),
                };
                matchers.push(matcher);
                if matches!(self.peek(), Token::Comma) { self.advance(); }
            }
            self.eat(&Token::RBrace)?;
        }

        Ok(Expr::VectorSelector(VectorSelector {
            name,
            matchers,
            offset: None,
            at: None,
        }))
    }

    fn parse_vector_matching(&mut self) -> Result<Option<VectorMatching>> {
        let on = match self.peek() {
            Token::On       => true,
            Token::Ignoring => false,
            _ => return Ok(None),
        };
        self.advance();
        self.eat(&Token::LParen)?;
        let mut labels = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            if let Token::Ident(s) = self.advance().clone() { labels.push(s); }
            if matches!(self.peek(), Token::Comma) { self.advance(); }
        }
        self.eat(&Token::RParen)?;

        let card = match self.peek() {
            Token::GroupLeft  => { self.advance(); MatchCardinality::ManyToOne }
            Token::GroupRight => { self.advance(); MatchCardinality::OneToMany }
            _                 => MatchCardinality::OneToOne,
        };

        let mut include = Vec::new();
        if !matches!(card, MatchCardinality::OneToOne) && matches!(self.peek(), Token::LParen) {
            self.advance();
            while !matches!(self.peek(), Token::RParen | Token::Eof) {
                if let Token::Ident(s) = self.advance().clone() { include.push(s); }
                if matches!(self.peek(), Token::Comma) { self.advance(); }
            }
            self.eat(&Token::RParen)?;
        }

        Ok(Some(VectorMatching { card, on, labels, include }))
    }

    fn parse_duration(&mut self) -> Result<i64> {
        match self.advance().clone() {
            Token::Duration(ms) => Ok(ms),
            Token::Number(n)    => Ok((n * 1000.0) as i64),
            other => Err(MetricsError::Parse(format!("expected duration, got {:?}", other))),
        }
    }
}

fn apply_offset(expr: Expr, offset_ms: i64) -> Expr {
    match expr {
        Expr::VectorSelector(mut vs) => { vs.offset = Some(offset_ms); Expr::VectorSelector(vs) }
        Expr::MatrixSelector(mut ms) => { ms.offset = Some(offset_ms); Expr::MatrixSelector(ms) }
        Expr::Subquery(mut sq)       => { sq.offset = Some(offset_ms); Expr::Subquery(sq) }
        other => other,
    }
}

fn apply_at(expr: Expr, ts_ms: i64) -> Expr {
    match expr {
        Expr::VectorSelector(mut vs) => { vs.at = Some(ts_ms); Expr::VectorSelector(vs) }
        Expr::MatrixSelector(mut ms) => { ms.at = Some(ts_ms); Expr::MatrixSelector(ms) }
        Expr::Subquery(mut sq)       => { sq.at = Some(ts_ms); Expr::Subquery(sq) }
        other => other,
    }
}

/// Parse a complete PromQL expression string.
pub fn parse(input: &str) -> Result<Expr> {
    Parser::new(input).parse_expr()
}
