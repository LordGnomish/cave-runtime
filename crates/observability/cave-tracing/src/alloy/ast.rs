// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Abstract syntax tree for the Alloy configuration syntax.
//!
//! Line-ported from grafana/alloy `syntax/ast/ast.go` + `walk.go` (v1.5.0,
//! Apache-2.0). The Go AST uses closed interfaces (`Node`/`Stmt`/`Expr`); here
//! those are modelled as Rust enums, which are likewise closed.

use super::scanner::Pos;
use super::token::Token;

/// A parsed file.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    /// Filename provided to the parser.
    pub name: String,
    /// Content of the file.
    pub body: Body,
    /// All comments in the file.
    pub comments: Vec<CommentGroup>,
}

/// A list of statements.
pub type Body = Vec<Stmt>;

/// A sequence of comments not separated by blank lines or other tokens.
pub type CommentGroup = Vec<Comment>;

/// A single line or block comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Starting position of the comment.
    pub start_pos: Pos,
    /// Text of the comment (no `\n` for line comments).
    pub text: String,
}

/// An identifier with its position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    /// Identifier name.
    pub name: String,
    /// Position of the name.
    pub name_pos: Pos,
}

/// A statement within the body of a file or block.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `name = expr`
    Attribute(AttributeStmt),
    /// `name.frag "label" { body }`
    Block(BlockStmt),
}

/// A key-value pair being set in a [`Body`] or [`BlockStmt`].
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeStmt {
    /// Attribute name.
    pub name: Ident,
    /// Assigned value.
    pub value: Expr,
}

/// A block declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStmt {
    /// `.`-delimited name fragments (e.g. `["prometheus", "scrape"]`).
    pub name: Vec<String>,
    /// Position of the first name fragment.
    pub name_pos: Pos,
    /// Optional user label.
    pub label: String,
    /// Position of the label, if present.
    pub label_pos: Option<Pos>,
    /// Block body.
    pub body: Body,
    /// Position of the opening `{`.
    pub lcurly_pos: Pos,
    /// Position of the closing `}`.
    pub rcurly_pos: Pos,
}

impl BlockStmt {
    /// Retrieves the `.`-delimited block name. Mirrors `BlockStmt.GetBlockName`.
    pub fn block_name(&self) -> String {
        self.name.join(".")
    }
}

/// An expression within the AST.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A named value reference.
    Identifier(IdentifierExpr),
    /// A constant literal of a specific token kind.
    Literal(LiteralExpr),
    /// `[ e, e, ... ]`
    Array(ArrayExpr),
    /// `{ k = v, ... }`
    Object(ObjectExpr),
    /// `value.name`
    Access(AccessExpr),
    /// `value[index]`
    Index(IndexExpr),
    /// `value(args...)`
    Call(CallExpr),
    /// `op value`
    Unary(UnaryExpr),
    /// `left op right`
    Binary(BinaryExpr),
    /// `( inner )`
    Paren(ParenExpr),
}

/// Refers to a named value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierExpr {
    /// The referenced identifier.
    pub ident: Ident,
}

/// A constant value of a specific token kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralExpr {
    /// Token kind (`STRING`, `NUMBER`, `FLOAT`, `BOOL`, `NULL`).
    pub kind: Token,
    /// Position of the literal.
    pub value_pos: Pos,
    /// Unparsed literal text (strings keep their surrounding quotes).
    pub value: String,
}

/// An array of values.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayExpr {
    /// Element expressions.
    pub elements: Vec<Expr>,
    /// Position of `[`.
    pub lbrack_pos: Pos,
    /// Position of `]`.
    pub rbrack_pos: Pos,
}

/// An object of key-value pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectExpr {
    /// Object fields.
    pub fields: Vec<ObjectField>,
    /// Position of `{`.
    pub lcurly_pos: Pos,
    /// Position of `}`.
    pub rcurly_pos: Pos,
}

/// An individual key-value pair within an object.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectField {
    /// Field name.
    pub name: Ident,
    /// True if the name was wrapped in quotes.
    pub quoted: bool,
    /// Field value.
    pub value: Expr,
}

/// Accesses a field in an object value by name.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessExpr {
    /// Accessed value.
    pub value: Box<Expr>,
    /// Field name.
    pub name: Ident,
}

/// Accesses an index in an array value.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpr {
    /// Indexed value.
    pub value: Box<Expr>,
    /// Index expression.
    pub index: Box<Expr>,
    /// Position of `[`.
    pub lbrack_pos: Pos,
    /// Position of `]`.
    pub rbrack_pos: Pos,
}

/// Invokes a function value with a set of arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct CallExpr {
    /// Callee value.
    pub value: Box<Expr>,
    /// Argument expressions.
    pub args: Vec<Expr>,
    /// Position of `(`.
    pub lparen_pos: Pos,
    /// Position of `)`.
    pub rparen_pos: Pos,
}

/// A unary operation on a single value.
#[derive(Debug, Clone, PartialEq)]
pub struct UnaryExpr {
    /// Operator token (`!`, `-`).
    pub kind: Token,
    /// Position of the operator.
    pub kind_pos: Pos,
    /// Operand.
    pub value: Box<Expr>,
}

