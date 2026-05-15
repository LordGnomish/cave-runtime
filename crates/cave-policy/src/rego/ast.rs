// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego AST — covers the full OPA Rego language grammar.

/// A complete Rego module (one .rego file).
#[derive(Debug, Clone)]
pub struct Module {
    pub package: Package,
    pub imports: Vec<Import>,
    pub rules: Vec<Rule>,
    pub comments: Vec<String>,
}

/// `package foo.bar.baz`
#[derive(Debug, Clone)]
pub struct Package {
    pub path: Vec<String>,
}

impl Package {
    pub fn to_dot_string(&self) -> String {
        self.path.join(".")
    }
}

/// `import data.foo.bar` or `import future.keywords.every`
#[derive(Debug, Clone)]
pub struct Import {
    pub path: Vec<String>,
    pub alias: Option<String>,
}

/// A Rego rule — one of: complete, partial-set, partial-object, function, default.
#[derive(Debug, Clone)]
pub struct Rule {
    /// `default allow = false`
    pub is_default: bool,
    pub head: RuleHead,
    /// Multiple bodies = disjunction (logical OR).
    pub bodies: Vec<Body>,
    /// `else = val { ... }`
    pub else_rules: Vec<ElseRule>,
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone)]
pub struct ElseRule {
    pub value: Option<Term>,
    pub body: Body,
}

/// Rule head.
#[derive(Debug, Clone)]
pub struct RuleHead {
    /// Rule name (simple) or a ref path for nested rules.
    pub name: String,
    /// Function arguments: `f(x, y)`.
    pub args: Vec<Term>,
    /// Set/object key: `violations[msg]`.
    pub key: Option<Term>,
    /// Assigned value: `allow = true`.
    pub value: Option<Term>,
    /// `contains` keyword for multi-value rules.
    pub contains: bool,
}

/// A rule body: conjunction of expressions.
pub type Body = Vec<Expr>;

/// A single expression (literal) in a rule body.
#[derive(Debug, Clone)]
pub enum Expr {
    /// A term used as a boolean condition: `x > 0`, `allow`, `f(x)`
    Term(Term),
    /// `x = y` — unification
    Unify(Term, Term),
    /// `x := y` — local assignment
    Assign(Term, Term),
    /// `not expr` — negation as failure
    Not(Box<Expr>),
    /// `not { body }` — negation over a body
    NotBody(Body),
    /// `every x in domain { body }`
    Every {
        key: Option<String>,
        value: String,
        domain: Term,
        body: Body,
    },
    /// `some x` — declare existential variable
    Some(Vec<String>),
    /// `some x, y in domain` — existential over collection
    SomeIn {
        key: Option<Term>,
        value: Term,
        domain: Term,
    },
    /// `expr with data.x as val`
    With {
        base: Box<Expr>,
        targets: Vec<WithTarget>,
    },
}

#[derive(Debug, Clone)]
pub struct WithTarget {
    pub path: Vec<String>,
    pub value: Term,
}

/// A Rego term — the fundamental value expression.
#[derive(Debug, Clone)]
pub enum Term {
    Null,
    Bool(bool),
    /// Stored as original text to avoid f64 precision issues.
    Number(String),
    String(String),
    /// A variable reference.
    Var(String),
    /// A compound reference: `data.foo["bar"][i]`
    Ref(Box<Term>, Vec<RefArg>),
    /// `[1, 2, 3]`
    Array(Vec<Term>),
    /// `{"a": 1, "b": 2}`
    Object(Vec<(Term, Term)>),
    /// `{1, 2, 3}`
    Set(Vec<Term>),
    /// `[x | x := arr[_]; x > 0]`
    ArrayCompr {
        term: Box<Term>,
        body: Body,
    },
    /// `{x | x := arr[_]}`
    SetCompr {
        term: Box<Term>,
        body: Body,
    },
    /// `{k: v | ...}`
    ObjectCompr {
        key: Box<Term>,
        value: Box<Term>,
        body: Body,
    },
    /// `f(a, b)` — function call, head is a Ref
    Call {
        func: Box<Term>,
        args: Vec<Term>,
    },
    /// `_` wildcard
    Wildcard,
}

/// An element in a ref: `.field` or `[term]`.
#[derive(Debug, Clone)]
pub enum RefArg {
    /// `.fieldname`
    Field(String),
    /// `[term]`
    Index(Term),
}

/// Metadata annotation (from `# METADATA` comments).
#[derive(Debug, Clone)]
pub struct Annotation {
    pub scope: String, // "rule" | "package" | "document"
    pub title: Option<String>,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub organizations: Vec<String>,
    pub related_resources: Vec<String>,
    pub custom: serde_json::Value,
    pub schemas: Vec<SchemaAnnotation>,
    pub entrypoint: bool,
}

#[derive(Debug, Clone)]
pub struct SchemaAnnotation {
    pub path: Vec<String>,
    pub schema: Option<serde_json::Value>,
    pub definition: Option<serde_json::Value>,
}

impl Term {
    /// Returns true if this term contains no variables (is fully concrete).
    pub fn is_ground(&self) -> bool {
        match self {
            Term::Null | Term::Bool(_) | Term::Number(_) | Term::String(_) => true,
            Term::Wildcard => false,
            Term::Var(_) => false,
            Term::Ref(base, args) => {
                base.is_ground()
                    && args.iter().all(|a| match a {
                        RefArg::Field(_) => true,
                        RefArg::Index(t) => t.is_ground(),
                    })
            }
            Term::Array(ts) => ts.iter().all(|t| t.is_ground()),
            Term::Object(kvs) => kvs.iter().all(|(k, v)| k.is_ground() && v.is_ground()),
            Term::Set(ts) => ts.iter().all(|t| t.is_ground()),
            Term::Call { args, .. } => args.iter().all(|t| t.is_ground()),
            // Comprehensions always introduce new scope
            Term::ArrayCompr { .. } | Term::SetCompr { .. } | Term::ObjectCompr { .. } => false,
        }
    }
}

impl std::fmt::Display for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Term::Null => write!(f, "null"),
            Term::Bool(b) => write!(f, "{b}"),
            Term::Number(n) => write!(f, "{n}"),
            Term::String(s) => write!(f, "\"{s}\""),
            Term::Var(v) => write!(f, "{v}"),
            Term::Wildcard => write!(f, "_"),
            Term::Array(items) => {
                let s: Vec<_> = items.iter().map(|x| x.to_string()).collect();
                write!(f, "[{}]", s.join(", "))
            }
            Term::Set(items) => {
                let s: Vec<_> = items.iter().map(|x| x.to_string()).collect();
                write!(f, "{{{}}}", s.join(", "))
            }
            Term::Object(kvs) => {
                let s: Vec<_> = kvs
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect();
                write!(f, "{{{}}}", s.join(", "))
            }
            Term::Ref(base, args) => {
                write!(f, "{base}")?;
                for arg in args {
                    match arg {
                        RefArg::Field(name) => write!(f, ".{name}")?,
                        RefArg::Index(t) => write!(f, "[{t}]")?,
                    }
                }
                Ok(())
            }
            Term::Call { func, args } => {
                let a: Vec<_> = args.iter().map(|x| x.to_string()).collect();
                write!(f, "{func}({})", a.join(", "))
            }
            _ => write!(f, "<expr>"),
        }
    }
}
