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
    Ok(eval(&expr, vars)?.as_num())
}

/// A runtime value during formula evaluation. expr-lang is dynamically
/// typed; we carry the numeric and boolean shapes a scaling-modifier
/// formula can produce.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    Num(f64),
    Bool(bool),
}

impl Value {
    /// Coerce to a float, exactly as upstream's `float(...)` /
    /// `expr.AsFloat64()` do (`true → 1.0`, `false → 0.0`).
    fn as_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
        }
    }
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
    Comma,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    Bang,
    Question,
    Colon,
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
            ',' => {
                out.push(Tok::Comma);
                i += 1;
            }
            '?' => {
                out.push(Tok::Question);
                i += 1;
            }
            ':' => {
                out.push(Tok::Colon);
                i += 1;
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::Le);
                    i += 2;
                } else {
                    out.push(Tok::Lt);
                    i += 1;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::Ge);
                    i += 2;
                } else {
                    out.push(Tok::Gt);
                    i += 1;
                }
            }
            '=' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::EqEq);
                    i += 2;
                } else {
                    return Err(FormulaError::Parse("bare '=' (use '==')".into()));
                }
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    out.push(Tok::Ne);
                    i += 2;
                } else {
                    out.push(Tok::Bang);
                    i += 1;
                }
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    out.push(Tok::AndAnd);
                    i += 2;
                } else {
                    return Err(FormulaError::Parse("bare '&' (use '&&')".into()));
                }
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    out.push(Tok::OrOr);
                    i += 2;
                } else {
                    return Err(FormulaError::Parse("bare '|' (use '||')".into()));
                }
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
                // expr-lang word-operators.
                match s.as_str() {
                    "and" => out.push(Tok::AndAnd),
                    "or" => out.push(Tok::OrOr),
                    "not" => out.push(Tok::Bang),
                    _ => out.push(Tok::Ident(s)),
                }
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
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
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
            Tok::OrOr => (BinOp::Or, 1),
            Tok::AndAnd => (BinOp::And, 2),
            Tok::EqEq => (BinOp::Eq, 3),
            Tok::Ne => (BinOp::Ne, 3),
            Tok::Lt => (BinOp::Lt, 4),
            Tok::Le => (BinOp::Le, 4),
            Tok::Gt => (BinOp::Gt, 4),
            Tok::Ge => (BinOp::Ge, 4),
            Tok::Plus => (BinOp::Add, 5),
            Tok::Minus => (BinOp::Sub, 5),
            Tok::Star => (BinOp::Mul, 6),
            Tok::Slash => (BinOp::Div, 6),
            Tok::Percent => (BinOp::Mod, 6),
            _ => return None,
        })
    }

    /// Precedence-climbing expression parser. `min_bp` is the minimum
    /// binding power this call will consume. Ternary `?:` has the lowest
    /// precedence and is handled only at the top (`min_bp == 0`).
    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, FormulaError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let Some(tok) = self.peek() else { break };
            if matches!(tok, Tok::Question) && min_bp == 0 {
                self.next();
                let then_branch = self.parse_expr(0)?;
                self.expect(&Tok::Colon)?;
                let else_branch = self.parse_expr(0)?;
                lhs = Expr::Ternary(Box::new(lhs), Box::new(then_branch), Box::new(else_branch));
                continue;
            }
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
            Some(Tok::Bang) => {
                self.next();
                Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)))
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
            Some(Tok::Ident(name)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.next();
                    let args = self.parse_arg_list(&Tok::RParen)?;
                    self.expect(&Tok::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            other => Err(FormulaError::Parse(format!("unexpected token {other:?}"))),
        }
    }

    /// Parse a comma-separated argument list up to (but not consuming)
    /// `close`.
    fn parse_arg_list(&mut self, close: &Tok) -> Result<Vec<Expr>, FormulaError> {
        let mut args = Vec::new();
        if self.peek() != Some(close) {
            loop {
                args.push(self.parse_expr(0)?);
                match self.peek() {
                    Some(Tok::Comma) => {
                        self.next();
                    }
                    _ => break,
                }
            }
        }
        Ok(args)
    }
}

// ─── evaluator ─────────────────────────────────────────────────────────────

fn eval(expr: &Expr, vars: &BTreeMap<String, f64>) -> Result<Value, FormulaError> {
    match expr {
        Expr::Num(n) => Ok(Value::Num(*n)),
        Expr::Var(name) => vars
            .get(name)
            .map(|v| Value::Num(*v))
            .ok_or_else(|| FormulaError::UnknownVariable(name.clone())),
        Expr::Unary(op, inner) => {
            let v = eval(inner, vars)?;
            match op {
                UnOp::Neg => Ok(Value::Num(-v.as_num())),
                UnOp::Not => Ok(Value::Bool(!v.truthy())),
            }
        }
        Expr::Ternary(cond, a, b) => {
            if eval(cond, vars)?.truthy() {
                eval(a, vars)
            } else {
                eval(b, vars)
            }
        }
        Expr::Binary(op, l, r) => eval_binary(*op, l, r, vars),
        Expr::Call(name, args) => eval_call(name, args, vars),
    }
}

fn eval_binary(
    op: BinOp,
    l: &Expr,
    r: &Expr,
    vars: &BTreeMap<String, f64>,
) -> Result<Value, FormulaError> {
    // Short-circuit logical operators.
    if op == BinOp::And {
        return Ok(Value::Bool(eval(l, vars)?.truthy() && eval(r, vars)?.truthy()));
    }
    if op == BinOp::Or {
        return Ok(Value::Bool(eval(l, vars)?.truthy() || eval(r, vars)?.truthy()));
    }
    let a = eval(l, vars)?.as_num();
    let b = eval(r, vars)?.as_num();
    Ok(match op {
        BinOp::Add => Value::Num(a + b),
        BinOp::Sub => Value::Num(a - b),
        BinOp::Mul => Value::Num(a * b),
        BinOp::Div => {
            if b == 0.0 {
                return Err(FormulaError::Eval("division by zero".into()));
            }
            Value::Num(a / b)
        }
        BinOp::Mod => {
            if b == 0.0 {
                return Err(FormulaError::Eval("modulo by zero".into()));
            }
            Value::Num(a % b)
        }
        BinOp::Lt => Value::Bool(a < b),
        BinOp::Le => Value::Bool(a <= b),
        BinOp::Gt => Value::Bool(a > b),
        BinOp::Ge => Value::Bool(a >= b),
        BinOp::Eq => Value::Bool(a == b),
        BinOp::Ne => Value::Bool(a != b),
        BinOp::And | BinOp::Or => unreachable!("handled above"),
    })
}

fn eval_call(name: &str, args: &[Expr], vars: &BTreeMap<String, f64>) -> Result<Value, FormulaError> {
    let one = |args: &[Expr]| -> Result<f64, FormulaError> {
        if args.len() != 1 {
            return Err(FormulaError::Eval(format!("{name}() expects 1 argument")));
        }
        Ok(eval(&args[0], vars)?.as_num())
    };
    let v = match name {
        // expr-lang float()/int() coercions — the wrapper KEDA applies.
        "float" => one(args)?,
        "int" => one(args)?.trunc(),
        other => return Err(FormulaError::Eval(format!("unknown function '{other}'"))),
    };
    Ok(Value::Num(v))
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