/// A binary operation against two values.
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpr {
    /// Left operand.
    pub left: Box<Expr>,
    /// Operator token.
    pub kind: Token,
    /// Position of the operator.
    pub kind_pos: Pos,
    /// Right operand.
    pub right: Box<Expr>,
}

/// An expression wrapped in parentheses.
#[derive(Debug, Clone, PartialEq)]
pub struct ParenExpr {
    /// Inner expression.
    pub inner: Box<Expr>,
    /// Position of `(`.
    pub lparen_pos: Pos,
    /// Position of `)`.
    pub rparen_pos: Pos,
}

impl Ident {
    fn start_pos(&self) -> Pos {
        self.name_pos
    }
    fn end_pos(&self) -> Pos {
        self.name_pos.advance(self.name.len().saturating_sub(1))
    }
}

impl Expr {
    /// Position of the first character of the expression. Mirrors
    /// `ast.StartPos`.
    pub fn start_pos(&self) -> Pos {
        match self {
            Expr::Identifier(e) => e.ident.start_pos(),
            Expr::Literal(e) => e.value_pos,
            Expr::Array(e) => e.lbrack_pos,
            Expr::Object(e) => e.lcurly_pos,
            Expr::Access(e) => e.value.start_pos(),
            Expr::Index(e) => e.value.start_pos(),
            Expr::Call(e) => e.value.start_pos(),
            Expr::Unary(e) => e.kind_pos,
            Expr::Binary(e) => e.left.start_pos(),
            Expr::Paren(e) => e.lparen_pos,
        }
    }

    /// Position of the last character of the expression. Mirrors `ast.EndPos`.
    pub fn end_pos(&self) -> Pos {
        match self {
            Expr::Identifier(e) => e.ident.end_pos(),
            Expr::Literal(e) => e.value_pos.advance(e.value.len().saturating_sub(1)),
            Expr::Array(e) => e.rbrack_pos,
            Expr::Object(e) => e.rcurly_pos,
            Expr::Access(e) => e.name.end_pos(),
            Expr::Index(e) => e.rbrack_pos,
            Expr::Call(e) => e.rparen_pos,
            Expr::Unary(e) => e.value.end_pos(),
            Expr::Binary(e) => e.right.end_pos(),
            Expr::Paren(e) => e.rparen_pos,
        }
    }
}

impl Stmt {
    /// Position of the first character of the statement.
    pub fn start_pos(&self) -> Pos {
        match self {
            Stmt::Attribute(a) => a.name.start_pos(),
            Stmt::Block(b) => b.name_pos,
        }
    }

    /// Position of the last character of the statement.
    pub fn end_pos(&self) -> Pos {
        match self {
            Stmt::Attribute(a) => a.value.end_pos(),
            Stmt::Block(b) => b.rcurly_pos,
        }
    }
}

/// A borrowed reference to any AST node, used by [`walk`].
#[derive(Debug)]
pub enum Node<'a> {
    /// A file.
    File(&'a File),
    /// A statement.
    Stmt(&'a Stmt),
    /// An expression.
    Expr(&'a Expr),
    /// An identifier.
    Ident(&'a Ident),
}

/// Traverses an AST in depth-first order, invoking `visit` for each node
/// (including the root). Mirrors `ast.Walk`; the case order matches the
/// declared node order in `ast.go`.
pub fn walk(node: &Node, visit: &mut dyn FnMut(&Node)) {
    visit(node);
    match node {
        Node::File(f) => {
            for s in &f.body {
                walk(&Node::Stmt(s), visit);
            }
        }
        Node::Stmt(Stmt::Attribute(a)) => {
            walk(&Node::Ident(&a.name), visit);
            walk(&Node::Expr(&a.value), visit);
        }
        Node::Stmt(Stmt::Block(b)) => {
            for s in &b.body {
                walk(&Node::Stmt(s), visit);
            }
        }
        Node::Ident(_) => {}
        Node::Expr(e) => walk_expr(e, visit),
    }
}

fn walk_expr(e: &Expr, visit: &mut dyn FnMut(&Node)) {
    match e {
        Expr::Identifier(x) => walk(&Node::Ident(&x.ident), visit),
        Expr::Literal(_) => {}
        Expr::Array(x) => {
            for el in &x.elements {
                walk(&Node::Expr(el), visit);
            }
        }
        Expr::Object(x) => {
            for f in &x.fields {
                walk(&Node::Ident(&f.name), visit);
                walk(&Node::Expr(&f.value), visit);
            }
        }
        Expr::Access(x) => {
            walk(&Node::Expr(&x.value), visit);
            walk(&Node::Ident(&x.name), visit);
        }
        Expr::Index(x) => {
            walk(&Node::Expr(&x.value), visit);
            walk(&Node::Expr(&x.index), visit);
        }
        Expr::Call(x) => {
            walk(&Node::Expr(&x.value), visit);
            for a in &x.args {
                walk(&Node::Expr(a), visit);
            }
        }
        Expr::Unary(x) => walk(&Node::Expr(&x.value), visit),
        Expr::Binary(x) => {
            walk(&Node::Expr(&x.left), visit);
            walk(&Node::Expr(&x.right), visit);
        }
        Expr::Paren(x) => walk(&Node::Expr(&x.inner), visit),
    }
}
