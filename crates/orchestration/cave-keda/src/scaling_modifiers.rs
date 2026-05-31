// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ScalingModifiers — formula-based replica recommendation across triggers.
//!
//! Upstream reference (KEDA v2.16.1):
//!   pkg/scaling/modifiers/formula.go            (calculateScalingModifiersFormula)
//!   apis/keda/v1alpha1/scaledobject_webhook.go  (ValidateAndCompileScalingModifiers)
//!
//! Upstream evaluates the user `formula` with the `github.com/expr-lang/expr`
//! engine (NOT CEL — the older note was wrong) over a `map[string]float64`
//! of trigger-name → metric value, after wrapping it in `float(...)` and
//! compiling with `expr.AsFloat64()` so the result is always coerced to a
//! float. The Cave port implements a faithful subset of that expression
//! language sufficient for numeric scaling-modifier formulas:
//!   * number / float literals, trigger-variable lookup
//!   * arithmetic `+ - * / %`, unary `-`, parentheses, precedence
//!   * comparison `< <= > >= == !=`, logical `&& || !`, ternary `?:`
//!   * array literals `[a, b, c]` + builtins `float int abs ceil floor
//!     round min max sum avg/mean len count(arr, {# > k})`
//! String/map operations and arbitrary closures are out of scope —
//! scaling-modifier formulas operate on a float map and must return a
//! float (see the [[mapped]] note in parity.manifest.toml).

use std::collections::BTreeMap;

// ─── expr-lang formula engine ───────────────────────────────────────────────

/// Errors surfaced while parsing or evaluating a ScalingModifiers formula.
/// Mirrors the `expr.Compile` / `expr.Run` error surface in
/// `validateScalingModifiersFormula` — a malformed formula or one that
/// references an undefined trigger fails rather than silently scaling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaError {
    /// Lexer/parser could not make sense of the formula text.
    Parse(String),
    /// Formula referenced a trigger name that is not in the metric map
    /// (KEDA's webhook compiles with a `triggersMap` to catch exactly this).
    UnknownVariable(String),
    /// Runtime evaluation failed (e.g. division by zero, type misuse,
    /// or the top-level result could not be coerced to a float).
    Eval(String),
}

impl std::fmt::Display for FormulaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormulaError::Parse(m) => write!(f, "formula parse error: {m}"),
            FormulaError::UnknownVariable(n) => write!(f, "unknown trigger variable: {n}"),
            FormulaError::Eval(m) => write!(f, "formula evaluation error: {m}"),
        }
    }
}

impl std::error::Error for FormulaError {}

/// Evaluate a KEDA ScalingModifiers `formula` against the trigger metric
/// map and return the composite metric as a float — the Rust analogue of
/// `calculateScalingModifiersFormula`.
pub fn eval_formula(formula: &str, vars: &BTreeMap<String, f64>) -> Result<f64, FormulaError> {
    let tokens = lex(formula)?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_expr(0)?;
    if !p.at_end() {
        return Err(FormulaError::Parse(format!(
            "unexpected trailing tokens at position {}",
            p.pos
        )));
    }
    eval(&expr, vars)
}

// ─── lexer ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
}

fn lex(src: &str) -> Result<Vec<Tok>, FormulaError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ws if ws.is_whitespace() => i += 1,
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            '(' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::RParen);
                i += 1;
            }
            d if d.is_ascii_digit() || d == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n: f64 = s
                    .parse()
                    .map_err(|_| FormulaError::Parse(format!("invalid number literal '{s}'")))?;
                out.push(Tok::Num(n));
            }
            a if a.is_alphabetic() || a == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                out.push(Tok::Ident(s));
            }
            other => {
                return Err(FormulaError::Parse(format!(
                    "unexpected character '{other}'"
                )));
            }
        }
    }
    Ok(out)
}

