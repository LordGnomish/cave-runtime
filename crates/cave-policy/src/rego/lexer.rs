// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego lexer — tokenizes Rego source text.

use crate::error::PolicyError;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Null,
    True,
    False,
    Number(String),
    String(String),
    // Keywords
    Package,
    Import,
    As,
    Default,
    Not,
    Some,
    Every,
    In,
    With,
    Else,
    If,
    Contains,
    // Special identifiers treated as keywords in context
    Data,
    Input,
    Future,
    // Identifier
    Ident(String),
    // Operators
    Assign, // :=
    Unify,  // =
    Eq,     // ==
    Ne,     // !=
    Lt,     // <
    Gt,     // >
    Le,     // <=
    Ge,     // >=
    Plus,   // +
    Minus,  // -
    Mul,    // *
    Div,    // /
    Mod,    // %
    And,    // &
    Or,     // |
    // Punctuation
    LBrace,     // {
    RBrace,     // }
    LBracket,   // [
    RBracket,   // ]
    LParen,     // (
    RParen,     // )
    Comma,      // ,
    Semicolon,  // ;
    Colon,      // :
    Dot,        // .
    Underscore, // _
    Newline,    // \n (significant in rule bodies)
    // Special
    Eof,
}

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub line: usize,
    pub col: usize,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
    col: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<(Token, Span)>, PolicyError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace_no_newline();
            if self.pos >= self.src.len() {
                tokens.push((Token::Eof, self.span()));
                break;
            }
            let span = self.span();
            let ch = self.src[self.pos];

            // Skip line comments
            if ch == b'#' {
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.advance();
                }
                continue;
            }

            // Newlines are significant
            if ch == b'\n' {
                self.advance();
                tokens.push((Token::Newline, span));
                continue;
            }

            let tok = match ch {
                b'{' => {
                    self.advance();
                    Token::LBrace
                }
                b'}' => {
                    self.advance();
                    Token::RBrace
                }
                b'[' => {
                    self.advance();
                    Token::LBracket
                }
                b']' => {
                    self.advance();
                    Token::RBracket
                }
                b'(' => {
                    self.advance();
                    Token::LParen
                }
                b')' => {
                    self.advance();
                    Token::RParen
                }
                b',' => {
                    self.advance();
                    Token::Comma
                }
                b';' => {
                    self.advance();
                    Token::Semicolon
                }
                b'+' => {
                    self.advance();
                    Token::Plus
                }
                b'-' => {
                    self.advance();
                    Token::Minus
                }
                b'*' => {
                    self.advance();
                    Token::Mul
                }
                b'/' => {
                    self.advance();
                    Token::Div
                }
                b'%' => {
                    self.advance();
                    Token::Mod
                }
                b'&' => {
                    self.advance();
                    Token::And
                }
                b'|' => {
                    self.advance();
                    Token::Or
                }
                b'.' => {
                    self.advance();
                    Token::Dot
                }
                b':' => {
                    self.advance();
                    if self.pos < self.src.len() && self.src[self.pos] == b'=' {
                        self.advance();
                        Token::Assign
                    } else {
                        Token::Colon
                    }
                }
                b'=' => {
                    self.advance();
                    if self.pos < self.src.len() && self.src[self.pos] == b'=' {
                        self.advance();
                        Token::Eq
                    } else {
                        Token::Unify
                    }
                }
                b'!' => {
                    self.advance();
                    if self.pos < self.src.len() && self.src[self.pos] == b'=' {
                        self.advance();
                        Token::Ne
                    } else {
                        return Err(PolicyError::Parse(format!(
                            "unexpected '!' at {}:{}",
                            span.line, span.col
                        )));
                    }
                }
                b'<' => {
                    self.advance();
                    if self.pos < self.src.len() && self.src[self.pos] == b'=' {
                        self.advance();
                        Token::Le
                    } else {
                        Token::Lt
                    }
                }
                b'>' => {
                    self.advance();
                    if self.pos < self.src.len() && self.src[self.pos] == b'=' {
                        self.advance();
                        Token::Ge
                    } else {
                        Token::Gt
                    }
                }
                b'"' => self.lex_string()?,
                b'`' => self.lex_raw_string()?,
                b'_' => {
                    // Could be _ (wildcard) or _identifier
                    let start = self.pos;
                    self.advance();
                    if self.pos < self.src.len()
                        && (self.src[self.pos].is_ascii_alphanumeric()
                            || self.src[self.pos] == b'_')
                    {
                        while self.pos < self.src.len()
                            && (self.src[self.pos].is_ascii_alphanumeric()
                                || self.src[self.pos] == b'_')
                        {
                            self.advance();
                        }
                        let ident = std::str::from_utf8(&self.src[start..self.pos])
                            .map_err(|e| PolicyError::Parse(e.to_string()))?
                            .to_string();
                        Token::Ident(ident)
                    } else {
                        Token::Underscore
                    }
                }
                c if c.is_ascii_digit() => self.lex_number()?,
                c if c.is_ascii_alphabetic() => self.lex_ident_or_keyword()?,
                c => {
                    return Err(PolicyError::Parse(format!(
                        "unexpected character '{}' at {}:{}",
                        c as char, span.line, span.col
                    )));
                }
            };
            tokens.push((tok, span));
        }
        Ok(tokens)
    }

    fn advance(&mut self) -> u8 {
        let c = self.src[self.pos];
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn span(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
        }
    }

    fn skip_whitespace_no_newline(&mut self) {
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if c == b' ' || c == b'\t' || c == b'\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn lex_string(&mut self) -> Result<Token, PolicyError> {
        self.advance(); // consume opening "
        let mut s = String::new();
        loop {
            if self.pos >= self.src.len() {
                return Err(PolicyError::Parse("unterminated string".into()));
            }
            let c = self.src[self.pos];
            if c == b'"' {
                self.advance();
                break;
            }
            if c == b'\\' {
                self.advance();
                if self.pos >= self.src.len() {
                    return Err(PolicyError::Parse("unterminated escape".into()));
                }
                let escaped = self.src[self.pos];
                self.advance();
                match escaped {
                    b'n' => s.push('\n'),
                    b't' => s.push('\t'),
                    b'r' => s.push('\r'),
                    b'\\' => s.push('\\'),
                    b'"' => s.push('"'),
                    b'/' => s.push('/'),
                    b'b' => s.push('\x08'),
                    b'f' => s.push('\x0C'),
                    b'u' => {
                        // Unicode escape: \uXXXX
                        if self.pos + 4 > self.src.len() {
                            return Err(PolicyError::Parse("invalid unicode escape".into()));
                        }
                        let hex = std::str::from_utf8(&self.src[self.pos..self.pos + 4])
                            .map_err(|e| PolicyError::Parse(e.to_string()))?;
                        let code = u32::from_str_radix(hex, 16)
                            .map_err(|_| PolicyError::Parse(format!("invalid unicode: {hex}")))?;
                        let ch = char::from_u32(code).ok_or_else(|| {
                            PolicyError::Parse(format!("invalid codepoint: {code}"))
                        })?;
                        s.push(ch);
                        self.pos += 4;
                        self.col += 4;
                    }
                    other => {
                        s.push('\\');
                        s.push(other as char);
                    }
                }
            } else {
                s.push(c as char);
                self.advance();
            }
        }
        Ok(Token::String(s))
    }

    fn lex_raw_string(&mut self) -> Result<Token, PolicyError> {
        self.advance(); // consume opening `
        let start = self.pos;
        while self.pos < self.src.len() && self.src[self.pos] != b'`' {
            self.advance();
        }
        if self.pos >= self.src.len() {
            return Err(PolicyError::Parse("unterminated raw string".into()));
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|e| PolicyError::Parse(e.to_string()))?
            .to_string();
        self.advance(); // consume closing `
        Ok(Token::String(s))
    }

    fn lex_number(&mut self) -> Result<Token, PolicyError> {
        let start = self.pos;
        // Optional negative sign is handled at the expression level
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
            self.advance();
        }
        // Decimal part
        if self.pos < self.src.len() && self.src[self.pos] == b'.' {
            self.advance();
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                self.advance();
            }
        }
        // Exponent
        if self.pos < self.src.len() && (self.src[self.pos] == b'e' || self.src[self.pos] == b'E') {
            self.advance();
            if self.pos < self.src.len()
                && (self.src[self.pos] == b'+' || self.src[self.pos] == b'-')
            {
                self.advance();
            }
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                self.advance();
            }
        }
        let num = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|e| PolicyError::Parse(e.to_string()))?
            .to_string();
        Ok(Token::Number(num))
    }

    fn lex_ident_or_keyword(&mut self) -> Result<Token, PolicyError> {
        let start = self.pos;
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_alphanumeric() || self.src[self.pos] == b'_')
        {
            self.advance();
        }
        let word = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|e| PolicyError::Parse(e.to_string()))?;
        Ok(match word {
            "null" => Token::Null,
            "true" => Token::True,
            "false" => Token::False,
            "package" => Token::Package,
            "import" => Token::Import,
            "as" => Token::As,
            "default" => Token::Default,
            "not" => Token::Not,
            "some" => Token::Some,
            "every" => Token::Every,
            "in" => Token::In,
            "with" => Token::With,
            "else" => Token::Else,
            "if" => Token::If,
            "contains" => Token::Contains,
            "data" => Token::Data,
            "input" => Token::Input,
            "future" => Token::Future,
            _ => Token::Ident(word.to_string()),
        })
    }
}

/// Tokenize a Rego source string, returning tokens with spans.
pub fn tokenize(src: &str) -> Result<Vec<(Token, Span)>, PolicyError> {
    Lexer::new(src).tokenize()
}
