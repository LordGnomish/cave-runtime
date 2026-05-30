// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of grafana/alloy `syntax/parser/internal.go` (v1.5.0).
//!
//! Exercises the recursive-descent parser: attributes vs blocks, block names
//! with `.` fragments + labels, operator precedence (incl. right-associative
//! `^`), unary ops, access/index/call chains, array + object literals, paren
//! grouping, multi-statement bodies, and error diagnostics.

use cave_tracing::alloy::ast::{Expr, Stmt};
use cave_tracing::alloy::parser::{parse_expression, parse_file};
use cave_tracing::alloy::token::Token;

fn parse_expr_ok(src: &str) -> Expr {
    let (e, diags) = parse_expression(src);
    assert!(diags.is_empty(), "unexpected diagnostics for {src:?}: {diags:?}");
    e
}

#[test]
fn parses_single_attribute() {
    let (file, diags) = parse_file("test", "x = 5\n");
    assert!(diags.is_empty(), "diags: {diags:?}");
    assert_eq!(file.body.len(), 1);
    match &file.body[0] {
        Stmt::Attribute(a) => {
            assert_eq!(a.name.name, "x");
            match &a.value {
                Expr::Literal(l) => {
                    assert_eq!(l.kind, Token::Number);
                    assert_eq!(l.value, "5");
                }
                other => panic!("expected literal, got {other:?}"),
            }
        }
        other => panic!("expected attribute, got {other:?}"),
    }
}

#[test]
fn parses_block_with_name_fragments_and_label() {
    let src = "prometheus.scrape \"default\" {\n  targets = []\n}\n";
    let (file, diags) = parse_file("test", src);
    assert!(diags.is_empty(), "diags: {diags:?}");
    assert_eq!(file.body.len(), 1);
    match &file.body[0] {
        Stmt::Block(b) => {
            assert_eq!(b.name, vec!["prometheus".to_string(), "scrape".to_string()]);
            assert_eq!(b.block_name(), "prometheus.scrape");
            assert_eq!(b.label, "default");
            assert_eq!(b.body.len(), 1);
            match &b.body[0] {
                Stmt::Attribute(a) => assert_eq!(a.name.name, "targets"),
                other => panic!("expected attribute in block, got {other:?}"),
            }
        }
        other => panic!("expected block, got {other:?}"),
    }
}

#[test]
fn operator_precedence_mul_binds_tighter_than_add() {
    // 1 + 2 * 3  ==>  Add(1, Mul(2, 3))
    match parse_expr_ok("1 + 2 * 3") {
        Expr::Binary(add) => {
            assert_eq!(add.kind, Token::Add);
            assert!(matches!(*add.left, Expr::Literal(ref l) if l.value == "1"));
            match *add.right {
                Expr::Binary(mul) => {
                    assert_eq!(mul.kind, Token::Mul);
                    assert!(matches!(*mul.left, Expr::Literal(ref l) if l.value == "2"));
                    assert!(matches!(*mul.right, Expr::Literal(ref l) if l.value == "3"));
                }
                other => panic!("expected mul on rhs, got {other:?}"),
            }
        }
        other => panic!("expected binary add, got {other:?}"),
    }
}

#[test]
fn pow_is_right_associative() {
    // 2 ^ 3 ^ 2  ==>  Pow(2, Pow(3, 2))
    match parse_expr_ok("2 ^ 3 ^ 2") {
        Expr::Binary(outer) => {
            assert_eq!(outer.kind, Token::Pow);
            assert!(matches!(*outer.left, Expr::Literal(ref l) if l.value == "2"));
            assert!(matches!(*outer.right, Expr::Binary(ref inner) if inner.kind == Token::Pow));
        }
        other => panic!("expected pow, got {other:?}"),
    }
}

#[test]
fn unary_operators() {
    assert!(matches!(parse_expr_ok("-5"), Expr::Unary(u) if u.kind == Token::Sub));
    assert!(matches!(parse_expr_ok("!true"), Expr::Unary(u) if u.kind == Token::Not));
}

#[test]
fn access_index_and_call_chains() {
    // a.b.c  ==>  Access(Access(a, b), c)
    match parse_expr_ok("a.b.c") {
        Expr::Access(outer) => {
            assert_eq!(outer.name.name, "c");
            assert!(matches!(*outer.value, Expr::Access(ref inner) if inner.name.name == "b"));
        }
        other => panic!("expected access, got {other:?}"),
    }
    // arr[0]
    assert!(matches!(parse_expr_ok("arr[0]"), Expr::Index(_)));
    // f(1, 2)
    match parse_expr_ok("f(1, 2)") {
        Expr::Call(c) => assert_eq!(c.args.len(), 2),
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn array_and_object_literals() {
    match parse_expr_ok("[1, 2, 3]") {
        Expr::Array(a) => assert_eq!(a.elements.len(), 3),
        other => panic!("expected array, got {other:?}"),
    }
    // object with a bare ident key and a quoted key
    match parse_expr_ok("{ a = 1, \"b c\" = 2 }") {
        Expr::Object(o) => {
            assert_eq!(o.fields.len(), 2);
            assert_eq!(o.fields[0].name.name, "a");
            assert!(!o.fields[0].quoted);
            assert_eq!(o.fields[1].name.name, "b c");
            assert!(o.fields[1].quoted);
        }
        other => panic!("expected object, got {other:?}"),
    }
}

#[test]
fn paren_grouping_overrides_precedence() {
    // (1 + 2) * 3  ==>  Mul(Paren(Add(1,2)), 3)
    match parse_expr_ok("(1 + 2) * 3") {
        Expr::Binary(mul) => {
            assert_eq!(mul.kind, Token::Mul);
            assert!(matches!(*mul.left, Expr::Paren(_)));
        }
        other => panic!("expected mul, got {other:?}"),
    }
}

#[test]
fn multiple_statements_separated_by_terminators() {
    let (file, diags) = parse_file("test", "a = 1\nb = 2\nc = 3\n");
    assert!(diags.is_empty(), "diags: {diags:?}");
    assert_eq!(file.body.len(), 3);
}

#[test]
fn missing_expression_produces_diagnostic() {
    // attribute with no right-hand side
    let (_file, diags) = parse_file("test", "x =\n");
    assert!(!diags.is_empty(), "expected at least one diagnostic");
}
