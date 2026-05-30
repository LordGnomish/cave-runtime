// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of grafana/alloy `syntax/scanner/scanner.go` (v1.5.0).
//!
//! Exercises automatic terminator insertion, identifiers/keywords, numbers vs
//! floats, double-quoted + backtick strings with escapes, operators via the
//! two-character switch, and comment handling.

use cave_tracing::alloy::scanner::Scanner;
use cave_tracing::alloy::token::Token;

/// Lex `src` to completion, returning every `(Token, literal)` pair including
/// the trailing `EOF`.
fn lex(src: &str) -> Vec<(Token, String)> {
    let mut s = Scanner::new(src);
    let mut out = Vec::new();
    loop {
        let (_pos, tok, lit) = s.scan();
        out.push((tok, lit));
        if tok == Token::Eof {
            break;
        }
    }
    out
}

fn toks(src: &str) -> Vec<Token> {
    lex(src).into_iter().map(|(t, _)| t).collect()
}

#[test]
fn attribute_inserts_terminator_before_newline() {
    assert_eq!(
        toks("foo = 5\n"),
        vec![Token::Ident, Token::Assign, Token::Number, Token::Terminator, Token::Eof],
    );
}

#[test]
fn terminator_synthesized_at_eof_without_trailing_newline() {
    // No trailing '\n', but the NUMBER set insertTerm, so EOF emits a
    // synthetic TERMINATOR first, then EOF.
    assert_eq!(
        toks("foo = 5"),
        vec![Token::Ident, Token::Assign, Token::Number, Token::Terminator, Token::Eof],
    );
}

#[test]
fn keywords_and_idents() {
    assert_eq!(lex("true")[0], (Token::Bool, "true".to_string()));
    assert_eq!(lex("false")[0], (Token::Bool, "false".to_string()));
    assert_eq!(lex("null")[0], (Token::Null, "null".to_string()));
    assert_eq!(lex("my_target1")[0], (Token::Ident, "my_target1".to_string()));
}

#[test]
fn numbers_floats_and_exponents() {
    assert_eq!(lex("1234")[0], (Token::Number, "1234".to_string()));
    assert_eq!(lex("12.5")[0], (Token::Float, "12.5".to_string()));
    assert_eq!(lex(".5")[0], (Token::Float, ".5".to_string()));
    assert_eq!(lex("1e10")[0], (Token::Float, "1e10".to_string()));
    assert_eq!(lex("1.5e-3")[0], (Token::Float, "1.5e-3".to_string()));
}

#[test]
fn double_quoted_string_keeps_quotes_and_escapes() {
    assert_eq!(lex(r#""hello""#)[0], (Token::String, r#""hello""#.to_string()));
    // Escaped quote does not terminate the string.
    assert_eq!(
        lex(r#""a\"b""#)[0],
        (Token::String, r#""a\"b""#.to_string()),
    );
}

#[test]
fn backtick_raw_string_allows_newlines() {
    let src = "`line1\nline2`";
    let first = lex(src);
    assert_eq!(first[0].0, Token::String);
    assert_eq!(first[0].1, src);
}

#[test]
fn two_char_operators_via_switch() {
    assert_eq!(toks("=="), vec![Token::Eq, Token::Eof]);
    assert_eq!(toks("="), vec![Token::Assign, Token::Eof]);
    assert_eq!(toks("!="), vec![Token::Neq, Token::Eof]);
    assert_eq!(toks("!"), vec![Token::Not, Token::Eof]);
    assert_eq!(toks("<="), vec![Token::Lte, Token::Eof]);
    assert_eq!(toks(">"), vec![Token::Gt, Token::Eof]);
    assert_eq!(toks("||"), vec![Token::Or, Token::Eof]);
    assert_eq!(toks("&&"), vec![Token::And, Token::Eof]);
}

#[test]
fn delimiters_set_terminator_after_closers() {
    // RCURLY / RPAREN / RBRACK set insertTerm; the closing ] before EOF emits
    // a TERMINATOR.
    assert_eq!(
        toks("[]"),
        vec![Token::LBrack, Token::RBrack, Token::Terminator, Token::Eof],
    );
}

#[test]
fn line_comment_is_skipped_but_keeps_terminator() {
    // "x = 1 // note\n y = 2" : the comment replaces the rest of the line;
    // because insertTerm was set by NUMBER 1, a TERMINATOR is emitted at the
    // comment, and the comment text itself is skipped.
    let t = toks("x = 1 // note\ny = 2");
    assert_eq!(
        t,
        vec![
            Token::Ident, Token::Assign, Token::Number, Token::Terminator,
            Token::Ident, Token::Assign, Token::Number, Token::Terminator,
            Token::Eof,
        ],
    );
}

#[test]
fn realistic_alloy_block() {
    // LCurly does NOT set insertTerm (only RCURLY/RPAREN/RBRACK do), so the
    // newline after `{` is plain whitespace and no terminator follows it.
    let t = toks("prometheus.scrape \"default\" {\n  targets = []\n}\n");
    assert_eq!(
        t,
        vec![
            Token::Ident, Token::Dot, Token::Ident, Token::String, Token::LCurly,
            Token::Ident, Token::Assign, Token::LBrack, Token::RBrack, Token::Terminator,
            Token::RCurly, Token::Terminator,
            Token::Eof,
        ],
    );
}

#[test]
fn position_tracks_line_and_column() {
    let mut s = Scanner::new("a = 1\nbb = 2");
    let (p0, t0, _) = s.scan(); // 'a'
    assert_eq!(t0, Token::Ident);
    assert_eq!((p0.line, p0.column), (1, 1));
    // advance to the second line's identifier
    let mut last = (p0, t0);
    loop {
        let (p, t, _) = s.scan();
        if t == Token::Ident {
            last = (p, t);
            break;
        }
        if t == Token::Eof {
            break;
        }
    }
    assert_eq!(last.1, Token::Ident);
    assert_eq!((last.0.line, last.0.column), (2, 1));
}
