// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lexical scanner for the Alloy configuration syntax.
//!
//! Line-ported from grafana/alloy `syntax/scanner/scanner.go` (v1.5.0,
//! Apache-2.0). The signature behaviour of the Alloy lexer is *automatic
//! terminator insertion*: a synthetic [`Token::Terminator`] (`"\n"`) is emitted
//! before a newline (or at EOF) whenever the previously scanned token can end a
//! statement, which lets the grammar use newlines as statement separators
//! without requiring them after every token.

use super::token::Token;

const EOF: char = '\0';

/// A position within a scanned source file: byte offset plus 1-based
/// line/column. Mirrors the information carried by Alloy's `token.Pos`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    /// Byte offset from the start of input.
    pub offset: usize,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
}

/// A lexical error encountered while scanning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanError {
    /// Position of the offending character.
    pub pos: Pos,
    /// Human-readable message.
    pub msg: String,
}

/// Lexical scanner over a UTF-8 source string.
pub struct Scanner {
    input: Vec<char>,
    line_starts: Vec<usize>, // byte-offset of the start of each char index's line
    // scanning state
    ch: char,
    offset: usize,     // index of `ch` into `input`
    read_offset: usize, // index of the next char to read
    insert_term: bool,
    include_comments: bool,
    insert_terms: bool,
    errors: Vec<ScanError>,
}

impl Scanner {
    /// Creates a scanner over `src` in the default mode (terminators inserted,
    /// comments skipped).
    pub fn new(src: &str) -> Scanner {
        Scanner::with_mode(src, false, true)
    }

    /// Creates a scanner with explicit modes. `include_comments` returns
    /// comment tokens instead of skipping them; `insert_terms` enables
    /// automatic terminator insertion (the upstream default; disabling it is
    /// for testing only).
    pub fn with_mode(src: &str, include_comments: bool, insert_terms: bool) -> Scanner {
        let input: Vec<char> = src.chars().collect();
        // Precompute line starts by char index for O(log n) position lookup.
        let mut line_starts = vec![0usize];
        for (i, &c) in input.iter().enumerate() {
            if c == '\n' {
                line_starts.push(i + 1);
            }
        }
        let mut s = Scanner {
            input,
            line_starts,
            ch: EOF,
            offset: 0,
            read_offset: 0,
            insert_term: false,
            include_comments,
            insert_terms,
            errors: Vec::new(),
        };
        s.next(); // preload first character
        if s.ch == '\u{FEFF}' {
            s.next(); // ignore a leading BOM
        }
        s
    }

    /// Errors accumulated during scanning.
    pub fn errors(&self) -> &[ScanError] {
        &self.errors
    }

    fn pos(&self, offset: usize) -> Pos {
        // Find the greatest line start <= offset.
        let idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        Pos {
            offset,
            line: idx + 1,
            column: offset - self.line_starts[idx] + 1,
        }
    }

    fn on_error(&mut self, offset: usize, msg: &str) {
        let pos = self.pos(offset);
        self.errors.push(ScanError { pos, msg: msg.to_string() });
    }

    fn peek(&self) -> char {
        if self.read_offset < self.input.len() {
            self.input[self.read_offset]
        } else {
            EOF
        }
    }

    fn next(&mut self) {
        if self.read_offset >= self.input.len() {
            self.offset = self.input.len();
            self.ch = EOF;
            return;
        }
        self.offset = self.read_offset;
        self.ch = self.input[self.read_offset];
        self.read_offset += 1;
    }

