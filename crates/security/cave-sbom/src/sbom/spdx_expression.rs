// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/spdx/expression/SpdxExpressionParser.java
//
//! SPDX license expression parser — AND / OR / WITH grammar.
//!
//! SPDX § Annex D defines composite license expressions like
//! `(MIT OR Apache-2.0) AND BSD-3-Clause WITH Bison-exception-2.2`.
//! This module ports the upstream recursive-descent parser so cave-sbom can
//! evaluate license policies against composite expressions instead of
//! treating them as opaque strings.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseExpr {
    /// A license identifier, optionally with a trailing "+" (later versions).
    License { id: String, or_later: bool },
    /// A license identifier with a `WITH` exception clause.
    WithException { id: String, exception: String },
    /// Logical conjunction.
    And(Vec<LicenseExpr>),
    /// Logical disjunction.
    Or(Vec<LicenseExpr>),
}

impl LicenseExpr {
    /// Flatten nested AND/OR with the same operator at the root.
    pub fn flatten(self) -> Self {
        match self {
            Self::And(items) => {
                let mut out = Vec::new();
                for i in items.into_iter().map(Self::flatten) {
                    if let Self::And(sub) = i {
                        out.extend(sub);
                    } else {
                        out.push(i);
                    }
                }
                if out.len() == 1 {
                    out.into_iter().next().unwrap()
                } else {
                    Self::And(out)
                }
            }
            Self::Or(items) => {
                let mut out = Vec::new();
                for i in items.into_iter().map(Self::flatten) {
                    if let Self::Or(sub) = i {
                        out.extend(sub);
                    } else {
                        out.push(i);
                    }
                }
                if out.len() == 1 {
                    out.into_iter().next().unwrap()
                } else {
                    Self::Or(out)
                }
            }
            other => other,
        }
    }

    /// Walk the expression and collect every license identifier mentioned.
    pub fn identifiers(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_ids(&mut out);
        out
    }

    fn collect_ids(&self, out: &mut Vec<String>) {
        match self {
            Self::License { id, .. } => out.push(id.clone()),
            Self::WithException { id, .. } => out.push(id.clone()),
            Self::And(items) | Self::Or(items) => {
                for i in items {
                    i.collect_ids(out);
                }
            }
        }
    }

    /// Evaluate against an allow-list. AND requires every leaf to be in the
    /// list; OR requires at least one. WITH treats the exception as part of
    /// the identifier (composite key `"<id> WITH <exception>"`).
    pub fn satisfied_by_allow_list(&self, allow: &[String]) -> bool {
        match self {
            Self::License { id, .. } => allow.iter().any(|a| a.eq_ignore_ascii_case(id)),
            Self::WithException { id, exception } => {
                let composite = format!("{} WITH {}", id, exception);
                allow.iter().any(|a| a.eq_ignore_ascii_case(&composite))
                    || allow.iter().any(|a| a.eq_ignore_ascii_case(id))
            }
            Self::And(items) => items.iter().all(|i| i.satisfied_by_allow_list(allow)),
            Self::Or(items) => items.iter().any(|i| i.satisfied_by_allow_list(allow)),
        }
    }
}

impl fmt::Display for LicenseExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::License { id, or_later } => {
                write!(f, "{}{}", id, if *or_later { "+" } else { "" })
            }
            Self::WithException { id, exception } => write!(f, "{} WITH {}", id, exception),
            Self::And(items) => {
                let parts: Vec<String> = items.iter().map(|i| i.to_string()).collect();
                write!(f, "({})", parts.join(" AND "))
            }
            Self::Or(items) => {
                let parts: Vec<String> = items.iter().map(|i| i.to_string()).collect();
                write!(f, "({})", parts.join(" OR "))
            }
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("empty expression")]
    Empty,
    #[error("unexpected token at position {pos}: {token:?}")]
    Unexpected { pos: usize, token: String },
    #[error("unbalanced parenthesis")]
    UnbalancedParen,
    #[error("missing license identifier")]
    MissingIdent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    And,
    Or,
    With,
    Plus,
    Ident(String),
}

