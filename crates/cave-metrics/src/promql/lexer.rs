//! PromQL lexer — tokenises a query string into tokens.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    StringLit(String),
    Ident(String),

    // Selectors
    LBrace,   // {
    RBrace,   // }
    LBracket, // [
    RBracket, // ]
    LParen,   // (
    RParen,   // )
    Comma,
    Colon,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Eq,       // ==
    Ne,       // !=
    Lt,       // <
    Le,       // <=
    Gt,       // >
    Ge,       // >=
    And,
    Or,
    Unless,
    Atan2,

    // Label matching
    EqMatch,     // =
    NeMatch,     // !=  (also Ne above for comparison)
    ReMatch,     // =~
    NreMatch,    // !~

    // Aggregation keywords
    By,
    Without,
    On,
    Ignoring,
    GroupLeft,
    GroupRight,
    Offset,
    Bool,

    // Duration / @
    Duration(i64), // value in milliseconds
    At,            // @

    // Aggregation ops
    Sum, Min, Max, Avg, Count, Stddev, Stdvar,
    Topk, Bottomk, Quantile, CountValues, Group,

    // Functions (ident enough; resolved by name)

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

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.advance();
        }
    }

    fn skip_line_comment(&mut self) {
        while matches!(self.peek(), Some(c) if c != '\n') {
            self.advance();
        }
    }

    fn read_string(&mut self, delimiter: char) -> Token {
        let mut s = String::new();
        loop {
            match self.advance() {
                None | Some('\n')      => break,
                Some(c) if c == delimiter => break,
                Some('\\') => {
                    match self.advance() {
                        Some('n')  => s.push('\n'),
                        Some('t')  => s.push('\t'),
                        Some('\\') => s.push('\\'),
                        Some(c)    => { s.push('\\'); s.push(c); }
                        None       => break,
                    }
                }
                Some(c) => s.push(c),
            }
        }
        Token::StringLit(s)
    }

    fn read_number(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.') {
            s.push(self.advance().unwrap());
        }
        // Scientific notation
        if matches!(self.peek(), Some('e') | Some('E')) {
            s.push(self.advance().unwrap());
            if matches!(self.peek(), Some('+') | Some('-')) {
                s.push(self.advance().unwrap());
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                s.push(self.advance().unwrap());
            }
        }
        // Duration check: 5m, 1h, etc.  (only if suffix follows immediately)
        if let Some(suffix_start) = self.peek() {
            if suffix_start.is_alphabetic() {
                let mut suffix = String::new();
                while matches!(self.peek(), Some(c) if c.is_alphanumeric()) {
                    suffix.push(self.advance().unwrap());
                }
                if let Ok(base) = s.parse::<f64>() {
                    if let Some(ms) = parse_duration_suffix(base, &suffix) {
                        return Token::Duration(ms);
                    }
                    // It's a number followed by an identifier — put back
                    // We can't actually put back, so return a best-effort number.
                }
                return Token::Duration(0); // fallback
            }
        }
        Token::Number(s.parse().unwrap_or(f64::NAN))
    }

    fn read_ident(&mut self, first: char) -> Token {
        let mut s = String::new();
        s.push(first);
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_' || c == ':') {
            s.push(self.advance().unwrap());
        }
        keyword_or_ident(s)
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => { tokens.push(Token::Eof); break; }
                Some('#') => { self.advance(); self.skip_line_comment(); }
                Some('"') | Some('\'') | Some('`') => {
                    let d = self.advance().unwrap();
                    tokens.push(self.read_string(d));
                }
                Some(c) if c.is_ascii_digit() || (c == '.' && matches!(self.peek2(), Some(d) if d.is_ascii_digit())) => {
                    self.advance();
                    tokens.push(self.read_number(c));
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    self.advance();
                    tokens.push(self.read_ident(c));
                }
                Some(c) => {
                    self.advance();
                    let tok = match c {
                        '{' => Token::LBrace,
                        '}' => Token::RBrace,
                        '[' => Token::LBracket,
                        ']' => Token::RBracket,
                        '(' => Token::LParen,
                        ')' => Token::RParen,
                        ',' => Token::Comma,
                        ':' => Token::Colon,
                        '+' => Token::Plus,
                        '*' => Token::Star,
                        '%' => Token::Percent,
                        '^' => Token::Caret,
                        '@' => Token::At,
                        '-' => Token::Minus,
                        '=' => {
                            if self.peek() == Some('=') { self.advance(); Token::Eq }
                            else if self.peek() == Some('~') { self.advance(); Token::ReMatch }
                            else { Token::EqMatch }
                        }
                        '!' => {
                            if self.peek() == Some('=') { self.advance(); Token::Ne }
                            else if self.peek() == Some('~') { self.advance(); Token::NreMatch }
                            else { Token::Ne }
                        }
                        '<' => {
                            if self.peek() == Some('=') { self.advance(); Token::Le }
                            else { Token::Lt }
                        }
                        '>' => {
                            if self.peek() == Some('=') { self.advance(); Token::Ge }
                            else { Token::Gt }
                        }
                        _ => continue,
                    };
                    tokens.push(tok);
                }
            }
        }
        tokens
    }
}

fn parse_duration_suffix(base: f64, suffix: &str) -> Option<i64> {
    // Handle compound: "1h30m" is complex; we handle simple ones.
    let ms: f64 = match suffix {
        "ms" => base,
        "s"  => base * 1_000.0,
        "m"  => base * 60_000.0,
        "h"  => base * 3_600_000.0,
        "d"  => base * 86_400_000.0,
        "w"  => base * 604_800_000.0,
        "y"  => base * 31_536_000_000.0,
        _    => return None,
    };
    Some(ms as i64)
}

fn keyword_or_ident(s: String) -> Token {
    match s.as_str() {
        "and"        => Token::And,
        "or"         => Token::Or,
        "unless"     => Token::Unless,
        "atan2"      => Token::Atan2,
        "by"         => Token::By,
        "without"    => Token::Without,
        "on"         => Token::On,
        "ignoring"   => Token::Ignoring,
        "group_left" => Token::GroupLeft,
        "group_right"=> Token::GroupRight,
        "offset"     => Token::Offset,
        "bool"       => Token::Bool,
        "sum"        => Token::Sum,
        "min"        => Token::Min,
        "max"        => Token::Max,
        "avg"        => Token::Avg,
        "count"      => Token::Count,
        "stddev"     => Token::Stddev,
        "stdvar"     => Token::Stdvar,
        "topk"       => Token::Topk,
        "bottomk"    => Token::Bottomk,
        "quantile"   => Token::Quantile,
        "count_values" => Token::CountValues,
        "group"      => Token::Group,
        "Inf" | "inf" => Token::Number(f64::INFINITY),
        "NaN" | "nan" => Token::Number(f64::NAN),
        _            => Token::Ident(s),
    }
}