    /// Scans the next token and returns its position, the token, and the
    /// literal text (with quotes for strings, keyword text for keywords). The
    /// end of input is indicated by [`Token::Eof`].
    pub fn scan(&mut self) -> (Pos, Token, String) {
        loop {
            self.skip_whitespace();

            let start = self.offset;
            let pos = self.pos(start);
            let mut insert_term = false;
            let tok;
            let mut lit = String::new();

            let ch = self.ch;
            if is_letter(ch) {
                lit = self.scan_identifier();
                if lit.len() > 1 {
                    tok = Token::lookup(&lit);
                    if matches!(tok, Token::Ident | Token::Null | Token::Bool) {
                        insert_term = true;
                    }
                } else {
                    insert_term = true;
                    tok = Token::Ident;
                }
            } else if is_decimal(ch) || (ch == '.' && is_decimal(self.peek())) {
                insert_term = true;
                let (t, l) = self.scan_number();
                tok = t;
                lit = l;
            } else {
                self.next(); // make progress; `ch` is the first char, self.ch the second
                match ch {
                    EOF => {
                        if self.insert_term {
                            self.insert_term = false;
                            return (pos, Token::Terminator, "\n".to_string());
                        }
                        tok = Token::Eof;
                    }
                    '\n' => {
                        // Only reachable when insert_term is true.
                        self.insert_term = false;
                        return (pos, Token::Terminator, "\n".to_string());
                    }
                    '\'' => {
                        self.on_error(start, "illegal single-quoted string; use double quotes");
                        insert_term = true;
                        tok = Token::Illegal;
                        lit = self.scan_string('\'', true, false);
                    }
                    '"' => {
                        insert_term = true;
                        tok = Token::String;
                        lit = self.scan_string('"', true, false);
                    }
                    '`' => {
                        insert_term = true;
                        tok = Token::String;
                        lit = self.scan_string('`', false, true);
                    }
                    '|' => {
                        if self.ch != '|' {
                            self.on_error(self.offset, "missing second | in ||");
                        } else {
                            self.next();
                        }
                        tok = Token::Or;
                    }
                    '&' => {
                        if self.ch != '&' {
                            self.on_error(self.offset, "missing second & in &&");
                        } else {
                            self.next();
                        }
                        tok = Token::And;
                    }
                    '!' => tok = self.switch2(Token::Not, Token::Neq, '='),
                    '=' => tok = self.switch2(Token::Assign, Token::Eq, '='),
                    '<' => tok = self.switch2(Token::Lt, Token::Lte, '='),
                    '>' => tok = self.switch2(Token::Gt, Token::Gte, '='),
                    '+' => tok = Token::Add,
                    '-' => tok = Token::Sub,
                    '*' => tok = Token::Mul,
                    '/' => {
                        if self.ch == '/' || self.ch == '*' {
                            // Comment. If we owe a terminator and the comment runs to
                            // end of line, emit the terminator instead and rewind to
                            // the comment start.
                            if self.insert_term && self.find_line_end() {
                                self.ch = '/';
                                self.offset = start;
                                self.read_offset = start + 1;
                                self.insert_term = false;
                                return (pos, Token::Terminator, "\n".to_string());
                            }
                            let comment = self.scan_comment();
                            if !self.include_comments {
                                self.insert_term = false;
                                continue; // goto scanAgain
                            }
                            tok = Token::Comment;
                            lit = comment;
                        } else {
                            tok = Token::Div;
                        }
                    }
                    '%' => tok = Token::Mod,
                    '^' => tok = Token::Pow,
                    '{' => tok = Token::LCurly,
                    '}' => {
                        insert_term = true;
                        tok = Token::RCurly;
                    }
                    '(' => tok = Token::LParen,
                    ')' => {
                        insert_term = true;
                        tok = Token::RParen;
                    }
                    '[' => tok = Token::LBrack,
                    ']' => {
                        insert_term = true;
                        tok = Token::RBrack;
                    }
                    ',' => tok = Token::Comma,
                    '.' => tok = Token::Dot,
                    _ => {
                        if ch != '\u{FEFF}' {
                            self.on_error(start, &format!("illegal character {:?}", ch));
                        }
                        insert_term = self.insert_term;
                        tok = Token::Illegal;
                        lit = ch.to_string();
                    }
                }
            }

            if self.insert_terms {
                self.insert_term = insert_term;
            }
            return (pos, tok, lit);
        }
    }

    fn skip_whitespace(&mut self) {
        while self.ch == ' '
            || self.ch == '\t'
            || self.ch == '\r'
            || (self.ch == '\n' && !self.insert_term)
        {
            self.next();
        }
    }

    fn switch2(&mut self, a: Token, b: Token, next: char) -> Token {
        if self.ch == next {
            self.next();
            b
        } else {
            a
        }
    }

    fn scan_identifier(&mut self) -> String {
        let off = self.offset;
        while is_letter(self.ch) || is_digit(self.ch) {
            self.next();
        }
        self.input[off..self.offset].iter().collect()
    }