fn tokenize(s: &str) -> Result<Vec<Token>, ParseError> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = s.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c.is_whitespace() {
            push_ident(&mut out, &mut buf);
        } else if c == '(' {
            push_ident(&mut out, &mut buf);
            out.push(Token::LParen);
        } else if c == ')' {
            push_ident(&mut out, &mut buf);
            out.push(Token::RParen);
        } else if c == '+' {
            push_ident(&mut out, &mut buf);
            out.push(Token::Plus);
        } else if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' {
            buf.push(c);
        } else {
            return Err(ParseError::Unexpected {
                pos: chars.peek().map(|(i, _)| *i).unwrap_or(0),
                token: c.to_string(),
            });
        }
    }
    push_ident(&mut out, &mut buf);
    Ok(out)
}

fn push_ident(out: &mut Vec<Token>, buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    let tok = match buf.to_ascii_uppercase().as_str() {
        "AND" => Token::And,
        "OR" => Token::Or,
        "WITH" => Token::With,
        _ => Token::Ident(std::mem::take(buf)),
    };
    if !matches!(tok, Token::Ident(_)) {
        buf.clear();
    }
    out.push(tok);
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(t: Vec<Token>) -> Self {
        Self { tokens: t, pos: 0 }
    }
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse_or(&mut self) -> Result<LicenseExpr, ParseError> {
        let mut left = self.parse_and()?;
        let mut rest = vec![];
        while matches!(self.peek(), Some(Token::Or)) {
            self.bump();
            rest.push(self.parse_and()?);
        }
        if rest.is_empty() {
            Ok(left)
        } else {
            rest.insert(0, std::mem::replace(&mut left, LicenseExpr::License {
                id: "_".into(),
                or_later: false,
            }));
            Ok(LicenseExpr::Or(rest))
        }
    }

    fn parse_and(&mut self) -> Result<LicenseExpr, ParseError> {
        let mut left = self.parse_atom()?;
        let mut rest = vec![];
        while matches!(self.peek(), Some(Token::And)) {
            self.bump();
            rest.push(self.parse_atom()?);
        }
        if rest.is_empty() {
            Ok(left)
        } else {
            rest.insert(0, std::mem::replace(&mut left, LicenseExpr::License {
                id: "_".into(),
                or_later: false,
            }));
            Ok(LicenseExpr::And(rest))
        }
    }

    fn parse_atom(&mut self) -> Result<LicenseExpr, ParseError> {
        match self.bump() {
            Some(Token::LParen) => {
                let e = self.parse_or()?;
                match self.bump() {
                    Some(Token::RParen) => Ok(e),
                    _ => Err(ParseError::UnbalancedParen),
                }
            }
            Some(Token::Ident(id)) => {
                let or_later = matches!(self.peek(), Some(Token::Plus));
                if or_later {
                    self.bump();
                }
                if matches!(self.peek(), Some(Token::With)) {
                    self.bump();
                    match self.bump() {
                        Some(Token::Ident(ex)) => Ok(LicenseExpr::WithException {
                            id,
                            exception: ex,
                        }),
                        _ => Err(ParseError::MissingIdent),
                    }
                } else {
                    Ok(LicenseExpr::License { id, or_later })
                }
            }
            Some(t) => Err(ParseError::Unexpected {
                pos: self.pos,
                token: format!("{:?}", t),
            }),
            None => Err(ParseError::MissingIdent),
        }
    }
}

