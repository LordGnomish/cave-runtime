// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Recursive-descent parser for the Rego-compatible policy language.

use super::ast::*;
use super::lexer::{lex, Token};
use serde_json::Value;

pub fn parse_policy(src: &str) -> Result<Policy, String> {
    let tokens = lex(src)?;
    let mut p = Parser::new(tokens);
    p.parse_policy()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        if t != Token::Eof { self.pos += 1; }
        t
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            t => Err(format!("expected identifier, got {t:?}")),
        }
    }

    fn skip_newlines(&mut self) {
        while self.peek() == &Token::Newline { self.advance(); }
    }

    fn skip_nl_and_semi(&mut self) {
        while matches!(self.peek(), Token::Newline | Token::Semicolon) { self.advance(); }
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.peek() == tok {
            self.advance();
            true
        } else {
            false
        }
    }

    // ── Top-level ─────────────────────────────────────────────────────────────

    fn parse_policy(&mut self) -> Result<Policy, String> {
        self.skip_newlines();
        let package = self.parse_package()?;
        self.skip_nl_and_semi();
        let mut imports = Vec::new();
        while self.peek() == &Token::Import {
            imports.push(self.parse_import()?);
            self.skip_nl_and_semi();
        }
        let mut rules = Vec::new();
        while self.peek() != &Token::Eof {
            self.skip_nl_and_semi();
            if self.peek() == &Token::Eof { break; }
            rules.push(self.parse_rule()?);
        }
        Ok(Policy { package, imports, rules })
    }

    fn parse_package(&mut self) -> Result<Vec<String>, String> {
        match self.advance() {
            Token::Package => {}
            t => return Err(format!("expected 'package', got {t:?}")),
        }
        self.parse_ref_path()
    }

    fn parse_import(&mut self) -> Result<Import, String> {
        match self.advance() {
            Token::Import => {}
            t => return Err(format!("expected 'import', got {t:?}")),
        }
        let path = self.parse_ref_path()?;
        let alias = if self.eat(&Token::As) {
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(Import { path, alias })
    }

    fn parse_ref_path(&mut self) -> Result<Vec<String>, String> {
        let mut parts = vec![self.expect_ident()?];
        while self.peek() == &Token::Dot {
            self.advance();
            parts.push(self.expect_ident()?);
        }
        Ok(parts)
    }

    // ── Rules ─────────────────────────────────────────────────────────────────

    fn parse_rule(&mut self) -> Result<Rule, String> {
        // default rule
        if self.peek() == &Token::Default {
            return self.parse_default_rule();
        }

        let name = self.expect_ident()?;

        // Partial rule: name[key]
        let head_key = if self.peek() == &Token::LBracket {
            self.advance();
            let k = self.parse_expr()?;
            if self.peek() != &Token::RBracket {
                return Err("expected ']' after partial rule key".into());
            }
            self.advance();
            Some(k)
        } else {
            None
        };

        // Head value: name = expr
        let head_value = if self.peek() == &Token::Eq {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        // Body
        let body = if self.peek() == &Token::LBrace {
            self.advance(); // consume '{'
            self.skip_newlines();
            let mut stmts = Vec::new();
            while self.peek() != &Token::RBrace && self.peek() != &Token::Eof {
                stmts.push(self.parse_statement()?);
                self.skip_nl_and_semi();
            }
            if !self.eat(&Token::RBrace) {
                return Err("expected '}' to close rule body".into());
            }
            stmts
        } else {
            Vec::new()
        };

        Ok(Rule {
            name,
            head_key,
            head_value,
            body,
            is_default: false,
            default_value: None,
        })
    }

    fn parse_default_rule(&mut self) -> Result<Rule, String> {
        self.advance(); // consume 'default'
        let name = self.expect_ident()?;
        // optional := or =
        if !self.eat(&Token::ColonEq) && !self.eat(&Token::Eq) {
            return Err("expected ':=' or '=' after default rule name".into());
        }
        let val = self.parse_literal_value()?;
        Ok(Rule {
            name,
            head_key: None,
            head_value: None,
            body: Vec::new(),
            is_default: true,
            default_value: Some(val),
        })
    }

    fn parse_literal_value(&mut self) -> Result<Value, String> {
        match self.advance() {
            Token::True  => Ok(Value::Bool(true)),
            Token::False => Ok(Value::Bool(false)),
            Token::Null  => Ok(Value::Null),
            Token::Number(n) => Ok(Value::Number(
                serde_json::Number::from_f64(n).unwrap_or(serde_json::Number::from(0)),
            )),
            Token::Str(s) => Ok(Value::String(s)),
            t => Err(format!("expected literal, got {t:?}")),
        }
    }

    // ── Statements ────────────────────────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Expr, String> {
        // `not expr`
        if self.peek() == &Token::Not {
            self.advance();
            let e = self.parse_expr()?;
            return Ok(Expr::Not(Box::new(e)));
        }
        // `some vars`
        if self.peek() == &Token::Some {
            self.advance();
            let name = self.expect_ident()?;
            return Ok(Expr::Var(name)); // declare variable (no-op in eval, just for documentation)
        }

        let lhs = self.parse_expr()?;

        // `lhs := rhs`
        if self.peek() == &Token::ColonEq {
            self.advance();
            let rhs = self.parse_expr()?;
            // LHS must be a Var for :=
            if let Expr::Var(name) = lhs {
                return Ok(Expr::Assign { name, value: Box::new(rhs) });
            }
            return Ok(Expr::Unify { left: Box::new(lhs), right: Box::new(rhs) });
        }

        // `lhs = rhs`  (unification)
        if self.peek() == &Token::Eq {
            self.advance();
            let rhs = self.parse_expr()?;
            return Ok(Expr::Unify { left: Box::new(lhs), right: Box::new(rhs) });
        }

        Ok(lhs)
    }

    // ── Expressions ───────────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let left = self.parse_additive()?;
        let op = match self.peek() {
            Token::EqEq  => CmpOp::Eq,
            Token::BangEq => CmpOp::Ne,
            Token::Lt    => CmpOp::Lt,
            Token::LtEq  => CmpOp::Le,
            Token::Gt    => CmpOp::Gt,
            Token::GtEq  => CmpOp::Ge,
            _ => return Ok(left),
        };
        self.advance();
        let right = self.parse_additive()?;
        Ok(Expr::Cmp { op, left: Box::new(left), right: Box::new(right) })
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus  => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star    => BinOp::Mul,
                Token::Slash   => BinOp::Div,
                Token::Percent => BinOp::Div, // treat % as modulo (simplified)
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.peek() == &Token::Not {
            self.advance();
            let e = self.parse_postfix()?;
            return Ok(Expr::Not(Box::new(e)));
        }
        if self.peek() == &Token::Minus {
            self.advance();
            let e = self.parse_postfix()?;
            // negation: wrap in BinOp(Sub, 0, e)
            return Ok(Expr::BinOp {
                op: BinOp::Sub,
                left: Box::new(Expr::Lit(Value::Number(serde_json::Number::from(0)))),
                right: Box::new(e),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut base = self.parse_primary()?;
        loop {
            match self.peek() {
                Token::Dot => {
                    self.advance();
                    let key = self.expect_ident()?;
                    base = Expr::Ref {
                        head: Box::new(base),
                        parts: vec![RefPart::Key(key)],
                    };
                }
                Token::LBracket => {
                    self.advance();
                    if self.peek() == &Token::Underscore {
                        self.advance();
                        if !self.eat(&Token::RBracket) {
                            return Err("expected ']'".into());
                        }
                        base = Expr::Ref {
                            head: Box::new(base),
                            parts: vec![RefPart::AnyIndex],
                        };
                    } else {
                        let idx = self.parse_expr()?;
                        if !self.eat(&Token::RBracket) {
                            return Err("expected ']'".into());
                        }
                        base = Expr::Ref {
                            head: Box::new(base),
                            parts: vec![RefPart::Index(Box::new(idx))],
                        };
                    }
                }
                Token::LParen => {
                    // function call: base(args)
                    let func_name = expr_to_func_name(&base)
                        .ok_or_else(|| "invalid function call target".to_string())?;
                    self.advance(); // consume '('
                    let args = self.parse_call_args()?;
                    if !self.eat(&Token::RParen) {
                        return Err("expected ')' after call args".into());
                    }
                    base = Expr::Call { func: func_name, args };
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if self.peek() == &Token::RParen { return Ok(args); }
        args.push(self.parse_expr()?);
        while self.eat(&Token::Comma) {
            args.push(self.parse_expr()?);
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::True  => { self.advance(); Ok(Expr::Lit(Value::Bool(true))) }
            Token::False => { self.advance(); Ok(Expr::Lit(Value::Bool(false))) }
            Token::Null  => { self.advance(); Ok(Expr::Lit(Value::Null)) }
            Token::Number(n) => { self.advance(); Ok(Expr::Lit(Value::Number(
                serde_json::Number::from_f64(n).unwrap_or(serde_json::Number::from(0))
            ))) }
            Token::Str(s) => { self.advance(); Ok(Expr::Lit(Value::String(s))) }
            Token::Ident(name) => {
                self.advance();
                Ok(Expr::Var(name))
            }
            Token::Underscore => {
                self.advance();
                Ok(Expr::Var("_".to_string()))
            }
            Token::LParen => {
                self.advance();
                let e = self.parse_expr()?;
                if !self.eat(&Token::RParen) {
                    return Err("expected ')'".into());
                }
                Ok(e)
            }
            Token::LBracket => {
                self.advance();
                self.parse_array_or_comprehension()
            }
            Token::LBrace => {
                self.advance();
                self.parse_object_or_set_or_comprehension()
            }
            t => Err(format!("unexpected token in expression: {t:?}")),
        }
    }

    fn parse_array_or_comprehension(&mut self) -> Result<Expr, String> {
        // empty array
        if self.eat(&Token::RBracket) {
            return Ok(Expr::Array(Vec::new()));
        }
        let first = self.parse_expr()?;
        // comprehension: [term | body]
        if self.eat(&Token::Pipe) {
            let body = self.parse_body_stmts(&Token::RBracket)?;
            if !self.eat(&Token::RBracket) {
                return Err("expected ']' to close array comprehension".into());
            }
            return Ok(Expr::ArrayComp { term: Box::new(first), body });
        }
        // regular array
        let mut items = vec![first];
        while self.eat(&Token::Comma) {
            if self.peek() == &Token::RBracket { break; }
            items.push(self.parse_expr()?);
        }
        if !self.eat(&Token::RBracket) {
            return Err("expected ']' to close array".into());
        }
        Ok(Expr::Array(items))
    }

    fn parse_object_or_set_or_comprehension(&mut self) -> Result<Expr, String> {
        // empty object
        if self.eat(&Token::RBrace) {
            return Ok(Expr::Object(Vec::new()));
        }
        let first = self.parse_expr()?;
        // set comprehension: {term | body}
        if self.eat(&Token::Pipe) {
            let body = self.parse_body_stmts(&Token::RBrace)?;
            if !self.eat(&Token::RBrace) {
                return Err("expected '}' to close set comprehension".into());
            }
            return Ok(Expr::SetComp { term: Box::new(first), body });
        }
        // object: {key: value, ...}
        if self.eat(&Token::Colon) {
            let val = self.parse_expr()?;
            // object comprehension: {key: value | body}
            if self.eat(&Token::Pipe) {
                let body = self.parse_body_stmts(&Token::RBrace)?;
                if !self.eat(&Token::RBrace) {
                    return Err("expected '}' to close object comprehension".into());
                }
                return Ok(Expr::ObjectComp {
                    key: Box::new(first),
                    value: Box::new(val),
                    body,
                });
            }
            let mut pairs = vec![(first, val)];
            while self.eat(&Token::Comma) {
                if self.peek() == &Token::RBrace { break; }
                let k = self.parse_expr()?;
                if !self.eat(&Token::Colon) {
                    return Err("expected ':' in object literal".into());
                }
                let v = self.parse_expr()?;
                pairs.push((k, v));
            }
            if !self.eat(&Token::RBrace) {
                return Err("expected '}' to close object".into());
            }
            return Ok(Expr::Object(pairs));
        }
        // set literal: {first, rest...}
        if self.eat(&Token::Comma) || self.peek() == &Token::RBrace {
            let mut items = vec![first];
            while self.eat(&Token::Comma) {
                if self.peek() == &Token::RBrace { break; }
                items.push(self.parse_expr()?);
            }
            if !self.eat(&Token::RBrace) {
                return Err("expected '}' to close set".into());
            }
            return Ok(Expr::Set(items));
        }
        // single-element set
        if self.eat(&Token::RBrace) {
            return Ok(Expr::Set(vec![first]));
        }
        Err("unexpected token in braced expression".into())
    }

    fn parse_body_stmts(&mut self, end: &Token) -> Result<Vec<Expr>, String> {
        let mut stmts = Vec::new();
        self.skip_nl_and_semi();
        while self.peek() != end && self.peek() != &Token::Eof {
            stmts.push(self.parse_statement()?);
            self.skip_nl_and_semi();
        }
        Ok(stmts)
    }
}

fn expr_to_func_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Var(name) => Some(name.clone()),
        Expr::Ref { head, parts } => {
            let mut base = expr_to_func_name(head)?;
            for p in parts {
                match p {
                    RefPart::Key(k) => { base.push('.'); base.push_str(k); }
                    _ => return None,
                }
            }
            Some(base)
        }
        _ => None,
    }
}
