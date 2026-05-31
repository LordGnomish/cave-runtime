// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo Workflows `when` conditional evaluator —
//! `argoproj/argo-workflows v4.0.5`
//! (`workflow/controller/operator.go` `shouldExecute` + the expr-lang /
//! govaluate expression layer).
//!
//! A DAG task or Steps step may carry a `when:` clause such as
//! `"{{tasks.flip.outputs.result}} == heads"`. The controller substitutes the
//! `{{...}}` placeholders against the live node context, then evaluates the
//! remaining boolean expression. A task whose `when` resolves to `false` is
//! marked `Skipped` (a fulfilled phase that satisfies its dependents).
//!
//! This is a pure, dependency-light port: `{{...}}` substitution plus a small
//! recursive-descent evaluator over `||` / `&&` / comparison operators
//! (`== != =~ !~ < <= > >=`) with numeric-aware equality and regex matching.

use regex::Regex;
use std::collections::HashMap;

/// Substitute `{{ key }}` placeholders in `expr` from `ctx`. Whitespace inside
/// the braces is trimmed. An unresolved placeholder is an error — mirroring
/// Argo, which fails the node rather than silently treating it as empty.
pub fn substitute(expr: &str, ctx: &HashMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(expr.len());
    let mut rest = expr;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let close = after
            .find("}}")
            .ok_or_else(|| format!("unterminated placeholder in `{}`", expr))?;
        let key = after[..close].trim();
        let value = ctx
            .get(key)
            .ok_or_else(|| format!("unresolved placeholder `{{{{{}}}}}`", key))?;
        out.push_str(value);
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Evaluate a `when` expression after `{{...}}` substitution. Returns the
/// boolean the controller uses to decide whether to execute the node.
pub fn evaluate_when(expr: &str, ctx: &HashMap<String, String>) -> Result<bool, String> {
    let resolved = substitute(expr, ctx)?;
    let tokens = tokenize(&resolved)?;
    let mut p = Parser { tokens: &tokens, pos: 0 };
    let v = p.parse_or()?;
    if p.pos != p.tokens.len() {
        return Err(format!("trailing tokens in `{}`", resolved));
    }
    Ok(v)
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// A string operand (quoted-stripped or a bare word/number).
    Val(String),
    Op(String),
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Result<Vec<Token>, String> {
    let bytes: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            '\'' | '"' => {
                let quote = c;
                let mut val = String::new();
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    val.push(bytes[i]);
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(format!("unterminated string literal in `{}`", s));
                }
                i += 1; // consume closing quote
                out.push(Token::Val(val));
            }
            _ => {
                // Two-char operators first.
                let two: String = bytes[i..(i + 2).min(bytes.len())].iter().collect();
                if matches!(two.as_str(), "==" | "!=" | "=~" | "!~" | "<=" | ">=" | "&&" | "||") {
                    out.push(Token::Op(two));
                    i += 2;
                    continue;
                }
                if matches!(c, '<' | '>') {
                    out.push(Token::Op(c.to_string()));
                    i += 1;
                    continue;
                }
                // Bare word / number — read until whitespace or a delimiter.
                let mut val = String::new();
                while i < bytes.len() {
                    let d = bytes[i];
                    if d.is_whitespace() || matches!(d, '(' | ')') {
                        break;
                    }
                    // Stop before an operator start so `a==b` (no spaces) splits.
                    let two: String = bytes[i..(i + 2).min(bytes.len())].iter().collect();
                    if matches!(two.as_str(), "==" | "!=" | "=~" | "!~" | "<=" | ">=" | "&&" | "||")
                        || matches!(d, '<' | '>')
                    {
                        break;
                    }
                    val.push(d);
                    i += 1;
                }
                if val.is_empty() {
                    return Err(format!("unexpected char `{}` in `{}`", c, s));
                }
                out.push(Token::Val(val));
            }
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn parse_or(&mut self) -> Result<bool, String> {
        let mut acc = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Op(o)) if o == "||") {
            self.pos += 1;
            let rhs = self.parse_and()?;
            acc = acc || rhs;
        }
        Ok(acc)
    }

    fn parse_and(&mut self) -> Result<bool, String> {
        let mut acc = self.parse_cmp()?;
        while matches!(self.peek(), Some(Token::Op(o)) if o == "&&") {
            self.pos += 1;
            let rhs = self.parse_cmp()?;
            acc = acc && rhs;
        }
        Ok(acc)
    }

    fn parse_cmp(&mut self) -> Result<bool, String> {
        // A parenthesised boolean sub-expression.
        if matches!(self.peek(), Some(Token::LParen)) {
            self.pos += 1;
            let v = self.parse_or()?;
            if !matches!(self.peek(), Some(Token::RParen)) {
                return Err("missing closing paren".into());
            }
            self.pos += 1;
            return Ok(v);
        }
        let lhs = match self.peek() {
            Some(Token::Val(v)) => v.clone(),
            other => return Err(format!("expected operand, got {:?}", other)),
        };
        self.pos += 1;
        // Bare boolean (no operator follows, or the next token ends the clause).
        let op = match self.peek() {
            Some(Token::Op(o)) if o != "&&" && o != "||" => o.clone(),
            _ => return parse_bool_literal(&lhs),
        };
        self.pos += 1;
        let rhs = match self.peek() {
            Some(Token::Val(v)) => v.clone(),
            other => return Err(format!("expected right operand, got {:?}", other)),
        };
        self.pos += 1;
        compare(&lhs, &op, &rhs)
    }
}

