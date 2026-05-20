// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego parser — converts token stream into AST.

use super::ast::*;
use super::lexer::{Token, tokenize};
use crate::error::PolicyError;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        // Strip newlines that appear outside of contexts where they matter,
        // keeping them only as separators in rule bodies handled by parse_body.
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek_at(&self, offset: usize) -> &Token {
        self.tokens.get(self.pos + offset).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> &Token {
        let t = self.tokens.get(self.pos).unwrap_or(&Token::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Token::Newline) {
            self.advance();
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), PolicyError> {
        self.skip_newlines();
        let tok = self.peek().clone();
        if tok == *expected {
            self.advance();
            Ok(())
        } else {
            Err(PolicyError::Parse(format!(
                "expected {expected:?}, got {tok:?}"
            )))
        }
    }

    fn expect_ident(&mut self) -> Result<String, PolicyError> {
        self.skip_newlines();
        match self.peek().clone() {
            Token::Ident(s) => {
                self.advance();
                Ok(s)
            }
            Token::Data => {
                self.advance();
                Ok("data".into())
            }
            Token::Input => {
                self.advance();
                Ok("input".into())
            }
            Token::Future => {
                self.advance();
                Ok("future".into())
            }
            other => Err(PolicyError::Parse(format!(
                "expected identifier, got {other:?}"
            ))),
        }
    }

    // ─── Module ───────────────────────────────────────────────────────────────

    pub fn parse_module(&mut self) -> Result<Module, PolicyError> {
        self.skip_newlines();
        let package = self.parse_package()?;
        let mut imports = Vec::new();
        let mut rules = Vec::new();

        loop {
            self.skip_newlines();
            match self.peek().clone() {
                Token::Eof => break,
                Token::Import => {
                    imports.push(self.parse_import()?);
                }
                _ => {
                    rules.push(self.parse_rule()?);
                }
            }
        }

        Ok(Module {
            package,
            imports,
            rules,
            comments: vec![],
        })
    }

    fn parse_package(&mut self) -> Result<Package, PolicyError> {
        self.expect(&Token::Package)?;
        let path = self.parse_ref_path()?;
        Ok(Package { path })
    }

    fn parse_import(&mut self) -> Result<Import, PolicyError> {
        self.expect(&Token::Import)?;
        let path = self.parse_ref_path()?;
        let alias = if matches!(self.peek(), Token::As) {
            self.advance();
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(Import { path, alias })
    }

    /// Parse a dotted path like `data.foo.bar` or `foo.bar`.
    fn parse_ref_path(&mut self) -> Result<Vec<String>, PolicyError> {
        let mut path = vec![self.expect_ident()?];
        while matches!(self.peek(), Token::Dot) {
            self.advance();
            path.push(self.expect_ident()?);
        }
        Ok(path)
    }

    // ─── Rules ────────────────────────────────────────────────────────────────

    fn parse_rule(&mut self) -> Result<Rule, PolicyError> {
        self.skip_newlines();

        // Check for default keyword
        let is_default = if matches!(self.peek(), Token::Default) {
            self.advance();
            true
        } else {
            false
        };

        let head = self.parse_rule_head()?;

        if is_default {
            // Default rules have no body
            return Ok(Rule {
                is_default: true,
                head,
                bodies: vec![],
                else_rules: vec![],
                annotations: vec![],
            });
        }

        // Parse bodies. Each body is delimited by `{` ... `}`.
        // A rule can have multiple bodies (disjunction).
        let mut bodies = Vec::new();
        let mut else_rules = Vec::new();

        // A rule can be:
        //   head { body }
        //   head if { body }
        //   head { body } { body }   (multiple bodies = OR)
        //   head := value (no body)
        //   head contains term { body }

        while matches!(self.peek(), Token::LBrace) || matches!(self.peek(), Token::If) {
            if matches!(self.peek(), Token::If) {
                self.advance(); // consume `if`
            }
            self.expect(&Token::LBrace)?;
            let body = self.parse_body()?;
            self.expect(&Token::RBrace)?;
            bodies.push(body);
            self.skip_newlines();
        }

        // Parse else clauses
        while matches!(self.peek(), Token::Else) {
            self.advance();
            let else_value = if matches!(self.peek(), Token::Unify | Token::Assign) {
                self.advance();
                Some(self.parse_term()?)
            } else {
                None
            };
            self.skip_newlines();
            let else_body = if matches!(self.peek(), Token::LBrace) {
                self.advance();
                let b = self.parse_body()?;
                self.expect(&Token::RBrace)?;
                b
            } else {
                vec![]
            };
            else_rules.push(ElseRule {
                value: else_value,
                body: else_body,
            });
        }

        Ok(Rule {
            is_default,
            head,
            bodies,
            else_rules,
            annotations: vec![],
        })
    }

    fn parse_rule_head(&mut self) -> Result<RuleHead, PolicyError> {
        let name = self.expect_ident()?;

        // Check for function args: `f(a, b)`
        let args = if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut args = Vec::new();
            while !matches!(self.peek(), Token::RParen | Token::Eof) {
                args.push(self.parse_term()?);
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                }
            }
            self.expect(&Token::RParen)?;
            args
        } else {
            vec![]
        };

        // Check for set/object key: `violations[msg]`
        let key = if matches!(self.peek(), Token::LBracket) {
            self.advance();
            let k = if matches!(self.peek(), Token::RBracket) {
                None
            } else {
                Some(self.parse_term()?)
            };
            self.expect(&Token::RBracket)?;
            k
        } else {
            None
        };

        // Check for `contains` keyword
        let contains = if matches!(self.peek(), Token::Contains) {
            self.advance();
            true
        } else {
            false
        };

        // Check for value assignment: `= val` or `:= val`
        let value = if matches!(self.peek(), Token::Unify | Token::Assign) {
            self.advance();
            Some(self.parse_term()?)
        } else {
            None
        };

        Ok(RuleHead {
            name,
            args,
            key,
            value,
            contains,
        })
    }

    // ─── Body ─────────────────────────────────────────────────────────────────

    fn parse_body(&mut self) -> Result<Body, PolicyError> {
        let mut exprs = Vec::new();
        self.skip_newlines();

        loop {
            match self.peek() {
                Token::RBrace | Token::Eof => break,
                Token::Semicolon | Token::Newline => {
                    self.advance();
                    self.skip_newlines();
                    if matches!(self.peek(), Token::RBrace | Token::Eof) {
                        break;
                    }
                }
                _ => {
                    exprs.push(self.parse_expr()?);
                    self.skip_newlines();
                }
            }
        }
        Ok(exprs)
    }

    // ─── Expressions ─────────────────────────────────────────────────────────

    fn parse_expr(&mut self) -> Result<Expr, PolicyError> {
        // Check for leading keywords
        let expr = match self.peek().clone() {
            Token::Not => {
                self.advance();
                if matches!(self.peek(), Token::LBrace) {
                    self.advance();
                    let body = self.parse_body()?;
                    self.expect(&Token::RBrace)?;
                    Expr::NotBody(body)
                } else {
                    let inner = self.parse_expr()?;
                    Expr::Not(Box::new(inner))
                }
            }
            Token::Some => {
                self.advance();
                self.parse_some_expr()?
            }
            Token::Every => {
                self.advance();
                self.parse_every_expr()?
            }
            _ => {
                // Could be: term, term = term, term := term, or a call
                let left = self.parse_term()?;
                match self.peek() {
                    Token::Unify => {
                        self.advance();
                        let right = self.parse_term()?;
                        Expr::Unify(left, right)
                    }
                    Token::Assign => {
                        self.advance();
                        let right = self.parse_term()?;
                        Expr::Assign(left, right)
                    }
                    _ => Expr::Term(left),
                }
            }
        };

        // Check for `with` modifier
        if matches!(self.peek(), Token::With) {
            let mut targets = Vec::new();
            while matches!(self.peek(), Token::With) {
                self.advance();
                let path = self.parse_ref_path()?;
                self.expect(&Token::As)?;
                let value = self.parse_term()?;
                targets.push(WithTarget { path, value });
            }
            Ok(Expr::With {
                base: Box::new(expr),
                targets,
            })
        } else {
            Ok(expr)
        }
    }

    fn parse_some_expr(&mut self) -> Result<Expr, PolicyError> {
        // `some x, y` OR `some k, v in domain` OR `some v in domain`
        let mut vars = Vec::new();
        let first = self.parse_term()?;

        if matches!(self.peek(), Token::Comma) {
            // Collect more vars
            vars.push(term_to_var(&first)?);
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                let t = self.parse_term()?;
                vars.push(term_to_var(&t)?);
            }
            if matches!(self.peek(), Token::In) {
                self.advance();
                let domain = self.parse_term()?;
                // some k, v in domain
                let key = vars.remove(0);
                let value = vars.remove(0);
                return Ok(Expr::SomeIn {
                    key: Some(Term::Var(key)),
                    value: Term::Var(value),
                    domain,
                });
            }
            return Ok(Expr::Some(vars));
        }

        if matches!(self.peek(), Token::In) {
            self.advance();
            let domain = self.parse_term()?;
            return Ok(Expr::SomeIn {
                key: None,
                value: first,
                domain,
            });
        }

        vars.push(term_to_var(&first)?);
        Ok(Expr::Some(vars))
    }

    fn parse_every_expr(&mut self) -> Result<Expr, PolicyError> {
        // `every v in domain { body }`
        // `every k, v in domain { body }`
        let first_var = self.expect_ident()?;
        let (key, value) = if matches!(self.peek(), Token::Comma) {
            self.advance();
            let v = self.expect_ident()?;
            (Some(first_var), v)
        } else {
            (None, first_var)
        };
        self.expect(&Token::In)?;
        let domain = self.parse_term()?;
        self.skip_newlines();
        self.expect(&Token::LBrace)?;
        let body = self.parse_body()?;
        self.expect(&Token::RBrace)?;
        Ok(Expr::Every {
            key,
            value,
            domain,
            body,
        })
    }

    // ─── Terms ────────────────────────────────────────────────────────────────

    pub fn parse_term(&mut self) -> Result<Term, PolicyError> {
        self.skip_newlines();
        // Handle unary minus for numbers
        if matches!(self.peek(), Token::Minus) {
            let next = self.peek_at(1);
            if matches!(next, Token::Number(_)) {
                self.advance(); // consume minus
                if let Token::Number(n) = self.advance().clone() {
                    return Ok(Term::Number(format!("-{n}")));
                }
            }
        }

        let base = self.parse_primary_term()?;
        self.parse_ref_suffix(base)
    }

    fn parse_primary_term(&mut self) -> Result<Term, PolicyError> {
        match self.peek().clone() {
            Token::Null => {
                self.advance();
                Ok(Term::Null)
            }
            Token::True => {
                self.advance();
                Ok(Term::Bool(true))
            }
            Token::False => {
                self.advance();
                Ok(Term::Bool(false))
            }
            Token::Number(n) => {
                self.advance();
                Ok(Term::Number(n))
            }
            Token::String(s) => {
                self.advance();
                Ok(Term::String(s))
            }
            Token::Underscore => {
                self.advance();
                Ok(Term::Wildcard)
            }
            Token::LBracket => self.parse_array_or_compr(),
            Token::LBrace => self.parse_object_or_set_or_compr(),
            Token::LParen => {
                self.advance();
                let t = self.parse_term()?;
                self.expect(&Token::RParen)?;
                Ok(t)
            }
            Token::Data => {
                self.advance();
                Ok(Term::Var("data".into()))
            }
            Token::Input => {
                self.advance();
                Ok(Term::Var("input".into()))
            }
            Token::Future => {
                self.advance();
                Ok(Term::Var("future".into()))
            }
            Token::Ident(name) => {
                self.advance();
                Ok(Term::Var(name))
            }
            // Keywords can appear as identifiers in ref position
            Token::Not | Token::Some | Token::Every | Token::Every => {
                let name = format!("{:?}", self.advance()).to_lowercase();
                Ok(Term::Var(name))
            }
            other => Err(PolicyError::Parse(format!(
                "unexpected token in term: {other:?}"
            ))),
        }
    }

    fn parse_ref_suffix(&mut self, mut base: Term) -> Result<Term, PolicyError> {
        let mut args: Vec<RefArg> = Vec::new();
        let mut is_call = false;

        loop {
            match self.peek() {
                Token::Dot => {
                    self.advance();
                    let field = self.expect_ident()?;
                    args.push(RefArg::Field(field));
                }
                Token::LBracket => {
                    self.advance();
                    if matches!(self.peek(), Token::RBracket) {
                        self.advance();
                        args.push(RefArg::Index(Term::Wildcard));
                    } else {
                        let idx = self.parse_term()?;
                        self.expect(&Token::RBracket)?;
                        args.push(RefArg::Index(idx));
                    }
                }
                Token::LParen => {
                    // Function call: wrap what we have as Ref, then make it a Call
                    is_call = true;
                    self.advance();
                    let mut call_args = Vec::new();
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        call_args.push(self.parse_term()?);
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                        }
                    }
                    self.expect(&Token::RParen)?;
                    let func = if args.is_empty() {
                        Box::new(base)
                    } else {
                        Box::new(Term::Ref(Box::new(base), args.clone()))
                    };
                    return Ok(Term::Call {
                        func,
                        args: call_args,
                    });
                }
                _ => break,
            }
        }

        if is_call {
            unreachable!()
        }

        if args.is_empty() {
            Ok(base)
        } else {
            Ok(Term::Ref(Box::new(base), args))
        }
    }

    fn parse_array_or_compr(&mut self) -> Result<Term, PolicyError> {
        self.expect(&Token::LBracket)?;
        self.skip_newlines();
        if matches!(self.peek(), Token::RBracket) {
            self.advance();
            return Ok(Term::Array(vec![]));
        }
        let first = self.parse_term()?;
        self.skip_newlines();

        // Array comprehension: [term | body]
        if matches!(self.peek(), Token::Or) {
            self.advance();
            let body = self.parse_body()?;
            self.expect(&Token::RBracket)?;
            return Ok(Term::ArrayCompr {
                term: Box::new(first),
                body,
            });
        }

        // Regular array
        let mut items = vec![first];
        while matches!(self.peek(), Token::Comma) {
            self.advance();
            self.skip_newlines();
            if matches!(self.peek(), Token::RBracket) {
                break;
            }
            items.push(self.parse_term()?);
        }
        self.skip_newlines();
        self.expect(&Token::RBracket)?;
        Ok(Term::Array(items))
    }

    fn parse_object_or_set_or_compr(&mut self) -> Result<Term, PolicyError> {
        self.expect(&Token::LBrace)?;
        self.skip_newlines();

        if matches!(self.peek(), Token::RBrace) {
            self.advance();
            return Ok(Term::Object(vec![]));
        }

        let first = self.parse_term()?;
        self.skip_newlines();

        match self.peek() {
            // Object: { key: value, ... }
            Token::Colon => {
                self.advance();
                let val = self.parse_term()?;
                self.skip_newlines();

                // Object comprehension: { k: v | body }
                if matches!(self.peek(), Token::Or) {
                    self.advance();
                    let body = self.parse_body()?;
                    self.expect(&Token::RBrace)?;
                    return Ok(Term::ObjectCompr {
                        key: Box::new(first),
                        value: Box::new(val),
                        body,
                    });
                }

                let mut kvs = vec![(first, val)];
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    self.skip_newlines();
                    if matches!(self.peek(), Token::RBrace) {
                        break;
                    }
                    let k = self.parse_term()?;
                    self.expect(&Token::Colon)?;
                    let v = self.parse_term()?;
                    kvs.push((k, v));
                    self.skip_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Term::Object(kvs))
            }
            // Set comprehension: { term | body }
            Token::Or => {
                self.advance();
                let body = self.parse_body()?;
                self.expect(&Token::RBrace)?;
                Ok(Term::SetCompr {
                    term: Box::new(first),
                    body,
                })
            }
            // Set: { a, b, c }
            Token::Comma => {
                let mut items = vec![first];
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    self.skip_newlines();
                    if matches!(self.peek(), Token::RBrace) {
                        break;
                    }
                    items.push(self.parse_term()?);
                    self.skip_newlines();
                }
                self.expect(&Token::RBrace)?;
                Ok(Term::Set(items))
            }
            Token::RBrace => {
                self.advance();
                // Single-element set
                Ok(Term::Set(vec![first]))
            }
            _ => {
                self.expect(&Token::RBrace)?;
                Ok(Term::Set(vec![first]))
            }
        }
    }
}

fn term_to_var(t: &Term) -> Result<String, PolicyError> {
    match t {
        Term::Var(v) => Ok(v.clone()),
        other => Err(PolicyError::Parse(format!(
            "expected variable, got: {other}"
        ))),
    }
}

/// Parse a Rego module from source text.
pub fn parse_module(src: &str) -> Result<Module, PolicyError> {
    let tokens: Vec<Token> = tokenize(src)?.into_iter().map(|(t, _)| t).collect();
    Parser::new(tokens).parse_module()
}

/// Parse a Rego query (list of exprs) from source text.
pub fn parse_query(src: &str) -> Result<Body, PolicyError> {
    let tokens: Vec<Token> = tokenize(src)?.into_iter().map(|(t, _)| t).collect();
    let mut p = Parser::new(tokens);
    let mut body = Vec::new();
    loop {
        p.skip_newlines();
        match p.peek() {
            Token::Eof => break,
            Token::Semicolon => {
                p.advance();
            }
            _ => body.push(p.parse_expr()?),
        }
    }
    Ok(body)
}
