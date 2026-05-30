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

use std::collections::BTreeMap;

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

// ─── Parser (core/caddyfile/parse.go) ───────────────────────────────────────

/// A parsed server block: the address keys preceding the opening brace plus
/// the directive → token-slice map captured inside the block.
///
/// Faithful to upstream `caddyfile.ServerBlock`: each directive's slice begins
/// with the directive name token itself, followed by every argument token on
/// the directive's line (including the tokens of any nested `{ … }` block).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ServerBlock {
    /// Address labels before the block (e.g. `example.com:53`).
    pub keys: Vec<String>,
    /// Directive name → captured tokens (directive token first).
    pub tokens: BTreeMap<String, Vec<Token>>,
}

/// A Corefile parse failure with the 1-based source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based source line where parsing failed.
    pub line: usize,
    /// Human-readable description.
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Corefile parse error (line {}): {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse Corefile/Caddyfile `input` into the list of [`ServerBlock`]s.
///
/// Port of `core/caddyfile/parse.go::parseAll` over the token stream produced
/// by [`tokenize`]. Server blocks with no address keys are dropped (matching
/// upstream, which only emits blocks where `len(Keys) > 0`).
pub fn parse(input: &str) -> Result<Vec<ServerBlock>, ParseError> {
    let mut parser = Parser::new(tokenize(input));
    parser.parse_all()
}

/// Cursor-based recursive parser mirroring caddy's `Dispenser` + `parser`.
struct Parser {
    tokens: Vec<Token>,
    cursor: isize,
    block: ServerBlock,
    eof: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, cursor: -1, block: ServerBlock::default(), eof: false }
    }

    // ── Dispenser primitives ────────────────────────────────────────────

    /// Advance to the next token; `false` at end of stream.
    fn next(&mut self) -> bool {
        if self.cursor < self.tokens.len() as isize - 1 {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn val(&self) -> &str {
        self.tokens.get(self.cursor as usize).map_or("", |t| t.text.as_str())
    }

    fn cur(&self) -> Token {
        self.tokens[self.cursor as usize].clone()
    }

    fn line(&self) -> usize {
        self.tokens.get(self.cursor as usize).map_or(0, |t| t.line)
    }

    /// True when the current token starts a new source line relative to the
    /// previous token (upstream `Dispenser.isNewLine`).
    fn is_newline(&self) -> bool {
        if self.cursor < 1 {
            return true;
        }
        if self.cursor as usize > self.tokens.len() - 1 {
            return false;
        }
        self.tokens[self.cursor as usize - 1].line < self.tokens[self.cursor as usize].line
    }

    // ── Grammar ──────────────────────────────────────────────────────────

    fn parse_all(&mut self) -> Result<Vec<ServerBlock>, ParseError> {
        let mut blocks = Vec::new();
        while self.next() {
            self.parse_one()?;
            if !self.block.keys.is_empty() {
                blocks.push(std::mem::take(&mut self.block));
            }
        }
        Ok(blocks)
    }

    fn parse_one(&mut self) -> Result<(), ParseError> {
        self.block = ServerBlock::default();
        self.begin()
    }

    fn begin(&mut self) -> Result<(), ParseError> {
        if self.tokens.is_empty() {
            return Ok(());
        }
        self.addresses()?;
        if self.eof {
            return Ok(());
        }
        self.block_contents()
    }

    /// Read the address labels that precede a block. A trailing comma on a
    /// token means another address follows (possibly on the next line).
    fn addresses(&mut self) -> Result<(), ParseError> {
        let mut expecting_another = false;
        loop {
            let mut tkn = self.val().to_string();

            // An open brace ends the address list.
            if tkn == "{" {
                if expecting_another {
                    return Err(ParseError {
                        line: self.line(),
                        message: "expected another address but found '{' — check for an extra comma".into(),
                    });
                }
                break;
            }

            if !tkn.is_empty() {
                if tkn.ends_with(',') {
                    tkn.pop();
                    expecting_another = true;
                } else {
                    expecting_another = false;
                }
                if !tkn.is_empty() {
                    self.block.keys.push(tkn);
                }
            }

            let has_next = self.next();
            if expecting_another && !has_next {
                return Err(ParseError {
                    line: self.line(),
                    message: "unexpected EOF while expecting another address".into(),
                });
            }
            if !has_next {
                self.eof = true;
                break;
            }
            if !expecting_another && self.is_newline() {
                break;
            }
        }
        Ok(())
    }

    fn block_contents(&mut self) -> Result<(), ParseError> {
        // A single-server config may have no braces at all.
        if self.val() != "{" {
            self.cursor -= 1;
            return Ok(());
        }
        self.directives()?;
        // Consume the closing brace.
        if !self.next() || self.val() != "}" {
            return Err(ParseError {
                line: self.line(),
                message: "expected '}' to close server block".into(),
            });
        }
        Ok(())
    }

    /// Iterate the directives inside a `{ … }` block.
    fn directives(&mut self) -> Result<(), ParseError> {
        while self.next() {
            if self.val() == "}" {
                // Unget so block_contents can consume the closing brace.
                self.cursor -= 1;
                break;
            }
            self.directive()?;
        }
        Ok(())
    }

    /// Capture one directive and all of its argument tokens, honouring nested
    /// `{ … }` braces, until the line ends at brace-nesting depth zero.
    fn directive(&mut self) -> Result<(), ParseError> {
        let dir = self.val().to_string();
        let mut nesting = 0i32;
        let tok = self.cur();
        self.block.tokens.entry(dir.clone()).or_default().push(tok);

        while self.next() {
            let v = self.val().to_string();
            if v == "{" {
                nesting += 1;
            } else if self.is_newline() && nesting == 0 {
                // Read one token too far — give it back to directives().
                self.cursor -= 1;
                break;
            } else if v == "}" && nesting > 0 {
                nesting -= 1;
            } else if v == "}" && nesting == 0 {
                return Err(ParseError {
                    line: self.line(),
                    message: "unexpected '}'".into(),
                });
            }
            let tok = self.cur();
            self.block.tokens.entry(dir.clone()).or_default().push(tok);
        }
        Ok(())
    }
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

    // ── Cycle 2: multiple addresses + duplicate directives ─────────────────

    #[test]
    fn parse_comma_separated_addresses_same_line() {
        let src = "example.com:53, example.org:53 {\n    whoami\n}\n";
        let blocks = parse(src).expect("parse ok");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].keys,
            vec!["example.com:53".to_string(), "example.org:53".to_string()]
        );
    }

    #[test]
    fn parse_trailing_comma_continues_addresses_on_next_line() {
        // A trailing comma means the address list continues on the next line.
        let src = "example.com:53,\nexample.org:53 {\n    whoami\n}\n";
        let blocks = parse(src).expect("parse ok");
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].keys,
            vec!["example.com:53".to_string(), "example.org:53".to_string()]
        );
    }

    #[test]
    fn parse_dangling_comma_at_eof_is_error() {
        // Trailing comma with nothing following is a parse error (caddy EOFErr).
        let err = parse("example.com:53,").unwrap_err();
        assert!(err.message.contains("EOF") || err.message.contains("another address"));
    }

    #[test]
    fn parse_duplicate_directive_accumulates_tokens() {
        // Two `forward` lines merge into one directive token slice (upstream
        // ServerBlock.Tokens is a map keyed by directive name).
        let src = "example.com {\n    forward . 1.1.1.1\n    forward . 8.8.8.8\n}\n";
        let blocks = parse(src).expect("parse ok");
        assert_eq!(
            texts(&blocks[0].tokens["forward"]),
            vec!["forward", ".", "1.1.1.1", "forward", ".", "8.8.8.8"]
        );
    }

    #[test]
    fn parse_multiple_server_blocks() {
        let src = ".:53 {\n    whoami\n}\n\nexample.com:1053 {\n    chaos\n}\n";
        let blocks = parse(src).expect("parse ok");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].keys, vec![".:53".to_string()]);
        assert_eq!(blocks[1].keys, vec!["example.com:1053".to_string()]);
    }
}
