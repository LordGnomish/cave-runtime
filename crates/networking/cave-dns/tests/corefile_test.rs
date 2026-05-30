// SPDX-License-Identifier: AGPL-3.0-or-later
//! Corefile / Caddyfile lexer tests — closes the `caddyfile-corefile-parser`
//! partial. Behaviour mirrors coredns `core/caddyfile/lexer.go` (Caddy v1):
//! whitespace-delimited tokens with line tracking, `#` line comments,
//! double-quoted strings with `\` escapes, and backtick literal strings.
use cave_dns::corefile::{tokenize, Token};

fn texts(input: &str) -> Vec<String> {
    tokenize(input).into_iter().map(|t| t.text).collect()
}

#[test]
fn splits_on_whitespace() {
    assert_eq!(texts("a b\tc\n d"), vec!["a", "b", "c", "d"]);
}

#[test]
fn empty_and_whitespace_only_yield_no_tokens() {
    assert!(tokenize("").is_empty());
    assert!(tokenize("   \n\t  \n").is_empty());
}

#[test]
fn tracks_line_numbers() {
    let toks = tokenize(".:53 {\n    whoami\n}\n");
    // line numbers are 1-based
    assert_eq!(toks[0], Token { text: ".:53".into(), line: 1 });
    assert_eq!(toks[1], Token { text: "{".into(), line: 1 });
    assert_eq!(toks[2], Token { text: "whoami".into(), line: 2 });
    assert_eq!(toks[3], Token { text: "}".into(), line: 3 });
}

#[test]
fn braces_separate_only_on_whitespace() {
    // whitespace-separated -> own tokens
    assert_eq!(texts("forward . 8.8.8.8 {"), vec!["forward", ".", "8.8.8.8", "{"]);
    // glued to a token -> part of the token (lexer does not special-case braces)
    assert_eq!(texts("foo{bar"), vec!["foo{bar"]);
}

#[test]
fn hash_starts_line_comment_at_token_boundary() {
    assert_eq!(texts("cache 30 # a trailing comment\nerrors"), vec!["cache", "30", "errors"]);
    assert_eq!(texts("# whole line\nready"), vec!["ready"]);
}

#[test]
fn hash_inside_token_is_literal() {
    // a '#' that is not at the start of a token is an ordinary rune
    assert_eq!(texts("a#b c"), vec!["a#b", "c"]);
}

#[test]
fn double_quoted_string_preserves_spaces() {
    assert_eq!(texts(r#"log "{remote} {name}""#), vec!["log", "{remote} {name}"]);
}

#[test]
fn quoted_escapes() {
    // \" -> literal quote ; \\ -> literal backslash
    assert_eq!(texts(r#""a\"b""#), vec![r#"a"b"#]);
    assert_eq!(texts(r#""a\\b""#), vec![r"a\b"]);
}

#[test]
fn quoted_string_can_span_newlines() {
    let toks = tokenize("\"line one\nline two\" next");
    assert_eq!(toks[0].text, "line one\nline two");
    assert_eq!(toks[1].text, "next");
}

#[test]
fn backtick_string_is_literal() {
    // backtick string keeps quotes/backslashes verbatim, no escape processing
    assert_eq!(texts("`a \"b\" \\c`"), vec![r#"a "b" \c"#]);
}

#[test]
fn comment_does_not_start_inside_quotes() {
    assert_eq!(texts(r#""a # b""#), vec!["a # b"]);
}

#[test]
fn realistic_corefile_block() {
    let cf = r#"
.:53 {
    forward . 1.1.1.1
    cache 30
    log
    errors
}
"#;
    assert_eq!(
        texts(cf),
        vec![".:53", "{", "forward", ".", "1.1.1.1", "cache", "30", "log", "errors", "}"]
    );
}