    fn digits(&mut self) -> usize {
        let mut count = 0;
        while is_decimal(self.ch) {
            self.next();
            count += 1;
        }
        count
    }

    fn scan_number(&mut self) -> (Token, String) {
        let mut tok = Token::Number;
        let off = self.offset;

        if self.ch != '.' {
            self.digits();
        }
        if self.ch == '.' {
            tok = Token::Float;
            self.next();
            self.digits();
        }
        if self.ch == 'e' || self.ch == 'E' {
            tok = Token::Float;
            self.next();
            if self.ch == '+' || self.ch == '-' {
                self.next();
            }
            if self.digits() == 0 {
                self.on_error(off, "exponent has no digits");
            }
        }
        (tok, self.input[off..self.offset].iter().collect())
    }

    fn scan_string(&mut self, until: char, escape: bool, multiline: bool) -> String {
        let off = self.offset - 1; // include the opening delimiter
        loop {
            let ch = self.ch;
            if (!multiline && ch == '\n') || ch == EOF {
                self.on_error(off, "string literal not terminated");
                break;
            }
            self.next();
            if ch == until {
                break;
            }
            if escape && ch == '\\' {
                self.scan_escape();
            }
        }
        self.input[off..self.offset].iter().collect()
    }

    fn scan_escape(&mut self) {
        // Consume a single escaped character (the byte after '\\'). The full
        // numeric-escape validation is unnecessary for tokenisation; the
        // parser/value layer revalidates. We consume one char so that an
        // escaped delimiter does not terminate the string.
        if self.ch != EOF {
            self.next();
        } else {
            self.on_error(self.offset, "escape sequence not terminated");
        }
    }

    fn scan_comment(&mut self) -> String {
        let off = self.offset - 1; // the initial '/' was already consumed
        if self.ch == '/' {
            self.next(); // consume second '/'
            while self.ch != '\n' && self.ch != EOF {
                self.next();
            }
        } else {
            // block comment: self.ch == '*'
            self.next(); // consume '*'
            loop {
                if self.ch == EOF {
                    self.on_error(off, "comment not terminated");
                    break;
                }
                let ch = self.ch;
                self.next();
                if ch == '*' && self.ch == '/' {
                    self.next();
                    break;
                }
            }
        }
        self.input[off..self.offset].iter().collect()
    }

    fn find_line_end(&mut self) -> bool {
        // The initial '/' was already consumed. Look ahead for a newline, then
        // restore scanner state to where it was when called.
        let saved_off = self.offset - 1;
        let saved_read = self.read_offset;
        let saved_ch = self.ch;

        let mut result = false;
        // self.ch is the second char of the comment sequence.
        loop {
            if self.ch == '/' {
                // line comments always contain newlines
                result = true;
                break;
            }
            if self.ch != '*' {
                break;
            }
            self.next();
            let mut found = false;
            while self.ch != EOF {
                let ch = self.ch;
                if ch == '\n' {
                    result = true;
                    found = true;
                    break;
                }
                self.next();
                if ch == '*' && self.ch == '/' {
                    self.next();
                    found = true;
                    break;
                }
            }
            if result {
                break;
            }
            if !found {
                break;
            }
            // skip whitespace looking for another comment
            while self.ch == ' ' || self.ch == '\t' || self.ch == '\r' {
                self.next();
            }
            if self.ch != '/' {
                break;
            }
            self.next();
        }

        // restore
        self.offset = saved_off;
        self.read_offset = saved_read;
        self.ch = saved_ch;
        result
    }
}

fn is_decimal(ch: char) -> bool {
    ch.is_ascii_digit()
}

fn is_digit(ch: char) -> bool {
    is_decimal(ch) || (!ch.is_ascii() && ch.is_numeric())
}

fn is_letter(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || (!ch.is_ascii() && ch.is_alphabetic())
}

/// Returns true if `s` scans as a single, complete identifier. Mirrors
/// `scanner.IsValidIdentifier`.
pub fn is_valid_identifier(s: &str) -> bool {
    let mut sc = Scanner::with_mode(s, false, false);
    let (_p, tok, lit) = sc.scan();
    tok == Token::Ident && lit == s
}
