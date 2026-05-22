// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SQL lexer/tokenizer.

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Select,
    From,
    Where,
    Insert,
    Into,
    Values,
    Update,
    Set,
    Delete,
    Create,
    Drop,
    Table,
    Alter,
    Add,
    Column,
    Index,
    Schema,
    Primary,
    Key,
    Not,
    Null,
    Default,
    Unique,
    Foreign,
    References,
    Check,
    Begin,
    Commit,
    Rollback,
    Savepoint,
    To,
    And,
    Or,
    Like,
    ILike,
    In,
    Is,
    Between,
    Case,
    When,
    Then,
    Else,
    End,
    Join,
    Inner,
    Left,
    Right,
    Full,
    On,
    As,
    Distinct,
    Order,
    By,
    Group,
    Having,
    Limit,
    Offset,
    Cast,
    Explain,
    Show,
    Copy,
    Stdin,
    If,
    Exists,
    Rename,
    Any,
    All,
    Returning,
    Conflict,
    Nothing,
    Do,

    // Identifiers and literals
    Identifier(String),
    QuotedIdentifier(String),
    Integer(i64),
    Float(f64),
    String(String),
    Keyword(String),

    // Operators
    Equal,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Concat,

    // Delimiters
    LeftParen,
    RightParen,
    Comma,
    Semicolon,
    Dot,

    // Special
    Eof,
}