fn parse_bool_literal(v: &str) -> Result<bool, String> {
    match v {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected boolean, got `{}`", other)),
    }
}

fn compare(lhs: &str, op: &str, rhs: &str) -> Result<bool, String> {
    match op {
        "==" => Ok(eq(lhs, rhs)),
        "!=" => Ok(!eq(lhs, rhs)),
        "=~" => regex_match(lhs, rhs),
        "!~" => regex_match(lhs, rhs).map(|m| !m),
        "<" | "<=" | ">" | ">=" => {
            let a: f64 = lhs
                .parse()
                .map_err(|_| format!("`{}` is not numeric for `{}`", lhs, op))?;
            let b: f64 = rhs
                .parse()
                .map_err(|_| format!("`{}` is not numeric for `{}`", rhs, op))?;
            Ok(match op {
                "<" => a < b,
                "<=" => a <= b,
                ">" => a > b,
                _ => a >= b,
            })
        }
        other => Err(format!("unknown operator `{}`", other)),
    }
}

/// Numeric-aware equality: if both sides parse as numbers compare numerically,
/// else compare as strings.
fn eq(a: &str, b: &str) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn regex_match(value: &str, pattern: &str) -> Result<bool, String> {
    let re = Regex::new(pattern).map_err(|e| format!("bad regex `{}`: {}", pattern, e))?;
    Ok(re.is_match(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn substitute_resolves_placeholders() {
        let c = ctx(&[("tasks.flip.outputs.result", "heads")]);
        assert_eq!(
            substitute("{{tasks.flip.outputs.result}} == heads", &c).unwrap(),
            "heads == heads"
        );
    }

    #[test]
    fn substitute_trims_inner_whitespace() {
        let c = ctx(&[("x", "1")]);
        assert_eq!(substitute("{{ x }} > 0", &c).unwrap(), "1 > 0");
    }

    #[test]
    fn substitute_errors_on_unresolved() {
        let c = ctx(&[]);
        assert!(substitute("{{missing}} == 1", &c).is_err());
    }

    #[test]
    fn eval_string_equality() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("foo == foo", &c), Ok(true));
        assert_eq!(evaluate_when("foo == bar", &c), Ok(false));
    }

    #[test]
    fn eval_string_inequality() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("foo != bar", &c), Ok(true));
        assert_eq!(evaluate_when("foo != foo", &c), Ok(false));
    }

    #[test]
    fn eval_quoted_operands() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("\"a b\" == \"a b\"", &c), Ok(true));
        assert_eq!(evaluate_when("'x' != 'y'", &c), Ok(true));
    }

    #[test]
    fn eval_numeric_comparison() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("5 > 3", &c), Ok(true));
        assert_eq!(evaluate_when("2 > 3", &c), Ok(false));
        assert_eq!(evaluate_when("3 >= 3", &c), Ok(true));
        assert_eq!(evaluate_when("2 <= 1", &c), Ok(false));
        assert_eq!(evaluate_when("4 < 10", &c), Ok(true));
    }

    #[test]
    fn eval_numeric_aware_equality() {
        let c = ctx(&[]);
        // "5" == "5.0" — numeric-aware so equal even though the strings differ.
        assert_eq!(evaluate_when("5 == 5.0", &c), Ok(true));
    }

    #[test]
    fn eval_regex_match() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("heads =~ h.*", &c), Ok(true));
        assert_eq!(evaluate_when("tails =~ h.*", &c), Ok(false));
        assert_eq!(evaluate_when("tails !~ h.*", &c), Ok(true));
    }

    #[test]
    fn eval_boolean_and_or() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("a == a && b == b", &c), Ok(true));
        assert_eq!(evaluate_when("a == a && b == c", &c), Ok(false));
        assert_eq!(evaluate_when("a == x || b == b", &c), Ok(true));
        assert_eq!(evaluate_when("a == x || b == y", &c), Ok(false));
    }

    #[test]
    fn eval_precedence_or_of_ands() {
        let c = ctx(&[]);
        // (false && false) || true  ==>  true
        assert_eq!(evaluate_when("a == x && b == y || c == c", &c), Ok(true));
    }

    #[test]
    fn eval_bare_boolean_literal() {
        let c = ctx(&[]);
        assert_eq!(evaluate_when("true", &c), Ok(true));
        assert_eq!(evaluate_when("false", &c), Ok(false));
    }

    #[test]
    fn eval_end_to_end_with_substitution() {
        let c = ctx(&[("tasks.flip.outputs.result", "heads")]);
        assert_eq!(
            evaluate_when("{{tasks.flip.outputs.result}} == heads", &c),
            Ok(true)
        );
        let c2 = ctx(&[("tasks.flip.outputs.result", "tails")]);
        assert_eq!(
            evaluate_when("{{tasks.flip.outputs.result}} == heads", &c2),
            Ok(false)
        );
    }

    #[test]
    fn eval_numeric_lt_requires_numbers() {
        let c = ctx(&[]);
        assert!(evaluate_when("foo < bar", &c).is_err());
    }
}
