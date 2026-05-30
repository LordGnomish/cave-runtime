// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of grafana/alloy `syntax/token/token.go` (v1.5.0).
//!
//! Exercises the lexical token set, keyword lookup, operator precedence, and
//! the token-class predicates that the scanner and parser depend on.

use cave_tracing::alloy::token::{
    Token, HIGHEST_PRECEDENCE, LOWEST_PRECEDENCE, UNARY_PRECEDENCE,
};

#[test]
fn lookup_resolves_keywords_and_idents() {
    // BOOL keyword for true/false, NULL for null, otherwise IDENT.
    assert_eq!(Token::lookup("true"), Token::Bool);
    assert_eq!(Token::lookup("false"), Token::Bool);
    assert_eq!(Token::lookup("null"), Token::Null);
    assert_eq!(Token::lookup("foobar"), Token::Ident);
    assert_eq!(Token::lookup("True"), Token::Ident); // case-sensitive
}

#[test]
fn display_matches_upstream_token_names() {
    // Operators/delimiters render as their literal; categories render as names.
    assert_eq!(Token::Or.to_string(), "||");
    assert_eq!(Token::And.to_string(), "&&");
    assert_eq!(Token::Eq.to_string(), "==");
    assert_eq!(Token::Neq.to_string(), "!=");
    assert_eq!(Token::Lte.to_string(), "<=");
    assert_eq!(Token::Pow.to_string(), "^");
    assert_eq!(Token::LCurly.to_string(), "{");
    assert_eq!(Token::RBrack.to_string(), "]");
    assert_eq!(Token::Dot.to_string(), ".");
    assert_eq!(Token::Ident.to_string(), "IDENT");
    assert_eq!(Token::Number.to_string(), "NUMBER");
    assert_eq!(Token::Terminator.to_string(), "TERMINATOR");
    assert_eq!(Token::Eof.to_string(), "EOF");
}

#[test]
fn binary_precedence_ladder() {
    // From token.go BinaryPrecedence(): || =1, && =2, cmp =3, add/sub =4,
    // mul/div/mod =5, pow =6, non-operators = LowestPrecedence (0).
    assert_eq!(Token::Or.binary_precedence(), 1);
    assert_eq!(Token::And.binary_precedence(), 2);
    assert_eq!(Token::Eq.binary_precedence(), 3);
    assert_eq!(Token::Neq.binary_precedence(), 3);
    assert_eq!(Token::Lt.binary_precedence(), 3);
    assert_eq!(Token::Gte.binary_precedence(), 3);
    assert_eq!(Token::Add.binary_precedence(), 4);
    assert_eq!(Token::Sub.binary_precedence(), 4);
    assert_eq!(Token::Mul.binary_precedence(), 5);
    assert_eq!(Token::Div.binary_precedence(), 5);
    assert_eq!(Token::Mod.binary_precedence(), 5);
    assert_eq!(Token::Pow.binary_precedence(), 6);
    // Non-binary-operators fall back to the lowest precedence.
    assert_eq!(Token::Ident.binary_precedence(), LOWEST_PRECEDENCE);
    assert_eq!(Token::LCurly.binary_precedence(), LOWEST_PRECEDENCE);
    assert_eq!(Token::Not.binary_precedence(), LOWEST_PRECEDENCE);
}

#[test]
fn precedence_constants() {
    assert_eq!(LOWEST_PRECEDENCE, 0);
    assert_eq!(UNARY_PRECEDENCE, 7);
    assert_eq!(HIGHEST_PRECEDENCE, 8);
    // Every binary operator sits strictly below the unary precedence so that
    // unary expressions bind tighter than any binary operator.
    for t in [Token::Or, Token::And, Token::Eq, Token::Add, Token::Mul, Token::Pow] {
        assert!(t.binary_precedence() < UNARY_PRECEDENCE);
    }
}

#[test]
fn token_class_predicates() {
    // is_literal: IDENT/NUMBER/FLOAT/STRING (the literalBeg..literalEnd band).
    assert!(Token::Ident.is_literal());
    assert!(Token::Number.is_literal());
    assert!(Token::Float.is_literal());
    assert!(Token::String.is_literal());
    assert!(!Token::Bool.is_literal());
    assert!(!Token::Or.is_literal());

    // is_keyword: BOOL/NULL only.
    assert!(Token::Bool.is_keyword());
    assert!(Token::Null.is_keyword());
    assert!(!Token::Ident.is_keyword());

    // is_operator: the operatorBeg..operatorEnd band (|| .. DOT).
    assert!(Token::Or.is_operator());
    assert!(Token::Assign.is_operator());
    assert!(Token::Dot.is_operator());
    assert!(Token::LCurly.is_operator());
    assert!(!Token::Terminator.is_operator()); // TERMINATOR sits past operatorEnd
    assert!(!Token::Ident.is_operator());
}

#[test]
fn illegal_token_renders_illegal() {
    assert_eq!(Token::Illegal.to_string(), "ILLEGAL");
}
