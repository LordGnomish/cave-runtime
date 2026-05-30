// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Recursive-descent parser for the Alloy configuration syntax.
//!
//! Line-ported from grafana/alloy `syntax/parser/internal.go` (v1.5.0,
//! Apache-2.0). Produces an [`ast::File`] plus a list of [`Diagnostic`]s.
//!
//! Expression precedence is handled by precedence-climbing in [`Parser::parse_bin_op`]
//! exactly as upstream, with the right-associative `^` peeled off in
//! [`Parser::parse_pow_expr`]. Comment-group attachment (the upstream
//! `consumeCommentGroup` doc-comment machinery) is intentionally omitted; it
//! affects formatting, not parse structure.

use super::ast::{
    ArrayExpr, AccessExpr, AttributeStmt, BinaryExpr, BlockStmt, Body, CallExpr, Expr, File, Ident,
    IdentifierExpr, IndexExpr, LiteralExpr, ObjectExpr, ObjectField, ParenExpr, Stmt, UnaryExpr,
};
use super::scanner::{is_valid_identifier, Pos, Scanner};
use super::token::Token;

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A hard error.
    Error,
}

/// A parser diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Position the diagnostic refers to.
    pub pos: Pos,
    /// Human-readable message.
    pub message: String,
}

/// Parses a whole Alloy config `File`. Returns the (possibly partial) file and
/// any diagnostics produced. Mirrors `parser.ParseFile`.
pub fn parse_file(name: &str, src: &str) -> (File, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let body = p.parse_body(Token::Eof);
    (
        File { name: name.to_string(), body, comments: Vec::new() },
        p.diags,
    )
}

/// Parses a single expression. Mirrors `parser.ParseExpression`.
pub fn parse_expression(src: &str) -> (Expr, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let expr = p.parse_expression();
    (expr, p.diags)
}

struct Parser {
    scanner: Scanner,
    pos: Pos,
    tok: Token,
    lit: String,
    diags: Vec<Diagnostic>,
}

impl Parser {
    fn new(src: &str) -> Parser {
        let scanner = Scanner::new(src);
        let mut p = Parser {
            scanner,
            pos: Pos { offset: 0, line: 1, column: 1 },
            tok: Token::Illegal,
            lit: String::new(),
            diags: Vec::new(),
        };
        p.next();
        p
    }

    fn next(&mut self) {
        let (pos, tok, lit) = self.scanner.scan();
        self.pos = pos;
        self.tok = tok;
        self.lit = lit;
    }

    fn add_error(&mut self, msg: impl Into<String>) {
        self.diags.push(Diagnostic {
            severity: Severity::Error,
            pos: self.pos,
            message: msg.into(),
        });
    }

    /// Consumes tokens until `to` (or EOF). Mirrors `advance`.
    fn advance(&mut self, to: Token) {
        while self.tok != to && self.tok != Token::Eof {
            self.next();
        }
    }

    /// Consumes tokens until any token in `to` (or EOF). Mirrors `advanceAny`.
    fn advance_any(&mut self, to: &[Token]) {
        while !to.contains(&self.tok) && self.tok != Token::Eof {
            self.next();
        }
    }

    /// Asserts the current token is `t`, emitting an error if not, then
    /// advances. Returns the position/token/lit of the token that was current.
    fn expect(&mut self, t: Token) -> (Pos, Token, String) {
        let cur = (self.pos, self.tok, self.lit.clone());
        if self.tok != t {
            self.add_error(format!("expected {}, got {}", t, self.tok));
        }
        self.next();
        cur
    }

    /// `Body = [ Statement { terminator Statement } ]`. Mirrors `parseBody`.
    fn parse_body(&mut self, until: Token) -> Body {
        let mut body: Body = Vec::new();
        while self.tok != until && self.tok != Token::Eof {
            if let Some(stmt) = self.parse_statement() {
                body.push(stmt);
            }
            if self.tok == until {
                break;
            }
            if self.tok != Token::Terminator {
                self.add_error(format!("expected {}, got {}", Token::Terminator, self.tok));
                self.consume_statement();
            }
            self.next();
        }
        body
    }