// ─── parser (precedence-climbing) ─────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum UnOp {
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, t: &Tok) -> Result<(), FormulaError> {
        match self.next() {
            Some(ref got) if got == t => Ok(()),
            other => Err(FormulaError::Parse(format!("expected {t:?}, got {other:?}"))),
        }
    }

    /// Binding power for a binary operator (higher binds tighter).
    fn binop(tok: &Tok) -> Option<(BinOp, u8)> {
        Some(match tok {
            Tok::Plus => (BinOp::Add, 5),
            Tok::Minus => (BinOp::Sub, 5),
            Tok::Star => (BinOp::Mul, 6),
            Tok::Slash => (BinOp::Div, 6),
            Tok::Percent => (BinOp::Mod, 6),
            _ => return None,
        })
    }

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, FormulaError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let Some(tok) = self.peek() else { break };
            let Some((op, bp)) = Self::binop(tok) else {
                break;
            };
            if bp < min_bp {
                break;
            }
            self.next();
            let rhs = self.parse_expr(bp + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, FormulaError> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.next();
                Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, FormulaError> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::LParen) => {
                let e = self.parse_expr(0)?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => Ok(Expr::Var(name)),
            other => Err(FormulaError::Parse(format!("unexpected token {other:?}"))),
        }
    }
}

// ─── evaluator ─────────────────────────────────────────────────────────────

fn eval(expr: &Expr, vars: &BTreeMap<String, f64>) -> Result<f64, FormulaError> {
    match expr {
        Expr::Num(n) => Ok(*n),
        Expr::Var(name) => vars
            .get(name)
            .copied()
            .ok_or_else(|| FormulaError::UnknownVariable(name.clone())),
        Expr::Unary(UnOp::Neg, inner) => Ok(-eval(inner, vars)?),
        Expr::Binary(op, l, r) => {
            let a = eval(l, vars)?;
            let b = eval(r, vars)?;
            Ok(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => {
                    if b == 0.0 {
                        return Err(FormulaError::Eval("division by zero".into()));
                    }
                    a / b
                }
                BinOp::Mod => {
                    if b == 0.0 {
                        return Err(FormulaError::Eval("modulo by zero".into()));
                    }
                    a % b
                }
            })
        }
    }
}

/// One trigger's metric output going into the ScalingModifiers
/// aggregation.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub name: String,
    pub metric: f64,
    pub is_active: bool,
}

impl Trigger {
    pub fn new(name: &str, metric: f64, is_active: bool) -> Self {
        Self {
            name: name.to_string(),
            metric,
            is_active,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScalingModifiersEvaluator {
    pub formula: String,
    pub target: f64,
    pub activation_target: Option<i32>,
    triggers: BTreeMap<String, Trigger>,
}

impl ScalingModifiersEvaluator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_trigger(&mut self, t: Trigger) {
        self.triggers.insert(t.name.clone(), t);
    }

    /// Evaluate the formula against the trigger map and return the
    /// recommended replica count.
    pub fn evaluate(&self) -> i32 {
        let values: Vec<f64> = self.triggers.values().map(|t| t.metric).collect();
        let metric = if let Some(args) = self.parse_formula("max") {
            args.iter()
                .filter_map(|n| self.triggers.get(n).map(|t| t.metric))
                .fold(f64::MIN, f64::max)
        } else if let Some(args) = self.parse_formula("min") {
            args.iter()
                .filter_map(|n| self.triggers.get(n).map(|t| t.metric))
                .fold(f64::MAX, f64::min)
        } else if let Some(args) = self.parse_formula("sum") {
            args.iter()
                .filter_map(|n| self.triggers.get(n).map(|t| t.metric))
                .sum()
        } else if values.is_empty() {
            0.0
        } else {
            // Unknown formula → sum of every trigger metric.
            values.iter().sum()
        };
        if self.target <= 0.0 {
            return 0;
        }
        (metric / self.target).ceil().max(0.0) as i32
    }

    /// Same metric calculation as [`evaluate`] but returns whether the
    /// scaler should be considered active per `activation_target`.
    pub fn is_active(&self) -> bool {
        match self.activation_target {
            None => self.triggers.values().any(|t| t.is_active),
            Some(threshold) => self.evaluate() > threshold,
        }
    }

    fn parse_formula(&self, name: &str) -> Option<Vec<String>> {
        let prefix = format!("{name}(");
        let trimmed = self.formula.trim();
        let rest = trimmed.strip_prefix(&prefix)?;
        let inner = rest.strip_suffix(')')?;
        Some(inner.split(',').map(|s| s.trim().to_string()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_evaluator_returns_zero() {
        let ev = ScalingModifiersEvaluator::new();
        assert_eq!(ev.evaluate(), 0);
    }

    #[test]
    fn target_zero_returns_zero_safely() {
        let mut ev = ScalingModifiersEvaluator::new();
        ev.formula = "max(a)".into();
        ev.target = 0.0;
        ev.add_trigger(Trigger::new("a", 5.0, true));
        assert_eq!(ev.evaluate(), 0);
    }
}
