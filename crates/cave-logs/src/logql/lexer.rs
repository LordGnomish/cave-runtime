//! LogQL lexer — converts a query string into a flat token list.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Structural
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    // Pipe operators
    Pipe,      // |
    PipeEq,    // |=
    PipeTilde, // |~
    // Comparison / match operators
    Eq,    // =
    Ne,    // !=
    Re,    // =~
    NRe,   // !~
    Gt,    // >
    Gte,   // >=
    Lt,    // <
    Lte,   // <=
    EqEq,  // ==
    Bang,  // ! (standalone, for !~ / != in filter position)
    // Values
    Ident(String),
    Str(String),
    Integer(i64),
    Float(f64),
    Dur(Duration),
    // End
    Eof,
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self { chars: input.chars().collect(), pos: 0 }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                tokens.push(Token::Eof);
                break;
            }
            let tok = self.next_token()?;
            tokens.push(tok);
        }
        Ok(tokens)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn next_token(&mut self) -> Result<Token, String> {
        let c = match self.peek() {
            Some(c) => c,
            None => return Ok(Token::Eof),
        };

        match c {
            '{' => { self.advance(); Ok(Token::LBrace) }
            '}' => { self.advance(); Ok(Token::RBrace) }
            '(' => { self.advance(); Ok(Token::LParen) }
            ')' => { self.advance(); Ok(Token::RParen) }
            '[' => { self.advance(); Ok(Token::LBracket) }
            ']' => { self.advance(); Ok(Token::RBracket) }
            ',' => { self.advance(); Ok(Token::Comma) }

            '|' => {
                self.advance();
                match self.peek() {
                    Some('=') => { self.advance(); Ok(Token::PipeEq) }
                    Some('~') => { self.advance(); Ok(Token::PipeTilde) }
                    _ => Ok(Token::Pipe),
                }
            }

            '=' => {
                self.advance();
                match self.peek() {
                    Some('~') => { self.advance(); Ok(Token::Re) }
                    Some('=') => { self.advance(); Ok(Token::EqEq) }
                    _ => Ok(Token::Eq),
                }
            }

            '!' => {
                self.advance();
                match self.peek() {
                    Some('=') => { self.advance(); Ok(Token::Ne) }
                    Some('~') => { self.advance(); Ok(Token::NRe) }
                    _ => Ok(Token::Bang),
                }
            }

            '>' => {
                self.advance();
                if self.peek() == Some('=') { self.advance(); Ok(Token::Gte) } else { Ok(Token::Gt) }
            }

            '<' => {
                self.advance();
                if self.peek() == Some('=') { self.advance(); Ok(Token::Lte) } else { Ok(Token::Lt) }
            }

            '"' | '\'' | '`' => self.read_string(),

            '0'..='9' => self.read_number_or_duration(),

            'a'..='z' | 'A'..='Z' | '_' => self.read_ident(),

            other => Err(format!("unexpected character: {other:?}")),
        }
    }

    fn read_string(&mut self) -> Result<Token, String> {
        let quote = self.advance().unwrap();
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("unterminated string literal".into()),
                Some(c) if c == quote => break,
                Some('\\') => {
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some(c) => s.push(c),
                        None => return Err("unterminated escape".into()),
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Ok(Token::Str(s))
    }

    fn read_number_or_duration(&mut self) -> Result<Token, String> {
        let start = self.pos;
        let mut has_dot = false;

        while matches!(self.peek(), Some('0'..='9')) {
            self.advance();
        }
        if self.peek() == Some('.') && matches!(self.peek2(), Some('0'..='9')) {
            has_dot = true;
            self.advance();
            while matches!(self.peek(), Some('0'..='9')) {
                self.advance();
            }
        }

        // Check for duration suffix
        let num_str: String = self.chars[start..self.pos].iter().collect();
        let suffix_start = self.pos;
        while matches!(self.peek(), Some('s' | 'm' | 'h' | 'd' | 'w' | 'y' | 'u' | 'n' | 'M')) {
            // collect suffix chars
            self.advance();
        }
        let suffix: String = self.chars[suffix_start..self.pos].iter().collect();

        if !suffix.is_empty() {
            let n: f64 = num_str.parse().map_err(|_| format!("bad number: {num_str}"))?;
            let dur = parse_duration_suffix(n, &suffix)?;
            return Ok(Token::Dur(dur));
        }

        if has_dot {
            let f: f64 = num_str.parse().map_err(|_| format!("bad float: {num_str}"))?;
            Ok(Token::Float(f))
        } else {
            let i: i64 = num_str.parse().map_err(|_| format!("bad int: {num_str}"))?;
            Ok(Token::Integer(i))
        }
    }

    fn read_ident(&mut self) -> Result<Token, String> {
        let start = self.pos;
        while matches!(self.peek(), Some('a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.')) {
            self.advance();
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        Ok(Token::Ident(s))
    }
}

fn parse_duration_suffix(n: f64, suffix: &str) -> Result<Duration, String> {
    let nanos = match suffix {
        "ns" => n,
        "us" | "µs" => n * 1_000.0,
        "ms" => n * 1_000_000.0,
        "s" => n * 1_000_000_000.0,
        "m" => n * 60.0 * 1_000_000_000.0,
        "h" => n * 3600.0 * 1_000_000_000.0,
        "d" => n * 86400.0 * 1_000_000_000.0,
        "w" => n * 7.0 * 86400.0 * 1_000_000_000.0,
        "y" => n * 365.0 * 86400.0 * 1_000_000_000.0,
        other => return Err(format!("unknown duration suffix: {other}")),
    };
    Ok(Duration::from_nanos(nanos as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_stream_selector() {
        let tokens = Lexer::new(r#"{app="foo",env=~"prod|staging"}"#).tokenize().unwrap();
        assert!(matches!(tokens[0], Token::LBrace));
        assert!(matches!(&tokens[1], Token::Ident(s) if s == "app"));
        assert_eq!(tokens[2], Token::Eq);
        assert!(matches!(&tokens[3], Token::Str(s) if s == "foo"));
        assert_eq!(tokens[4], Token::Comma);
        assert!(matches!(&tokens[5], Token::Ident(s) if s == "env"));
        assert_eq!(tokens[6], Token::Re);
    }

    #[test]
    fn lex_pipe_ops() {
        let tokens = Lexer::new(r#"|= "error" |~ "err.*""#).tokenize().unwrap();
        assert_eq!(tokens[0], Token::PipeEq);
        assert!(matches!(&tokens[1], Token::Str(s) if s == "error"));
        assert_eq!(tokens[2], Token::PipeTilde);
    }

    #[test]
    fn lex_duration() {
        let tokens = Lexer::new("5m").tokenize().unwrap();
        assert!(matches!(&tokens[0], Token::Dur(d) if d.as_secs() == 300));
    }
}