    /// Consumes the remainder of a statement, balancing `{}`, `[]`, `()`.
    /// Mirrors `consumeStatement`.
    fn consume_statement(&mut self) {
        let (mut curly, mut brack, mut paren) = (0i32, 0i32, 0i32);
        while self.tok != Token::Eof {
            match self.tok {
                Token::LCurly => curly += 1,
                Token::RCurly => curly -= 1,
                Token::LBrack => brack += 1,
                Token::RBrack => brack -= 1,
                Token::LParen => paren += 1,
                Token::RParen => paren -= 1,
                _ => {}
            }
            if self.tok == Token::Terminator && curly <= 0 && brack <= 0 && paren <= 0 {
                return;
            }
            self.next();
        }
    }

    /// `Statement = Attribute | Block`. Mirrors `parseStatement`.
    fn parse_statement(&mut self) -> Option<Stmt> {
        let block_name = match self.parse_block_name() {
            Some(bn) => bn,
            None => {
                self.advance(Token::Ident);
                return None;
            }
        };

        match self.tok {
            Token::Assign => {
                self.next(); // consume "="
                if block_name.fragments.len() != 1 {
                    self.diags.push(Diagnostic {
                        severity: Severity::Error,
                        pos: block_name.start,
                        message: "attribute names may only consist of a single identifier with no \".\"".into(),
                    });
                } else if block_name.label_pos.is_some() {
                    self.diags.push(Diagnostic {
                        severity: Severity::Error,
                        pos: block_name.label_pos.unwrap(),
                        message: "attribute names may not have labels".into(),
                    });
                }
                let value = self.parse_expression();
                Some(Stmt::Attribute(AttributeStmt {
                    name: Ident { name: block_name.fragments[0].clone(), name_pos: block_name.start },
                    value,
                }))
            }
            Token::LCurly => {
                let (lcurly_pos, _, _) = self.expect(Token::LCurly);
                let body = self.parse_body(Token::RCurly);
                let (rcurly_pos, _, _) = self.expect(Token::RCurly);
                Some(Stmt::Block(BlockStmt {
                    name: block_name.fragments,
                    name_pos: block_name.start,
                    label: block_name.label,
                    label_pos: block_name.label_pos,
                    body,
                    lcurly_pos,
                    rcurly_pos,
                }))
            }
            _ => {
                if block_name.valid_attribute() {
                    self.add_error(format!(
                        "expected attribute assignment or block body, got {}",
                        self.tok
                    ));
                } else {
                    self.add_error(format!("expected block body, got {}", self.tok));
                }
                self.advance(Token::Ident);
                None
            }
        }
    }

    /// `BlockName = identifier { "." identifier } [ string ]`. Mirrors
    /// `parseBlockName`.
    fn parse_block_name(&mut self) -> Option<BlockName> {
        if self.tok != Token::Ident {
            self.add_error(format!("expected identifier, got {}", self.tok));
            return None;
        }

        let mut bn = BlockName {
            fragments: vec![self.lit.clone()],
            label: String::new(),
            start: self.pos,
            label_pos: None,
        };
        self.next();

        while self.tok == Token::Dot {
            self.next();
            if self.tok != Token::Ident {
                self.add_error(format!("expected identifier, got {}", self.tok));
            }
            bn.fragments.push(self.lit.clone());
            self.next();
        }

        if self.tok != Token::Assign && self.tok != Token::LCurly {
            if self.tok == Token::String {
                if !self.lit.starts_with('"') {
                    self.add_error(format!(
                        "expected block label to be a double quoted string, but got {:?}",
                        self.lit
                    ));
                }
                if self.lit.len() > 2 {
                    let inner = self.lit[1..self.lit.len() - 1].to_string();
                    if !is_valid_identifier(&inner) {
                        self.add_error(format!(
                            "expected block label to be a valid identifier, but got {:?}",
                            inner
                        ));
                    }
                    bn.label = inner;
                }
                bn.label_pos = Some(self.pos);
            } else {
                self.add_error(format!("expected block label, got {}", self.tok));
            }
            self.next();
        }

        Some(bn)
    }