pub fn parse_expression(s: &str) -> Result<LicenseExpr, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError::Empty);
    }
    let tokens = tokenize(s)?;
    let mut p = Parser::new(tokens);
    let e = p.parse_or()?;
    if p.peek().is_some() {
        return Err(ParseError::Unexpected {
            pos: p.pos,
            token: format!("{:?}", p.peek().unwrap()),
        });
    }
    Ok(e.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lic(s: &str) -> LicenseExpr {
        LicenseExpr::License {
            id: s.into(),
            or_later: false,
        }
    }

    #[test]
    fn parse_single_license() {
        assert_eq!(parse_expression("MIT").unwrap(), lic("MIT"));
    }

    #[test]
    fn parse_or_later() {
        let e = parse_expression("GPL-2.0+").unwrap();
        assert_eq!(
            e,
            LicenseExpr::License {
                id: "GPL-2.0".into(),
                or_later: true
            }
        );
    }

    #[test]
    fn parse_simple_or() {
        let e = parse_expression("MIT OR Apache-2.0").unwrap();
        assert_eq!(e, LicenseExpr::Or(vec![lic("MIT"), lic("Apache-2.0")]));
    }

    #[test]
    fn parse_simple_and() {
        let e = parse_expression("MIT AND BSD-3-Clause").unwrap();
        assert_eq!(e, LicenseExpr::And(vec![lic("MIT"), lic("BSD-3-Clause")]));
    }

    #[test]
    fn parse_with_exception() {
        let e = parse_expression("GPL-2.0 WITH Classpath-exception-2.0").unwrap();
        assert_eq!(
            e,
            LicenseExpr::WithException {
                id: "GPL-2.0".into(),
                exception: "Classpath-exception-2.0".into()
            }
        );
    }

    #[test]
    fn parse_parens_change_precedence() {
        let e = parse_expression("(MIT OR Apache-2.0) AND BSD-3-Clause").unwrap();
        assert_eq!(
            e,
            LicenseExpr::And(vec![
                LicenseExpr::Or(vec![lic("MIT"), lic("Apache-2.0")]),
                lic("BSD-3-Clause"),
            ])
        );
    }

    #[test]
    fn parse_three_way_or_flattens() {
        let e = parse_expression("MIT OR Apache-2.0 OR BSD-3-Clause").unwrap();
        assert_eq!(
            e,
            LicenseExpr::Or(vec![lic("MIT"), lic("Apache-2.0"), lic("BSD-3-Clause")])
        );
    }

    #[test]
    fn empty_expression_fails() {
        assert_eq!(parse_expression(""), Err(ParseError::Empty));
        assert_eq!(parse_expression("   "), Err(ParseError::Empty));
    }

    #[test]
    fn missing_ident_after_with_fails() {
        assert!(parse_expression("GPL-2.0 WITH").is_err());
    }

    #[test]
    fn unbalanced_paren_fails() {
        assert!(parse_expression("(MIT OR Apache-2.0").is_err());
    }

    #[test]
    fn allow_list_satisfied_for_or() {
        let e = parse_expression("MIT OR GPL-3.0").unwrap();
        assert!(e.satisfied_by_allow_list(&["MIT".into()]));
        assert!(e.satisfied_by_allow_list(&["GPL-3.0".into()]));
        assert!(!e.satisfied_by_allow_list(&["BSD-3-Clause".into()]));
    }

    #[test]
    fn allow_list_satisfied_for_and() {
        let e = parse_expression("MIT AND BSD-3-Clause").unwrap();
        assert!(e.satisfied_by_allow_list(&["MIT".into(), "BSD-3-Clause".into()]));
        assert!(!e.satisfied_by_allow_list(&["MIT".into()]));
    }

    #[test]
    fn identifiers_walks_tree() {
        let e = parse_expression("(MIT OR Apache-2.0) AND BSD-3-Clause").unwrap();
        let mut ids = e.identifiers();
        ids.sort();
        assert_eq!(ids, vec!["Apache-2.0", "BSD-3-Clause", "MIT"]);
    }

    #[test]
    fn display_round_trip_preserves_meaning() {
        let e = parse_expression("MIT AND (Apache-2.0 OR BSD-3-Clause)").unwrap();
        let s = e.to_string();
        let again = parse_expression(&s).unwrap();
        // Flattening means structural equality is preserved.
        assert_eq!(again, e);
    }
}
