// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LogQL lexer — converts a query string into a flat token stream.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Punctuation
    LBrace,        // {
    RBrace,        // }
    LParen,        // (
    RParen,        // )
    LBracket,      // [
    RBracket,      // ]
    Comma,         // ,
    Pipe,          // |
    PipeEq,        // |=
    PipeNeq,       // !=  (in pipeline context)
    PipeTilde,     // |~
    PipeBangTilde, // !~
    Eq,            // =
    Neq,           // !=  (in label context)
    Re,            // =~
    NotRe,         // !~  (in label context)

    // Comparison operators
    Gt,   // >
    Gte,  // >=
    Lt,   // <
    Lte,  // <=
    EqEq, // ==

    // Arithmetic operators
    Plus,    // +
    Minus,   // -
    Star,    // *
    Slash,   // /
    Percent, // %
    Caret,   // ^

    // Keywords
    By,
    Without,
    And,
    Or,
    Unless,
    Bool,
    On,
    Ignoring,
    GroupLeft,
    GroupRight,
    Offset,

    // Range agg functions
    Rate,
    CountOverTime,
    BytesOverTime,
    BytesRate,
    AbsentOverTime,
    SumOverTime,
    AvgOverTime,
    MaxOverTime,
    MinOverTime,
    FirstOverTime,
    LastOverTime,
    StddevOverTime,
    StdvarOverTime,
    QuantileOverTime,

    // Vector agg functions
    Sum,
    Avg,
    Max,
    Min,
    Count,
    Stddev,
    Stdvar,
    Topk,
    Bottomk,
    Quantile,

    // Parser keywords
    Json,
    Logfmt,
    Regexp,
    Pattern,
    Unpack,
    Unwrap,
    Duration,
    Bytes,

    // Format keywords
    LineFormat,
    LabelFormat,
    Decolorize,
    Drop,
    Keep,

    // Literals
    Ident(String),
    Str(String),
    Number(f64),
    /// Duration literal, stored as nanoseconds.
    DurationLit(u64),
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub pos: usize,
    pub msg: String,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lex error at {}: {}", self.pos, self.msg)
    }
}