    /// `Expression = BinOpExpr`. Mirrors `ParseExpression`.
    fn parse_expression(&mut self) -> Expr {
        self.parse_bin_op(1)
    }

    /// Precedence-climbing binary-expression parser (left-associative).
    /// Mirrors `parseBinOp`.
    fn parse_bin_op(&mut self, in_prec: i32) -> Expr {
        let mut lhs = self.parse_pow_expr();
        loop {
            let tok = self.tok;
            let pos = self.pos;
            let prec = tok.binary_precedence();
            if prec < in_prec {
                return lhs;
            }
            self.next(); // consume the operator
            let rhs = self.parse_bin_op(prec + 1);
            lhs = Expr::Binary(BinaryExpr {
                left: Box::new(lhs),
                kind: tok,
                kind_pos: pos,
                right: Box::new(rhs),
            });
        }
    }

    /// `PowExpr = UnaryExpr [ "^" PowExpr ]` (right-associative). Mirrors
    /// `parsePowExpr`.
    fn parse_pow_expr(&mut self) -> Expr {
        let lhs = self.parse_unary_expr();
        if self.tok == Token::Pow {
            let pos = self.pos;
            self.next();
            let rhs = self.parse_pow_expr();
            return Expr::Binary(BinaryExpr {
                left: Box::new(lhs),
                kind: Token::Pow,
                kind_pos: pos,
                right: Box::new(rhs),
            });
        }
        lhs
    }

    /// Mirrors `parseUnaryExpr`, including the trailing access/index/call chain.
    fn parse_unary_expr(&mut self) -> Expr {
        if is_unary_op(self.tok) {
            let op = self.tok;
            let pos = self.pos;
            self.next();
            return Expr::Unary(UnaryExpr {
                kind: op,
                kind_pos: pos,
                value: Box::new(self.parse_unary_expr()),
            });
        }

        let mut primary = self.parse_primary_expr();

        loop {
            match self.tok {
                Token::Dot => {
                    self.next();
                    let (name_pos, _, name) = self.expect(Token::Ident);
                    primary = Expr::Access(AccessExpr {
                        value: Box::new(primary),
                        name: Ident { name, name_pos },
                    });
                }
                Token::LBrack => {
                    let (lbrack, _, _) = self.expect(Token::LBrack);
                    let index = self.parse_expression();
                    let (rbrack, _, _) = self.expect(Token::RBrack);
                    primary = Expr::Index(IndexExpr {
                        value: Box::new(primary),
                        index: Box::new(index),
                        lbrack_pos: lbrack,
                        rbrack_pos: rbrack,
                    });
                }
                Token::LParen => {
                    let (lparen, _, _) = self.expect(Token::LParen);
                    let args = if self.tok != Token::RParen {
                        self.parse_expression_list(Token::RParen)
                    } else {
                        Vec::new()
                    };
                    let (rparen, _, _) = self.expect(Token::RParen);
                    primary = Expr::Call(CallExpr {
                        value: Box::new(primary),
                        args,
                        lparen_pos: lparen,
                        rparen_pos: rparen,
                    });
                }
                Token::String | Token::LCurly => {
                    // A user trying to assign a block to an attribute; consume
                    // it to recover, recording the error.
                    let start = primary.start_pos();
                    if self.tok == Token::String {
                        self.next();
                    }
                    if self.expect(Token::LCurly).1 != Token::LCurly {
                        self.consume_statement();
                        return primary;
                    }
                    self.parse_body(Token::RCurly);
                    if self.expect(Token::RCurly).1 != Token::RCurly {
                        self.consume_statement();
                        return primary;
                    }
                    self.diags.push(Diagnostic {
                        severity: Severity::Error,
                        pos: start,
                        message: "cannot use a block as an expression".into(),
                    });
                }
                _ => break,
            }
        }
        primary
    }

