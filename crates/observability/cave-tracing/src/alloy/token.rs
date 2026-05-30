// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lexical tokens of the Alloy configuration syntax.
//!
//! Line-ported from grafana/alloy `syntax/token/token.go` (v1.5.0, Apache-2.0).
//! `Token` is the individual lexical token; `Pos` tracking lives in the scanner.

use std::fmt;

/// An individual Alloy lexical token.
///
/// The ordering mirrors the upstream `const` block so that the band predicates
/// ([`Token::is_literal`], [`Token::is_keyword`], [`Token::is_operator`]) can be
/// expressed as range checks against the `*Beg`/`*End` sentinels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Token {
    /// Invalid token.
    Illegal = 0,
    /// Literal text. Used by the builder for writing tokens; the scanner never
    /// returns `LITERAL`.
    Literal,
    /// End-of-file.
    Eof,
    /// `// Hello, world!`
    Comment,

    // literalBeg
    /// `foobar`
    Ident,
    /// `1234`
    Number,
    /// `1234.0`
    Float,
    /// `"foobar"`
    String,
    // literalEnd

    // keywordBeg
    /// `true` / `false`
    Bool,
    /// `null`
    Null,
    // keywordEnd

    // operatorBeg
    /// `||`
    Or,
    /// `&&`
    And,
    /// `!`
    Not,
    /// `=`
    Assign,
    /// `==`
    Eq,
    /// `!=`
    Neq,
    /// `<`
    Lt,
    /// `<=`
    Lte,
    /// `>`
    Gt,
    /// `>=`
    Gte,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
    /// `^`
    Pow,
    /// `{`
    LCurly,
    /// `}`
    RCurly,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `[`
    LBrack,
    /// `]`
    RBrack,
    /// `,`
    Comma,
    /// `.`
    Dot,
    // operatorEnd

    /// `\n`
    Terminator,
}

/// Lowest precedence — the precedence of non-operator tokens.
pub const LOWEST_PRECEDENCE: i32 = 0;
/// Precedence of a unary operator (`!`, `-`).
pub const UNARY_PRECEDENCE: i32 = 7;
/// The highest precedence level used by the parser.
pub const HIGHEST_PRECEDENCE: i32 = 8;

impl Token {
    /// Maps a string to its keyword token or [`Token::Ident`] if it is not a
    /// keyword. Mirrors `token.Lookup`.
    pub fn lookup(ident: &str) -> Token {
        match ident {
            "true" | "false" => Token::Bool,
            "null" => Token::Null,
            _ => Token::Ident,
        }
    }

    /// Returns the operator precedence of the binary operator `self`. If `self`
    /// is not a binary operator, the result is [`LOWEST_PRECEDENCE`]. Mirrors
    /// `Token.BinaryPrecedence`.
    pub fn binary_precedence(self) -> i32 {
        match self {
            Token::Or => 1,
            Token::And => 2,
            Token::Eq | Token::Neq | Token::Lt | Token::Lte | Token::Gt | Token::Gte => 3,
            Token::Add | Token::Sub => 4,
            Token::Mul | Token::Div | Token::Mod => 5,
            Token::Pow => 6,
            _ => LOWEST_PRECEDENCE,
        }
    }

    /// True if the token corresponds to a keyword (`BOOL`, `NULL`).
    pub fn is_keyword(self) -> bool {
        matches!(self, Token::Bool | Token::Null)
    }

    /// True if the token corresponds to a literal token or identifier
    /// (`IDENT`, `NUMBER`, `FLOAT`, `STRING`).
    pub fn is_literal(self) -> bool {
        matches!(self, Token::Ident | Token::Number | Token::Float | Token::String)
    }

    /// True if the token corresponds to an operator or delimiter (the band
    /// from `||` through `.`).
    pub fn is_operator(self) -> bool {
        (Token::Or..=Token::Dot).contains(&self)
    }

    /// The string representation corresponding to the token. Operators and
    /// delimiters render as their literal; categories render as their name.
    pub fn name(self) -> &'static str {
        match self {
            Token::Illegal => "ILLEGAL",
            Token::Literal => "LITERAL",
            Token::Eof => "EOF",
            Token::Comment => "COMMENT",
            Token::Ident => "IDENT",
            Token::Number => "NUMBER",
            Token::Float => "FLOAT",
            Token::String => "STRING",
            Token::Bool => "BOOL",
            Token::Null => "NULL",
            Token::Or => "||",
            Token::And => "&&",
            Token::Not => "!",
            Token::Assign => "=",
            Token::Eq => "==",
            Token::Neq => "!=",
            Token::Lt => "<",
            Token::Lte => "<=",
            Token::Gt => ">",
            Token::Gte => ">=",
            Token::Add => "+",
            Token::Sub => "-",
            Token::Mul => "*",
            Token::Div => "/",
            Token::Mod => "%",
            Token::Pow => "^",
            Token::LCurly => "{",
            Token::RCurly => "}",
            Token::LParen => "(",
            Token::RParen => ")",
            Token::LBrack => "[",
            Token::RBrack => "]",
            Token::Comma => ",",
            Token::Dot => ".",
            Token::Terminator => "TERMINATOR",
        }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
