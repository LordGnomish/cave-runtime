// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of grafana/alloy `syntax/ast/ast.go` + `walk.go` (v1.5.0).
//!
//! Exercises AST node construction, `start_pos`/`end_pos`, `BlockStmt`'s
//! block-name join, and depth-first `walk` traversal.

use cave_tracing::alloy::ast::{
    walk, ArrayExpr, AttributeStmt, BinaryExpr, BlockStmt, Expr, Ident, LiteralExpr, Node, Stmt,
};
use cave_tracing::alloy::scanner::Pos;
use cave_tracing::alloy::token::Token;

fn pos(offset: usize) -> Pos {
    Pos { offset, line: 1, column: offset + 1 }
}

#[test]
fn literal_start_and_end_pos() {
    // "1234" at offset 0 spans offsets 0..=3.
    let lit = Expr::Literal(LiteralExpr {
        kind: Token::Number,
        value: "1234".to_string(),
        value_pos: pos(0),
    });
    assert_eq!(lit.start_pos().offset, 0);
    assert_eq!(lit.end_pos().offset, 3);
}

#[test]
fn binary_expr_spans_left_to_right() {
    // "1 + 22" : left literal at 0, right literal "22" at 4..=5.
    let left = Expr::Literal(LiteralExpr { kind: Token::Number, value: "1".into(), value_pos: pos(0) });
    let right = Expr::Literal(LiteralExpr { kind: Token::Number, value: "22".into(), value_pos: pos(4) });
    let bin = Expr::Binary(BinaryExpr {
        left: Box::new(left),
        kind: Token::Add,
        kind_pos: pos(2),
        right: Box::new(right),
    });
    assert_eq!(bin.start_pos().offset, 0); // start of left
    assert_eq!(bin.end_pos().offset, 5); // end of right ("22" ends at 5)
}

#[test]
fn attribute_stmt_pos_and_block_name() {
    let attr = Stmt::Attribute(AttributeStmt {
        name: Ident { name: "targets".into(), name_pos: pos(0) },
        value: Expr::Array(ArrayExpr { elements: vec![], lbrack_pos: pos(10), rbrack_pos: pos(11) }),
    });
    assert_eq!(attr.start_pos().offset, 0); // start of name
    assert_eq!(attr.end_pos().offset, 11); // rbrack of empty array value

    let block = BlockStmt {
        name: vec!["prometheus".into(), "scrape".into()],
        name_pos: pos(0),
        label: "default".into(),
        label_pos: Some(pos(18)),
        body: vec![],
        lcurly_pos: pos(28),
        rcurly_pos: pos(40),
    };
    assert_eq!(block.block_name(), "prometheus.scrape");
    assert_eq!(Stmt::Block(block).end_pos().offset, 40); // rcurly
}

#[test]
fn walk_visits_all_nodes_depth_first() {
    // (1 + 2) as a binary expr → walk hits: binary, left literal, right literal.
    let bin = Expr::Binary(BinaryExpr {
        left: Box::new(Expr::Literal(LiteralExpr { kind: Token::Number, value: "1".into(), value_pos: pos(0) })),
        kind: Token::Add,
        kind_pos: pos(2),
        right: Box::new(Expr::Literal(LiteralExpr { kind: Token::Number, value: "2".into(), value_pos: pos(4) })),
    });

    let mut literals = 0usize;
    let mut total = 0usize;
    walk(&Node::Expr(&bin), &mut |n: &Node| {
        total += 1;
        if let Node::Expr(Expr::Literal(_)) = n {
            literals += 1;
        }
    });
    assert_eq!(literals, 2);
    assert_eq!(total, 3); // binary + 2 literals
}