    /// Mirrors `parsePrimaryExpr`.
    fn parse_primary_expr(&mut self) -> Expr {
        match self.tok {
            Token::Ident => {
                let res = Expr::Identifier(IdentifierExpr {
                    ident: Ident { name: self.lit.clone(), name_pos: self.pos },
                });
                self.next();
                res
            }
            Token::String | Token::Number | Token::Float | Token::Bool | Token::Null => {
                let res = Expr::Literal(LiteralExpr {
                    kind: self.tok,
                    value: self.lit.clone(),
                    value_pos: self.pos,
                });
                self.next();
                res
            }
            Token::LParen => {
                let (lparen, _, _) = self.expect(Token::LParen);
                let inner = self.parse_expression();
                let (rparen, _, _) = self.expect(Token::RParen);
                Expr::Paren(ParenExpr {
                    inner: Box::new(inner),
                    lparen_pos: lparen,
                    rparen_pos: rparen,
                })
            }
            Token::LBrack => {
                let (lbrack, _, _) = self.expect(Token::LBrack);
                let elements = if self.tok != Token::RBrack {
                    self.parse_expression_list(Token::RBrack)
                } else {
                    Vec::new()
                };
                let (rbrack, _, _) = self.expect(Token::RBrack);
                Expr::Array(ArrayExpr { elements, lbrack_pos: lbrack, rbrack_pos: rbrack })
            }
            Token::LCurly => {
                let (lcurly, _, _) = self.expect(Token::LCurly);
                let fields = if self.tok != Token::RCurly {
                    self.parse_field_list(Token::RCurly)
                } else {
                    Vec::new()
                };
                let (rcurly, _, _) = self.expect(Token::RCurly);
                Expr::Object(ObjectExpr { fields, lcurly_pos: lcurly, rcurly_pos: rcurly })
            }
            _ => {
                self.add_error(format!("expected expression, got {}", self.tok));
                let res = Expr::Literal(LiteralExpr {
                    kind: Token::Null,
                    value: "null".into(),
                    value_pos: self.pos,
                });
                self.advance_any(STATEMENT_END);
                res
            }
        }
    }

    /// `ExpressionList = Expression { "," Expression } [ "," ]`. Mirrors
    /// `parseExpressionList`.
    fn parse_expression_list(&mut self, until: Token) -> Vec<Expr> {
        let mut exprs = Vec::new();
        while self.tok != until && self.tok != Token::Eof {
            exprs.push(self.parse_expression());
            if self.tok == until {
                break;
            }
            if self.tok != Token::Comma {
                self.add_error("missing ',' in expression list");
            }
            self.next();
        }
        exprs
    }

    /// `FieldList = Field { "," Field } [ "," ]`. Mirrors `parseFieldList`.
    fn parse_field_list(&mut self, until: Token) -> Vec<ObjectField> {
        let mut fields = Vec::new();
        while self.tok != until && self.tok != Token::Eof {
            fields.push(self.parse_field());
            if self.tok == until {
                break;
            }
            if self.tok != Token::Comma {
                self.add_error("missing ',' in field list");
            }
            self.next();
        }
        fields
    }

    /// `Field = ( string | identifier ) "=" Expression`. Mirrors `parseField`.
    fn parse_field(&mut self) -> ObjectField {
        let mut name = Ident { name: String::new(), name_pos: self.pos };
        let mut quoted = false;
        if self.tok == Token::String || self.tok == Token::Ident {
            name.name = self.lit.clone();
            name.name_pos = self.pos;
            if self.tok == Token::String && self.lit.len() > 2 {
                name.name = self.lit[1..self.lit.len() - 1].to_string();
                quoted = true;
            }
            self.next();
        } else {
            self.add_error(format!(
                "expected field name (string or identifier), got {}",
                self.tok
            ));
            self.advance(Token::Assign);
        }
        self.expect(Token::Assign);
        let value = self.parse_expression();
        ObjectField { name, quoted, value }
    }
}

const STATEMENT_END: &[Token] = &[
    Token::Terminator,
    Token::RParen,
    Token::RCurly,
    Token::RBrack,
    Token::Comma,
];

fn is_unary_op(tok: Token) -> bool {
    matches!(tok, Token::Not | Token::Sub)
}

struct BlockName {
    fragments: Vec<String>,
    label: String,
    start: Pos,
    label_pos: Option<Pos>,
}

impl BlockName {
    fn valid_attribute(&self) -> bool {
        self.fragments.len() == 1 && self.label.is_empty()
    }
}