pub struct Lexer {
    input: Vec<char>,
    position: usize,
    current: Option<char>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let current = if chars.is_empty() {
            None
        } else {
            Some(chars[0])
        };
        Lexer {
            input: chars,
            position: 0,
            current,
        }
    }

    fn advance(&mut self) {
        self.position += 1;
        self.current = if self.position < self.input.len() {
            Some(self.input[self.position])
        } else {
            None
        };
    }

    fn peek(&self) -> Option<char> {
        if self.position + 1 < self.input.len() {
            Some(self.input[self.position + 1])
        } else {
            None
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        if self.current == Some('-') && self.peek() == Some('-') {
            while self.current.is_some() && self.current != Some('\n') {
                self.advance();
            }
        }
    }

    fn read_identifier(&mut self) -> String {
        let mut result = String::new();
        while let Some(ch) = self.current {
            if ch.is_alphanumeric() || ch == '_' {
                result.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        result
    }

    fn read_quoted_identifier(&mut self) -> String {
        self.advance(); // skip opening quote
        let mut result = String::new();
        while let Some(ch) = self.current {
            if ch == '"' {
                self.advance();
                if self.current == Some('"') {
                    result.push('"');
                    self.advance();
                } else {
                    break;
                }
            } else {
                result.push(ch);
                self.advance();
            }
        }
        result
    }

    fn read_string(&mut self) -> String {
        self.advance(); // skip opening quote
        let mut result = String::new();
        while let Some(ch) = self.current {
            if ch == '\'' {
                self.advance();
                if self.current == Some('\'') {
                    result.push('\'');
                    self.advance();
                } else {
                    break;
                }
            } else if ch == '\\' && self.peek() == Some('n') {
                result.push('\n');
                self.advance();
                self.advance();
            } else {
                result.push(ch);
                self.advance();
            }
        }
        result
    }

    fn read_number(&mut self) -> Token {
        let mut result = String::new();
        let mut is_float = false;
        while let Some(ch) = self.current {
            if ch.is_numeric() {
                result.push(ch);
                self.advance();
            } else if ch == '.' && !is_float {
                is_float = true;
                result.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        if is_float {
            Token::Float(result.parse().unwrap_or(0.0))
        } else {
            Token::Integer(result.parse().unwrap_or(0))
        }
    }

    pub fn next_token(&mut self) -> Token {
        loop {
            self.skip_whitespace();
            if self.current == Some('-') && self.peek() == Some('-') {
                self.skip_comment();
            } else {
                break;
            }
        }

        match self.current {
            None => Token::Eof,
            Some('(') => {
                self.advance();
                Token::LeftParen
            }
            Some(')') => {
                self.advance();
                Token::RightParen
            }
            Some(',') => {
                self.advance();
                Token::Comma
            }
            Some(';') => {
                self.advance();
                Token::Semicolon
            }
            Some('.') => {
                self.advance();
                Token::Dot
            }
            Some('+') => {
                self.advance();
                Token::Plus
            }
            Some('-') => {
                self.advance();
                Token::Minus
            }
            Some('*') => {
                self.advance();
                Token::Star
            }
            Some('/') => {
                self.advance();
                Token::Slash
            }
            Some('%') => {
                self.advance();
                Token::Percent
            }
            Some('=') => {
                self.advance();
                Token::Equal
            }
            Some('<') => {
                self.advance();
                if self.current == Some('>') {
                    self.advance();
                    Token::NotEqual
                } else if self.current == Some('=') {
                    self.advance();
                    Token::LessEqual
                } else {
                    Token::Less
                }
            }
            Some('>') => {
                self.advance();
                if self.current == Some('=') {
                    self.advance();
                    Token::GreaterEqual
                } else {
                    Token::Greater
                }
            }
            Some('!') => {
                self.advance();
                if self.current == Some('=') {
                    self.advance();
                    Token::NotEqual
                } else {
                    Token::Keyword("!".to_string())
                }
            }
            Some('"') => {
                let ident = self.read_quoted_identifier();
                Token::QuotedIdentifier(ident)
            }
            Some('\'') => {
                let s = self.read_string();
                Token::String(s)
            }
            Some(ch) if ch.is_numeric() => self.read_number(),
            Some(ch) if ch.is_alphabetic() || ch == '_' => {
                let ident = self.read_identifier();
                Self::keyword_or_ident(&ident)
            }
            Some(_) => {
                self.advance();
                self.next_token()
            }
        }
    }

    fn keyword_or_ident(s: &str) -> Token {
        let upper = s.to_uppercase();
        match upper.as_str() {
            "SELECT" => Token::Select,
            "FROM" => Token::From,
            "WHERE" => Token::Where,
            "INSERT" => Token::Insert,
            "INTO" => Token::Into,
            "VALUES" => Token::Values,
            "UPDATE" => Token::Update,
            "SET" => Token::Set,
            "DELETE" => Token::Delete,
            "CREATE" => Token::Create,
            "DROP" => Token::Drop,
            "TABLE" => Token::Table,
            "ALTER" => Token::Alter,
            "ADD" => Token::Add,
            "COLUMN" => Token::Column,
            "INDEX" => Token::Index,
            "SCHEMA" => Token::Schema,
            "PRIMARY" => Token::Primary,
            "KEY" => Token::Key,
            "NOT" => Token::Not,
            "NULL" => Token::Null,
            "DEFAULT" => Token::Default,
            "UNIQUE" => Token::Unique,
            "FOREIGN" => Token::Foreign,
            "REFERENCES" => Token::References,
            "CHECK" => Token::Check,
            "BEGIN" => Token::Begin,
            "COMMIT" => Token::Commit,
            "ROLLBACK" => Token::Rollback,
            "SAVEPOINT" => Token::Savepoint,
            "TO" => Token::To,
            "AND" => Token::And,
            "OR" => Token::Or,
            "LIKE" => Token::Like,
            "ILIKE" => Token::ILike,
            "IN" => Token::In,
            "IS" => Token::Is,
            "BETWEEN" => Token::Between,
            "CASE" => Token::Case,
            "WHEN" => Token::When,
            "THEN" => Token::Then,
            "ELSE" => Token::Else,
            "END" => Token::End,
            "JOIN" => Token::Join,
            "INNER" => Token::Inner,
            "LEFT" => Token::Left,
            "RIGHT" => Token::Right,
            "FULL" => Token::Full,
            "ON" => Token::On,
            "AS" => Token::As,
            "DISTINCT" => Token::Distinct,
            "ORDER" => Token::Order,
            "BY" => Token::By,
            "GROUP" => Token::Group,
            "HAVING" => Token::Having,
            "LIMIT" => Token::Limit,
            "OFFSET" => Token::Offset,
            "CAST" => Token::Cast,
            "EXPLAIN" => Token::Explain,
            "SHOW" => Token::Show,
            "COPY" => Token::Copy,
            "STDIN" => Token::Stdin,
            "IF" => Token::If,
            "EXISTS" => Token::Exists,
            "RENAME" => Token::Rename,
            "ANY" => Token::Any,
            "ALL" => Token::All,
            "RETURNING" => Token::Returning,
            "CONFLICT" => Token::Conflict,
            "NOTHING" => Token::Nothing,
            "DO" => Token::Do,
            _ => Token::Identifier(s.to_string()),
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token();
            let is_eof = token == Token::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_keywords() {
        let mut lexer = Lexer::new("SELECT * FROM users");
        assert_eq!(lexer.next_token(), Token::Select);
        assert_eq!(lexer.next_token(), Token::Star);
        assert_eq!(lexer.next_token(), Token::From);
    }

    #[test]
    fn test_lexer_identifiers() {
        let mut lexer = Lexer::new("foo bar _baz");
        assert!(matches!(lexer.next_token(), Token::Identifier(ref s) if s == "foo"));
        assert!(matches!(lexer.next_token(), Token::Identifier(ref s) if s == "bar"));
        assert!(matches!(lexer.next_token(), Token::Identifier(ref s) if s == "_baz"));
    }

    #[test]
    fn test_lexer_literals() {
        let mut lexer = Lexer::new("123 45.67 'hello' true");
        assert_eq!(lexer.next_token(), Token::Integer(123));
        assert!(matches!(lexer.next_token(), Token::Float(_)));
        assert!(matches!(lexer.next_token(), Token::String(ref s) if s == "hello"));
    }

    #[test]
    fn test_lexer_operators() {
        let mut lexer = Lexer::new("= <> < > <= >= + - * / %");
        assert_eq!(lexer.next_token(), Token::Equal);
        assert_eq!(lexer.next_token(), Token::NotEqual);
        assert_eq!(lexer.next_token(), Token::Less);
    }
}
