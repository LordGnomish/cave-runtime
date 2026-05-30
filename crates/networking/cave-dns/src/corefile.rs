// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Corefile / Caddyfile lexer.
//!
//! Port of `coredns` `core/caddyfile/lexer.go` (the Caddy v1 lexer CoreDNS
//! vendors). Closes the `caddyfile-corefile-parser` partial: cave-dns can now
//! tokenise a native Corefile instead of only accepting the serde JSON/TOML
//! shim.
//!
//! Lexing rules (faithful to upstream `lexer.next`):
//!   * Tokens are delimited by Unicode whitespace; `\n` advances the line.
//!   * `#` begins a comment to end-of-line — but only when it is the first
//!     rune of a token. A `#` inside a token (e.g. `a#b`) is a literal rune.
//!   * `"` opens a double-quoted token: `\` escapes the next rune, the token
//!     closes on an unescaped `"`, and embedded whitespace (incl. newlines)
//!     is preserved.
//!   * `` ` `` opens a backtick token: everything up to the next backtick is
//!     literal (no escape processing).
//!   * Braces are not special to the lexer; they only become their own tokens
//!     when whitespace-separated.

/// A single lexed token plus the 1-based line on which it began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token text (quotes stripped, escapes resolved).
    pub text: String,
    /// 1-based source line where the token started.
    pub line: usize,
}

/// Tokenise Corefile/Caddyfile source into [`Token`]s.
#[must_use]
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut val = String::new();
    let mut started = false; // whether `val` holds an in-progress token
    let mut token_line = 1usize;
    let mut line = 1usize;

    let mut comment = false;
    let mut quoted = false;
    let mut bt_quote = false;
    let mut escaped = false;

    let mut flush = |val: &mut String, started: &mut bool, token_line: usize, tokens: &mut Vec<Token>| {
        tokens.push(Token { text: std::mem::take(val), line: token_line });
        *started = false;
    };

    for ch in input.chars() {
        if quoted {
            if escaped {
                val.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                quoted = false;
                flush(&mut val, &mut started, token_line, &mut tokens);
                continue;
            }
            val.push(ch);
            continue;
        }

        if bt_quote {
            if ch == '`' {
                bt_quote = false;
                flush(&mut val, &mut started, token_line, &mut tokens);
                continue;
            }
            val.push(ch);
            continue;
        }

        if ch.is_whitespace() {
            if ch == '\n' {
                line += 1;
                comment = false;
            }
            if started {
                flush(&mut val, &mut started, token_line, &mut tokens);
            }
            continue;
        }

        if comment {
            continue;
        }

        if !started {
            token_line = line;
            match ch {
                '#' => {
                    comment = true;
                    continue;
                }
                '"' => {
                    quoted = true;
                    started = true;
                    continue;
                }
                '`' => {
                    bt_quote = true;
                    started = true;
                    continue;
                }
                _ => {}
            }
        }

        started = true;
        val.push(ch);
    }

    // Flush any trailing unterminated token (matches upstream EOF handling).
    if started {
        flush(&mut val, &mut started, token_line, &mut tokens);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(toks: &[Token]) -> Vec<&str> {
        toks.iter().map(|t| t.text.as_str()).collect()
    }

    #[test]
    fn trailing_unterminated_token_flushes_at_eof() {
        let toks = tokenize("errors");
        assert_eq!(toks, vec![Token { text: "errors".into(), line: 1 }]);
    }

    // ── Cycle 1: parse.go — server blocks + directives ─────────────────────

    #[test]
    fn parse_single_block_with_directives() {
        let src = "example.com:53 {\n    whoami\n    forward . 1.1.1.1 8.8.8.8\n}\n";
        let blocks = parse(src).expect("parse ok");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].keys, vec!["example.com:53".to_string()]);

        // A bare directive carries only its own name token (caddy stores the
        // directive token as the first element of its token slice).
        assert_eq!(texts(&blocks[0].tokens["whoami"]), vec!["whoami"]);

        // A directive with args carries name + every arg on the line.
        assert_eq!(
            texts(&blocks[0].tokens["forward"]),
            vec!["forward", ".", "1.1.1.1", "8.8.8.8"]
        );
    }

    #[test]
    fn parse_directive_with_nested_block() {
        // The nested `{ ... }` tokens belong to the enclosing directive.
        let src = "example.com {\n    forward . 1.1.1.1 {\n        policy random\n        max_fails 3\n    }\n}\n";
        let blocks = parse(src).expect("parse ok");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            texts(&blocks[0].tokens["forward"]),
            vec!["forward", ".", "1.1.1.1", "{", "policy", "random", "max_fails", "3", "}"]
        );
    }

    #[test]
    fn parse_one_line_server_block_without_braces() {
        // A naked address line (no braces) is a valid single-server config.
        let blocks = parse(".:53\n").expect("parse ok");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].keys, vec![".:53".to_string()]);
        assert!(blocks[0].tokens.is_empty());
    }
}
