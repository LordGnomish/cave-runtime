// SPDX-License-Identifier: AGPL-3.0-or-later
//! Abstract Syntax Tree for the Rego-compatible policy language.

use serde_json::Value;

/// A single token of a Rego source file (used by parser internals).
/// A parsed policy package.
#[derive(Debug, Clone)]
pub struct Policy {
    pub package: Vec<String>,
    pub imports: Vec<Import>,
    pub rules: Vec<Rule>,
}

/// An import declaration.
#[derive(Debug, Clone)]
pub struct Import {
    pub path: Vec<String>,
    pub alias: Option<String>,
}

/// A single rule (complete, partial, or default).
#[derive(Debug, Clone)]
pub struct Rule {
    /// Rule name (e.g., "allow").
    pub name: String,
    /// Optional key for partial rules: `name[key]`.
    pub head_key: Option<Expr>,
    /// Value assigned in the head: `name = value`.  None means `true`.
    pub head_value: Option<Expr>,
    /// Body conditions (empty = unconditional).
    pub body: Vec<Expr>,
    /// Whether this is a `default` rule.
    pub is_default: bool,
    /// The literal default value (only valid when `is_default` is true).
    pub default_value: Option<Value>,
}

/// An expression in the Rego language.
#[derive(Debug, Clone)]
pub enum Expr {
    // ── Literals ──────────────────────────────────────────────────────────────
    Lit(Value),

    // ── References ───────────────────────────────────────────────────────────
    /// A bare variable: `x`, `input`, `data`.
    Var(String),
    /// A dotted/bracketed path: `input.foo`, `data.users[i]`.
    Ref {
        head: Box<Expr>,
        parts: Vec<RefPart>,
    },

    // ── Operations ───────────────────────────────────────────────────────────
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Cmp {
        op: CmpOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `not expr`
    Not(Box<Expr>),

    // ── Binding ───────────────────────────────────────────────────────────────
    /// `name := value`
    Assign {
        name: String,
        value: Box<Expr>,
    },
    /// `left = right`  (unification / equality check)
    Unify {
        left: Box<Expr>,
        right: Box<Expr>,
    },

    // ── Calls ────────────────────────────────────────────────────────────────
    Call {
        func: String,
        args: Vec<Expr>,
    },

    // ── Constructors ─────────────────────────────────────────────────────────
    Array(Vec<Expr>),
    Object(Vec<(Expr, Expr)>),
    Set(Vec<Expr>),

    // ── Comprehensions ───────────────────────────────────────────────────────
    ArrayComp {
        term: Box<Expr>,
        body: Vec<Expr>,
    },
    SetComp {
        term: Box<Expr>,
        body: Vec<Expr>,
    },
    ObjectComp {
        key: Box<Expr>,
        value: Box<Expr>,
        body: Vec<Expr>,
    },
}

/// A single step in a reference path.
#[derive(Debug, Clone)]
pub enum RefPart {
    /// `.foo`
    Key(String),
    /// `[expr]`
    Index(Box<Expr>),
    /// `[_]`  – iterates over all values
    AnyIndex,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
