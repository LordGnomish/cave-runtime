//! PromQL lexer/tokenizer.

#![allow(dead_code)]

use crate::error::{MetricsError, MetricsResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Number(f64),
    Str(String),
    Duration(i64), // milliseconds
    // Brackets
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    // Punctuation
    Comma,
    Semicolon,
    Colon,
    At,
    // Keywords
    Offset,
    By,
    Without,
    Bool,
    On,
    Ignoring,
    GroupLeft,
    GroupRight,
    And,
    Or,
    Unless,
    Not,
    // Comparison
    Eq,      // ==
    NotEq,   // !=
    Lt,      // <
    Gt,      // >
    Lte,     // <=
    Gte,     // >=
    EqTilde, // =~
    NotEqTilde, // !~
    Assign,  // =
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Atan2,
    // End
    Eof,
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    peeked: Option<Token>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            peeked: None,
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.input.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while matches!(self.peek_char(), Some(c) if c.is_whitespace()) {
                self.next_char();
            }
            if self.peek_char() == Some('#') {
                while matches!(self.peek_char(), Some(c) if c != '\n') {
                    self.next_char();
                }
            } else {
                break;
            }
        }
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_alphanumeric() || c == '_' || c == ':' {
                s.push(c);
                self.next_char();
            } else {
                break;
            }
        }
        s
    }

    fn read_number(&mut self) -> MetricsResult<f64> {
        let mut s = String::new();
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
                // Only allow +/- after e/E
                if (c == '+' || c == '-') && !s.ends_with('e') && !s.ends_with('E') {
                    break;
                }
                s.push(c);
                self.next_char();
            } else {
                break;
            }
        }
        // Check if it's a duration: number followed directly by duration suffix
        if let Some(c) = self.peek_char() {
            if matches!(c, 's' | 'm' | 'h' | 'd' | 'w' | 'y') {
                let base: f64 = s.parse().map_err(|_| MetricsError::Parse(format!("bad number: {}", s)))?;
                let mult = self.read_duration_suffix();
                return Ok(base * mult as f64);
                // Caller will decide; but actually we can just return the number and let
                // the caller handle durations via read_duration
            }
        }
        s.parse().map_err(|_| MetricsError::Parse(format!("bad number: {}", s)))
    }

    fn read_duration_suffix(&mut self) -> i64 {
        let ms_per_s = 1000i64;
        match self.peek_char() {
            Some('s') => { self.next_char(); ms_per_s }
            Some('m') => {
                self.next_char();
                if self.peek_char() == Some('s') {
                    self.next_char();
                    1 // 1 ms
                } else {
                    60 * ms_per_s
                }
            }
            Some('h') => { self.next_char(); 3600 * ms_per_s }
            Some('d') => { self.next_char(); 86400 * ms_per_s }
            Some('w') => { self.next_char(); 7 * 86400 * ms_per_s }
            Some('y') => { self.next_char(); 365 * 86400 * ms_per_s }
            _ => 1,
        }
    }

    fn read_string(&mut self, quote: char) -> MetricsResult<String> {
        let mut s = String::new();
        loop {
            match self.next_char() {
                None => return Err(MetricsError::Parse("unterminated string".to_string())),
                Some(c) if c == quote => break,
                Some('\\') => {
                    match self.next_char() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some(c) => s.push(c),
                        None => return Err(MetricsError::Parse("unterminated escape".to_string())),
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn lex_one(&mut self) -> MetricsResult<Token> {
        self.skip_whitespace_and_comments();
        let c = match self.peek_char() {
            None => return Ok(Token::Eof),
            Some(c) => c,
        };

        match c {
            '{' => { self.next_char(); Ok(Token::LBrace) }
            '}' => { self.next_char(); Ok(Token::RBrace) }
            '(' => { self.next_char(); Ok(Token::LParen) }
            ')' => { self.next_char(); Ok(Token::RParen) }
            '[' => { self.next_char(); Ok(Token::LBracket) }
            ']' => { self.next_char(); Ok(Token::RBracket) }
            ',' => { self.next_char(); Ok(Token::Comma) }
            ';' => { self.next_char(); Ok(Token::Semicolon) }
            ':' => { self.next_char(); Ok(Token::Colon) }
            '@' => { self.next_char(); Ok(Token::At) }
            '+' => { self.next_char(); Ok(Token::Add) }
            '-' => { self.next_char(); Ok(Token::Sub) }
            '*' => { self.next_char(); Ok(Token::Mul) }
            '/' => { self.next_char(); Ok(Token::Div) }
            '%' => { self.next_char(); Ok(Token::Mod) }
            '^' => { self.next_char(); Ok(Token::Pow) }
            '=' => {
                self.next_char();
                if self.peek_char() == Some('=') {
                    self.next_char();
                    Ok(Token::Eq)
                } else if self.peek_char() == Some('~') {
                    self.next_char();
                    Ok(Token::EqTilde)
                } else {
                    Ok(Token::Assign)
                }
            }
            '!' => {
                self.next_char();
                if self.peek_char() == Some('=') {
                    self.next_char();
                    Ok(Token::NotEq)
                } else if self.peek_char() == Some('~') {
                    self.next_char();
                    Ok(Token::NotEqTilde)
                } else {
                    Ok(Token::Not)
                }
            }
            '<' => {
                self.next_char();
                if self.peek_char() == Some('=') {
                    self.next_char();
                    Ok(Token::Lte)
                } else {
                    Ok(Token::Lt)
                }
            }
            '>' => {
                self.next_char();
                if self.peek_char() == Some('=') {
                    self.next_char();
                    Ok(Token::Gte)
                } else {
                    Ok(Token::Gt)
                }
            }
            '"' | '\'' | '`' => {
                self.next_char();
                let s = self.read_string(c)?;
                Ok(Token::Str(s))
            }
            _ if c.is_ascii_digit() => {
                // Could be number or duration
                let start = self.pos;
                let mut num_str = String::new();
                while let Some(ch) = self.peek_char() {
                    if ch.is_ascii_digit() || ch == '.' || ch == 'e' || ch == 'E' {
                        num_str.push(ch);
                        self.next_char();
                    } else if (ch == '+' || ch == '-') && (num_str.ends_with('e') || num_str.ends_with('E')) {
                        num_str.push(ch);
                        self.next_char();
                    } else {
                        break;
                    }
                }
                // Check for duration suffix
                if let Some(ch) = self.peek_char() {
                    if matches!(ch, 's' | 'h' | 'd' | 'w' | 'y') || (ch == 'm') {
                        // Parse as duration
                        let base: f64 = num_str.parse().map_err(|_| MetricsError::Parse(format!("bad number at {}: {}", start, num_str)))?;
                        let mult = self.read_duration_suffix();
                        return Ok(Token::Duration((base * mult as f64) as i64));
                    }
                }
                let n: f64 = num_str.parse().map_err(|_| MetricsError::Parse(format!("bad number: {}", num_str)))?;
                Ok(Token::Number(n))
            }
            _ if c.is_alphabetic() || c == '_' => {
                let ident = self.read_ident();
                Ok(match ident.as_str() {
                    "offset" => Token::Offset,
                    "by" => Token::By,
                    "without" => Token::Without,
                    "bool" => Token::Bool,
                    "on" => Token::On,
                    "ignoring" => Token::Ignoring,
                    "group_left" => Token::GroupLeft,
                    "group_right" => Token::GroupRight,
                    "and" => Token::And,
                    "or" => Token::Or,
                    "unless" => Token::Unless,
                    "not" => Token::Not,
                    "atan2" => Token::Atan2,
                    "Inf" | "inf" => Token::Number(f64::INFINITY),
                    "NaN" | "nan" => Token::Number(f64::NAN),
                    _ => Token::Ident(ident),
                })
            }
            other => Err(MetricsError::Parse(format!("unexpected character: {:?}", other))),
        }
    }

    pub fn next(&mut self) -> MetricsResult<Token> {
        if let Some(t) = self.peeked.take() {
            return Ok(t);
        }
        self.lex_one()
    }

    pub fn peek(&mut self) -> MetricsResult<&Token> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lex_one()?);
        }
        Ok(self.peeked.as_ref().unwrap())
    }
}
