// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego lexer — converts source text into a flat token stream.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Package,
    Import,
    Default,
    Not,
    As,
    With,
    Some,
    Every,
    In,
    Else,
    If,
    Contains,
    // Literals
    True,
    False,
    Null,
    Number(f64),
    Str(String),
    // Identifiers
    Ident(String),
    // Operators
    ColonEq,  // :=
    EqEq,     // ==
    BangEq,   // !=
    Lt,       // <
    LtEq,     // <=
    Gt,       // >
    GtEq,     // >=
    Eq,       // =
    Plus,     // +
    Minus,    // -
    Star,     // *
    Slash,    // /
    Percent,  // %
    Amp,      // &
    Pipe,     // |
    // Punctuation
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    Semicolon,
    Dot,
    Colon,
    Underscore,
    Newline,
    Eof,
}

pub struct Lexer<'src> {
    src: &'src str,
    pos: usize,
    pub tokens: Vec<(Token, usize)>,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self { src, pos: 0, tokens: Vec::new() }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek2(&self) -> Option<char> {
        let mut it = self.src[self.pos..].chars();
        it.next();
        it.next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }

    fn skip_line_comment(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' { break; }
            self.advance();
        }
    }

    fn read_string(&mut self) -> Result<String, String> {
        // opening `"` already consumed
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("unterminated string".into()),
                Some('"') => return Ok(s),
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some(c) => { s.push('\\'); s.push(c); }
                    None => return Err("unterminated escape".into()),
                },
                Some(c) => s.push(c),
            }
        }
    }

    fn read_raw_string(&mut self) -> Result<String, String> {
        // opening backtick already consumed
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("unterminated raw string".into()),
                Some('`') => return Ok(s),
                Some(c) => s.push(c),
            }
        }
    }

    fn read_number(&mut self, first: char) -> f64 {
        let mut s = String::new();
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '-' || c == '+' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s.parse().unwrap_or(0.0)
    }

    fn keyword_or_ident(s: &str) -> Token {
        match s {
            "package"  => Token::Package,
            "import"   => Token::Import,
            "default"  => Token::Default,
            "not"      => Token::Not,
            "as"       => Token::As,
            "with"     => Token::With,
            "some"     => Token::Some,
            "every"    => Token::Every,
            "in"       => Token::In,
            "else"     => Token::Else,
            "if"       => Token::If,
            "contains" => Token::Contains,
            "true"     => Token::True,
            "false"    => Token::False,
            "null"     => Token::Null,
            _          => Token::Ident(s.to_string()),
        }
    }

    pub fn tokenize(&mut self) -> Result<(), String> {
        loop {
            let start = self.pos;
            match self.peek() {
                None => { self.tokens.push((Token::Eof, start)); break; }
                Some('#') => { self.advance(); self.skip_line_comment(); }
                Some('\n') => {
                    self.advance();
                    self.tokens.push((Token::Newline, start));
                }
                Some(c) if c.is_whitespace() => { self.advance(); }
                Some('"') => {
                    self.advance();
                    let s = self.read_string()?;
                    self.tokens.push((Token::Str(s), start));
                }
                Some('`') => {
                    self.advance();
                    let s = self.read_raw_string()?;
                    self.tokens.push((Token::Str(s), start));
                }
                Some(':') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        self.tokens.push((Token::ColonEq, start));
                    } else {
                        self.tokens.push((Token::Colon, start));
                    }
                }
                Some('=') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        self.tokens.push((Token::EqEq, start));
                    } else {
                        self.tokens.push((Token::Eq, start));
                    }
                }
                Some('!') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        self.tokens.push((Token::BangEq, start));
                    } else {
                        return Err(format!("unexpected char '!' at {start}"));
                    }
                }
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        self.tokens.push((Token::LtEq, start));
                    } else {
                        self.tokens.push((Token::Lt, start));
                    }
                }
                Some('>') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        self.tokens.push((Token::GtEq, start));
                    } else {
                        self.tokens.push((Token::Gt, start));
                    }
                }
                Some('+') => { self.advance(); self.tokens.push((Token::Plus, start)); }
                Some('-') => {
                    self.advance();
                    // negative number?
                    if self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        let n = self.read_number('-');
                        self.tokens.push((Token::Number(n), start));
                    } else {
                        self.tokens.push((Token::Minus, start));
                    }
                }
                Some('*') => { self.advance(); self.tokens.push((Token::Star, start)); }
                Some('/') => { self.advance(); self.tokens.push((Token::Slash, start)); }
                Some('%') => { self.advance(); self.tokens.push((Token::Percent, start)); }
                Some('&') => { self.advance(); self.tokens.push((Token::Amp, start)); }
                Some('|') => { self.advance(); self.tokens.push((Token::Pipe, start)); }
                Some('{') => { self.advance(); self.tokens.push((Token::LBrace, start)); }
                Some('}') => { self.advance(); self.tokens.push((Token::RBrace, start)); }
                Some('[') => { self.advance(); self.tokens.push((Token::LBracket, start)); }
                Some(']') => { self.advance(); self.tokens.push((Token::RBracket, start)); }
                Some('(') => { self.advance(); self.tokens.push((Token::LParen, start)); }
                Some(')') => { self.advance(); self.tokens.push((Token::RParen, start)); }
                Some(',') => { self.advance(); self.tokens.push((Token::Comma, start)); }
                Some(';') => { self.advance(); self.tokens.push((Token::Semicolon, start)); }
                Some('.') => { self.advance(); self.tokens.push((Token::Dot, start)); }
                Some('_') if self.peek2().map(|c| !c.is_alphanumeric() && c != '_').unwrap_or(true) => {
                    self.advance();
                    self.tokens.push((Token::Underscore, start));
                }
                Some(c) if c.is_ascii_digit() => {
                    self.advance();
                    let n = self.read_number(c);
                    self.tokens.push((Token::Number(n), start));
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    let mut ident = String::new();
                    while let Some(c2) = self.peek() {
                        if c2.is_alphanumeric() || c2 == '_' {
                            ident.push(c2);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let tok = Self::keyword_or_ident(&ident);
                    self.tokens.push((tok, start));
                }
                Some(c) => {
                    return Err(format!("unexpected char {c:?} at position {start}"));
                }
            }
        }
        Ok(())
    }
}

/// Lex a Rego source string, stripping newlines that are not statement
/// separators (i.e., consecutive newlines collapse to one).
pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let mut lexer = Lexer::new(src);
    lexer.tokenize()?;
    // Collapse repeated newlines
    let mut out = Vec::with_capacity(lexer.tokens.len());
    let mut last_was_nl = false;
    for (tok, _) in lexer.tokens {
        match &tok {
            Token::Newline => {
                if !last_was_nl {
                    out.push(tok);
                }
                last_was_nl = true;
            }
            _ => {
                last_was_nl = false;
                out.push(tok);
            }
        }
    }
    Ok(out)
}