pub struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                break;
            }
            let tok = self.next_token()?;
            tokens.push(tok);
        }
        Ok(tokens)
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.input.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.input.get(self.pos).copied();
        self.pos += 1;
        b
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        match self.peek() {
            Some(b'{') => {
                self.advance();
                Ok(Token::LBrace)
            }
            Some(b'}') => {
                self.advance();
                Ok(Token::RBrace)
            }
            Some(b'(') => {
                self.advance();
                Ok(Token::LParen)
            }
            Some(b')') => {
                self.advance();
                Ok(Token::RParen)
            }
            Some(b'[') => {
                self.advance();
                Ok(Token::LBracket)
            }
            Some(b']') => {
                self.advance();
                Ok(Token::RBracket)
            }
            Some(b',') => {
                self.advance();
                Ok(Token::Comma)
            }
            Some(b'+') => {
                self.advance();
                Ok(Token::Plus)
            }
            Some(b'*') => {
                self.advance();
                Ok(Token::Star)
            }
            Some(b'%') => {
                self.advance();
                Ok(Token::Percent)
            }
            Some(b'^') => {
                self.advance();
                Ok(Token::Caret)
            }

            Some(b'-') => {
                self.advance();
                Ok(Token::Minus)
            }
            Some(b'/') => {
                self.advance();
                // Skip line comments: // ...
                if self.peek() == Some(b'/') {
                    while self.peek().map_or(false, |b| b != b'\n') {
                        self.advance();
                    }
                    self.skip_whitespace();
                    self.next_token()
                } else {
                    Ok(Token::Slash)
                }
            }

            Some(b'|') => {
                self.advance();
                match self.peek() {
                    Some(b'=') => {
                        self.advance();
                        Ok(Token::PipeEq)
                    }
                    Some(b'~') => {
                        self.advance();
                        Ok(Token::PipeTilde)
                    }
                    _ => Ok(Token::Pipe),
                }
            }
            Some(b'=') => {
                self.advance();
                match self.peek() {
                    Some(b'~') => {
                        self.advance();
                        Ok(Token::Re)
                    }
                    Some(b'=') => {
                        self.advance();
                        Ok(Token::EqEq)
                    }
                    _ => Ok(Token::Eq),
                }
            }
            Some(b'!') => {
                self.advance();
                match self.peek() {
                    Some(b'=') => {
                        self.advance();
                        Ok(Token::Neq)
                    }
                    Some(b'~') => {
                        self.advance();
                        Ok(Token::NotRe)
                    }
                    _ => Err(LexError {
                        pos: start,
                        msg: "unexpected `!`".into(),
                    }),
                }
            }
            Some(b'>') => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token::Gte)
                } else {
                    Ok(Token::Gt)
                }
            }
            Some(b'<') => {
                self.advance();
                if self.peek() == Some(b'=') {
                    self.advance();
                    Ok(Token::Lte)
                } else {
                    Ok(Token::Lt)
                }
            }

            Some(b'"') | Some(b'`') => self.read_string(),
            Some(b'0'..=b'9') => self.read_number_or_duration(),
            Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'_') => self.read_ident_or_keyword(),

            Some(b) => Err(LexError {
                pos: start,
                msg: format!("unexpected byte: {:?}", b as char),
            }),
            None => Err(LexError {
                pos: start,
                msg: "unexpected EOF".into(),
            }),
        }
    }

    fn read_string(&mut self) -> Result<Token, LexError> {
        let quote = self.advance().unwrap();
        let mut s = String::new();
        loop {
            match self.advance() {
                None => {
                    return Err(LexError {
                        pos: self.pos,
                        msg: "unterminated string".into(),
                    });
                }
                Some(b) if b == quote => break,
                Some(b'\\') if quote == b'"' => match self.advance() {
                    Some(b'n') => s.push('\n'),
                    Some(b't') => s.push('\t'),
                    Some(b'r') => s.push('\r'),
                    Some(b'\\') => s.push('\\'),
                    Some(b'"') => s.push('"'),
                    Some(b) => {
                        s.push('\\');
                        s.push(b as char);
                    }
                    None => {
                        return Err(LexError {
                            pos: self.pos,
                            msg: "unterminated escape".into(),
                        });
                    }
                },
                Some(b) => s.push(b as char),
            }
        }
        Ok(Token::Str(s))
    }

    fn read_number_or_duration(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        while self
            .peek()
            .map_or(false, |b| b.is_ascii_digit() || b == b'.')
        {
            self.advance();
        }
        let num_str = std::str::from_utf8(&self.input[start..self.pos]).unwrap();

        // Check for duration suffix: s, m, h, d, w, y, ms, us, µs, ns
        let suffix_start = self.pos;
        while self
            .peek()
            .map_or(false, |b| b.is_ascii_alphabetic() || b == b'\xc2')
        {
            self.advance();
        }
        if self.pos > suffix_start {
            let suffix = std::str::from_utf8(&self.input[suffix_start..self.pos]).unwrap_or("");
            let base: f64 = num_str.parse().map_err(|_| LexError {
                pos: start,
                msg: "bad number".into(),
            })?;
            let ns = match suffix {
                "ns" => base as u64,
                "us" | "µs" => (base * 1_000.0) as u64,
                "ms" => (base * 1_000_000.0) as u64,
                "s" => (base * 1_000_000_000.0) as u64,
                "m" => (base * 60_000_000_000.0) as u64,
                "h" => (base * 3_600_000_000_000.0) as u64,
                "d" => (base * 86_400_000_000_000.0) as u64,
                "w" => (base * 604_800_000_000_000.0) as u64,
                "y" => (base * 31_536_000_000_000_000.0) as u64,
                _ => {
                    return Err(LexError {
                        pos: suffix_start,
                        msg: format!("unknown duration unit: {}", suffix),
                    });
                }
            };
            return Ok(Token::DurationLit(ns));
        }

        let n: f64 = num_str.parse().map_err(|_| LexError {
            pos: start,
            msg: "bad number".into(),
        })?;
        Ok(Token::Number(n))
    }

    fn read_ident_or_keyword(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        while self
            .peek()
            .map_or(false, |b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.advance();
        }
        let word = std::str::from_utf8(&self.input[start..self.pos]).unwrap();
        let tok = match word {
            "by" => Token::By,
            "without" => Token::Without,
            "and" => Token::And,
            "or" => Token::Or,
            "unless" => Token::Unless,
            "bool" => Token::Bool,
            "on" => Token::On,
            "ignoring" => Token::Ignoring,
            "group_left" => Token::GroupLeft,
            "group_right" => Token::GroupRight,
            "offset" => Token::Offset,
            // Range agg functions
            "rate" => Token::Rate,
            "count_over_time" => Token::CountOverTime,
            "bytes_over_time" => Token::BytesOverTime,
            "bytes_rate" => Token::BytesRate,
            "absent_over_time" => Token::AbsentOverTime,
            "sum_over_time" => Token::SumOverTime,
            "avg_over_time" => Token::AvgOverTime,
            "max_over_time" => Token::MaxOverTime,
            "min_over_time" => Token::MinOverTime,
            "first_over_time" => Token::FirstOverTime,
            "last_over_time" => Token::LastOverTime,
            "stddev_over_time" => Token::StddevOverTime,
            "stdvar_over_time" => Token::StdvarOverTime,
            "quantile_over_time" => Token::QuantileOverTime,
            // Vector agg functions
            "sum" => Token::Sum,
            "avg" => Token::Avg,
            "max" => Token::Max,
            "min" => Token::Min,
            "count" => Token::Count,
            "stddev" => Token::Stddev,
            "stdvar" => Token::Stdvar,
            "topk" => Token::Topk,
            "bottomk" => Token::Bottomk,
            "quantile" => Token::Quantile,
            // Parsers
            "json" => Token::Json,
            "logfmt" => Token::Logfmt,
            "regexp" => Token::Regexp,
            "pattern" => Token::Pattern,
            "unpack" => Token::Unpack,
            "unwrap" => Token::Unwrap,
            "duration" => Token::Duration,
            "bytes" => Token::Bytes,
            // Formatters
            "line_format" => Token::LineFormat,
            "label_format" => Token::LabelFormat,
            "decolorize" => Token::Decolorize,
            "drop" => Token::Drop,
            "keep" => Token::Keep,
            _ => Token::Ident(word.to_owned()),
        };
        Ok(tok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(s: &str) -> Vec<Token> {
        Lexer::new(s).tokenize().unwrap()
    }

    #[test]
    fn stream_selector() {
        let toks = lex(r#"{app="nginx",env=~"prod.*"}"#);
        assert!(toks.contains(&Token::LBrace));
        assert!(toks.contains(&Token::RBrace));
        assert!(toks.contains(&Token::Eq));
        assert!(toks.contains(&Token::Re));
    }

    #[test]
    fn pipeline() {
        let toks = lex(r#"{job="x"} |= "error" | json | label_format new=old"#);
        assert!(toks.contains(&Token::PipeEq));
        assert!(toks.contains(&Token::Json));
        assert!(toks.contains(&Token::LabelFormat));
    }

    #[test]
    fn range_agg() {
        let toks = lex("rate({app=\"x\"}[5m])");
        assert!(toks.contains(&Token::Rate));
        assert!(
            matches!(toks.iter().find(|t| matches!(t, Token::DurationLit(_))), Some(Token::DurationLit(ns)) if *ns == 300_000_000_000)
        );
    }

    #[test]
    fn numbers() {
        let toks = lex("42 3.14 100ms 5s 1h");
        assert!(toks.contains(&Token::Number(42.0)));
        assert!(toks.contains(&Token::Number(3.14)));
        assert!(toks.contains(&Token::DurationLit(100_000_000))); // 100ms
        assert!(toks.contains(&Token::DurationLit(5_000_000_000))); // 5s
        assert!(toks.contains(&Token::DurationLit(3_600_000_000_000))); // 1h
    }
}
