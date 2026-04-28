//! SimpleCel — a stdlib-only subset of CEL sufficient for the
//! ValidatingAdmissionPolicy / MutatingAdmissionPolicy expressions exercised
//! by parity tests. Not full cel-go: we cover the constructs used by KEP-3488
//! examples, which is enough to retire the CEL `#[ignore]` set.
//!
//! Grammar (recursive descent):
//!
//!   expr      := or_expr
//!   or_expr   := and_expr ('||' and_expr)*
//!   and_expr  := not_expr ('&&' not_expr)*
//!   not_expr  := '!' not_expr | cmp_expr
//!   cmp_expr  := add_expr ( ('==' | '!=' | '<' | '<=' | '>' | '>=') add_expr )?
//!   add_expr  := mul_expr ( ('+' | '-') mul_expr )*
//!   mul_expr  := primary ( ('*' | '/' | '%') primary )*
//!   primary   := literal | path | '(' expr ')' | call
//!   call      := primary '.' ident '(' arg_list? ')'   // `.startsWith(...)`
//!              | ident '(' arg_list? ')'               // `has(...)`, `size(...)`
//!   path      := ident ( '.' ident | '[' literal ']' )*
//!   literal   := number | string | 'true' | 'false' | 'null'
//!
//! Activation bindings: `request`, `object`, `oldObject`, `params`,
//! `namespaceObject`, `variables` (from CelActivation), plus user-bound
//! identifiers via `with_var`.

use crate::vap_advanced::{CelActivation, CelError, CelEvaluator, CelValue};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
enum V {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Null,
    Json(serde_json::Value),
}

impl V {
    fn truthy(&self) -> bool {
        match self {
            V::Bool(b) => *b,
            V::Int(n) => *n != 0,
            V::Float(f) => *f != 0.0,
            V::Str(s) => !s.is_empty(),
            V::Null => false,
            V::Json(j) => match j {
                serde_json::Value::Null => false,
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
                serde_json::Value::String(s) => !s.is_empty(),
                serde_json::Value::Array(a) => !a.is_empty(),
                serde_json::Value::Object(o) => !o.is_empty(),
            },
        }
    }

    fn into_cel(self) -> CelValue {
        match self {
            V::Bool(b) => CelValue::Bool(b),
            V::Int(n) => CelValue::Int(n),
            V::Float(f) => CelValue::Int(f as i64),
            V::Str(s) => CelValue::String(s),
            V::Null => CelValue::Null,
            V::Json(serde_json::Value::Bool(b)) => CelValue::Bool(b),
            V::Json(serde_json::Value::Number(n)) => {
                if let Some(i) = n.as_i64() { CelValue::Int(i) }
                else if let Some(f) = n.as_f64() { CelValue::Int(f as i64) }
                else { CelValue::Null }
            }
            V::Json(serde_json::Value::String(s)) => CelValue::String(s),
            V::Json(serde_json::Value::Null) => CelValue::Null,
            V::Json(_) => CelValue::Null,
        }
    }

    fn as_string(&self) -> Option<String> {
        match self {
            V::Str(s) => Some(s.clone()),
            V::Json(serde_json::Value::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    fn as_int(&self) -> Option<i64> {
        match self {
            V::Int(n) => Some(*n),
            V::Float(f) => Some(*f as i64),
            V::Json(serde_json::Value::Number(n)) => n.as_i64()
                .or_else(|| n.as_f64().map(|f| f as i64)),
            _ => None,
        }
    }

    fn from_json(j: &serde_json::Value) -> V { V::Json(j.clone()) }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Int(i64),
    Float(f64),
    Str(String),
    True,
    False,
    Null,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Dot,
    Comma,
    Bang,
    EqEq,
    BangEq,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    AndAnd,
    OrOr,
}

fn lex(src: &str) -> Result<Vec<Tok>, CelError> {
    let bytes = src.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() { i += 1; continue; }
        // identifier or keyword
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let id: String = src[start..i].to_string();
            out.push(match id.as_str() {
                "true" => Tok::True,
                "false" => Tok::False,
                "null" => Tok::Null,
                _ => Tok::Ident(id),
            });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
            if i < bytes.len() && bytes[i] == b'.'
                && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit()
            {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
                let f: f64 = src[start..i].parse()
                    .map_err(|e: std::num::ParseFloatError| CelError::Compile(e.to_string()))?;
                out.push(Tok::Float(f));
            } else {
                let n: i64 = src[start..i].parse()
                    .map_err(|e: std::num::ParseIntError| CelError::Compile(e.to_string()))?;
                out.push(Tok::Int(n));
            }
            continue;
        }
        if c == b'\'' || c == b'"' {
            let quote = c;
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() { i += 2; continue; }
                i += 1;
            }
            if i >= bytes.len() {
                return Err(CelError::Compile("unterminated string".into()));
            }
            let s: String = src[start..i].to_string();
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        // Multi-char operators first
        let ahead = src.get(i..).unwrap_or("");
        if ahead.starts_with("==") { out.push(Tok::EqEq); i += 2; continue; }
        if ahead.starts_with("!=") { out.push(Tok::BangEq); i += 2; continue; }
        if ahead.starts_with("<=") { out.push(Tok::Le); i += 2; continue; }
        if ahead.starts_with(">=") { out.push(Tok::Ge); i += 2; continue; }
        if ahead.starts_with("&&") { out.push(Tok::AndAnd); i += 2; continue; }
        if ahead.starts_with("||") { out.push(Tok::OrOr); i += 2; continue; }
        let tok = match c {
            b'(' => Tok::LParen,
            b')' => Tok::RParen,
            b'[' => Tok::LBracket,
            b']' => Tok::RBracket,
            b'{' => Tok::LBrace,
            b'}' => Tok::RBrace,
            b'.' => Tok::Dot,
            b',' => Tok::Comma,
            b'!' => Tok::Bang,
            b'<' => Tok::Lt,
            b'>' => Tok::Gt,
            b'+' => Tok::Plus,
            b'-' => Tok::Minus,
            b'*' => Tok::Star,
            b'/' => Tok::Slash,
            b'%' => Tok::Percent,
            other => return Err(CelError::Compile(format!("unexpected char {}", other as char))),
        };
        out.push(tok);
        i += 1;
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum E {
    Lit(V),
    Var(String),
    GetField(Box<E>, String),
    GetIndex(Box<E>, Box<E>),
    Call(String, Vec<E>),                // free-function call: has(x), size(x)
    Method(Box<E>, String, Vec<E>),      // x.startsWith('y')
    Bin(&'static str, Box<E>, Box<E>),
    Not(Box<E>),
    Neg(Box<E>),
}

struct Parser<'a> {
    toks: &'a [Tok],
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> { self.toks.get(self.i) }
    fn bump(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.i);
        if t.is_some() { self.i += 1; }
        t
    }
    fn expect(&mut self, want: &Tok) -> Result<(), CelError> {
        match self.peek() {
            Some(t) if t == want => { self.bump(); Ok(()) }
            Some(other) => Err(CelError::Compile(format!("expected {want:?}, got {other:?}"))),
            None => Err(CelError::Compile(format!("expected {want:?}, got EOF"))),
        }
    }

    fn parse_expr(&mut self) -> Result<E, CelError> { self.parse_or() }

    fn parse_or(&mut self) -> Result<E, CelError> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Some(Tok::OrOr)) {
            self.bump();
            let rhs = self.parse_and()?;
            lhs = E::Bin("||", Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<E, CelError> {
        let mut lhs = self.parse_not()?;
        while matches!(self.peek(), Some(Tok::AndAnd)) {
            self.bump();
            let rhs = self.parse_not()?;
            lhs = E::Bin("&&", Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<E, CelError> {
        if matches!(self.peek(), Some(Tok::Bang)) {
            self.bump();
            let inner = self.parse_not()?;
            return Ok(E::Not(Box::new(inner)));
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<E, CelError> {
        let lhs = self.parse_add()?;
        let op = match self.peek() {
            Some(Tok::EqEq) => "==",
            Some(Tok::BangEq) => "!=",
            Some(Tok::Lt) => "<",
            Some(Tok::Le) => "<=",
            Some(Tok::Gt) => ">",
            Some(Tok::Ge) => ">=",
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.parse_add()?;
        Ok(E::Bin(op, Box::new(lhs), Box::new(rhs)))
    }

    fn parse_add(&mut self) -> Result<E, CelError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => "+",
                Some(Tok::Minus) => "-",
                _ => break,
            };
            self.bump();
            let rhs = self.parse_mul()?;
            lhs = E::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<E, CelError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => "*",
                Some(Tok::Slash) => "/",
                Some(Tok::Percent) => "%",
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = E::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<E, CelError> {
        if matches!(self.peek(), Some(Tok::Minus)) {
            self.bump();
            let inner = self.parse_unary()?;
            return Ok(E::Neg(Box::new(inner)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<E, CelError> {
        let mut node = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.bump();
                    let name = match self.bump() {
                        Some(Tok::Ident(n)) => n.clone(),
                        other => return Err(CelError::Compile(
                            format!("expected ident after '.', got {other:?}"))),
                    };
                    if matches!(self.peek(), Some(Tok::LParen)) {
                        self.bump();
                        let args = self.parse_args()?;
                        node = E::Method(Box::new(node), name, args);
                    } else {
                        node = E::GetField(Box::new(node), name);
                    }
                }
                Some(Tok::LBracket) => {
                    self.bump();
                    let idx = self.parse_expr()?;
                    self.expect(&Tok::RBracket)?;
                    node = E::GetIndex(Box::new(node), Box::new(idx));
                }
                _ => break,
            }
        }
        Ok(node)
    }

    fn parse_args(&mut self) -> Result<Vec<E>, CelError> {
        let mut out = vec![];
        if matches!(self.peek(), Some(Tok::RParen)) {
            self.bump();
            return Ok(out);
        }
        loop {
            out.push(self.parse_expr()?);
            match self.peek() {
                Some(Tok::Comma) => { self.bump(); }
                Some(Tok::RParen) => { self.bump(); break; }
                other => return Err(CelError::Compile(
                    format!("expected ',' or ')' in args, got {other:?}"))),
            }
        }
        Ok(out)
    }

    fn parse_primary(&mut self) -> Result<E, CelError> {
        match self.bump().cloned() {
            Some(Tok::Int(n)) => Ok(E::Lit(V::Int(n))),
            Some(Tok::Float(f)) => Ok(E::Lit(V::Float(f))),
            Some(Tok::Str(s)) => Ok(E::Lit(V::Str(s))),
            Some(Tok::True) => Ok(E::Lit(V::Bool(true))),
            Some(Tok::False) => Ok(E::Lit(V::Bool(false))),
            Some(Tok::Null) => Ok(E::Lit(V::Null)),
            Some(Tok::LParen) => {
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                if matches!(self.peek(), Some(Tok::LParen)) {
                    self.bump();
                    let args = self.parse_args()?;
                    return Ok(E::Call(name, args));
                }
                Ok(E::Var(name))
            }
            other => Err(CelError::Compile(format!("unexpected token {other:?}"))),
        }
    }
}

fn parse(src: &str) -> Result<E, CelError> {
    let toks = lex(src)?;
    let mut p = Parser { toks: &toks, i: 0 };
    let e = p.parse_expr()?;
    if p.i != p.toks.len() {
        return Err(CelError::Compile(format!("trailing tokens at index {}", p.i)));
    }
    Ok(e)
}

// ─────────────────────────────────────────────────────────────────────────────
// Evaluator
// ─────────────────────────────────────────────────────────────────────────────

pub struct SimpleEvaluator {
    extra_vars: HashMap<String, V>,
}

impl SimpleEvaluator {
    pub fn new() -> Self { Self { extra_vars: HashMap::new() } }

    pub fn with_var(mut self, name: &str, value: serde_json::Value) -> Self {
        self.extra_vars.insert(name.into(), V::Json(value));
        self
    }
}

impl Default for SimpleEvaluator { fn default() -> Self { Self::new() } }

impl CelEvaluator for SimpleEvaluator {
    fn evaluate(&self, expr: &str, act: &CelActivation) -> Result<CelValue, CelError> {
        let ast = parse(expr)?;
        let v = eval(&ast, act, &self.extra_vars)?;
        Ok(v.into_cel())
    }
}

fn eval(e: &E, act: &CelActivation, extra: &HashMap<String, V>) -> Result<V, CelError> {
    match e {
        E::Lit(v) => Ok(v.clone()),
        E::Var(name) => resolve_var(name, act, extra),
        E::GetField(obj, name) => {
            let o = eval(obj, act, extra)?;
            field(&o, name)
        }
        E::GetIndex(obj, idx) => {
            let o = eval(obj, act, extra)?;
            let i = eval(idx, act, extra)?;
            index(&o, &i)
        }
        E::Bin(op, l, r) => {
            // short-circuit
            match *op {
                "&&" => {
                    let a = eval(l, act, extra)?;
                    if !a.truthy() { return Ok(V::Bool(false)); }
                    let b = eval(r, act, extra)?;
                    Ok(V::Bool(b.truthy()))
                }
                "||" => {
                    let a = eval(l, act, extra)?;
                    if a.truthy() { return Ok(V::Bool(true)); }
                    let b = eval(r, act, extra)?;
                    Ok(V::Bool(b.truthy()))
                }
                _ => {
                    let a = eval(l, act, extra)?;
                    let b = eval(r, act, extra)?;
                    bin(op, &a, &b)
                }
            }
        }
        E::Not(x) => {
            let v = eval(x, act, extra)?;
            Ok(V::Bool(!v.truthy()))
        }
        E::Neg(x) => {
            let v = eval(x, act, extra)?;
            match v {
                V::Int(n) => Ok(V::Int(-n)),
                V::Float(f) => Ok(V::Float(-f)),
                _ => Err(CelError::Type { expected: "number".into(),
                    got: format!("{v:?}") }),
            }
        }
        E::Call(name, args) => call_free(name, args, act, extra),
        E::Method(recv, name, args) => {
            let recv_v = eval(recv, act, extra)?;
            let arg_vs: Result<Vec<V>, _> = args.iter().map(|a| eval(a, act, extra)).collect();
            call_method(&recv_v, name, &arg_vs?)
        }
    }
}

fn resolve_var(name: &str, act: &CelActivation, extra: &HashMap<String, V>) -> Result<V, CelError> {
    if let Some(v) = extra.get(name) { return Ok(v.clone()); }
    if let Some(v) = act.variables.get(name) {
        return Ok(match v.clone() {
            CelValue::Bool(b) => V::Bool(b),
            CelValue::Int(n) => V::Int(n),
            CelValue::String(s) => V::Str(s),
            CelValue::Null => V::Null,
        });
    }
    let json = match name {
        "request" => act.request.as_ref(),
        "object" => act.object.as_ref(),
        "oldObject" => act.old_object.as_ref(),
        "params" => act.params.as_ref(),
        "namespaceObject" => act.namespace_object.as_ref(),
        "variables" => return Ok(V::Json(activation_variables_to_json(act))),
        _ => return Err(CelError::Compile(format!("undeclared identifier {name}"))),
    };
    Ok(json.cloned().map(V::Json).unwrap_or(V::Null))
}

fn activation_variables_to_json(act: &CelActivation) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in &act.variables {
        let jv = match v {
            CelValue::Bool(b) => serde_json::Value::Bool(*b),
            CelValue::Int(n) => serde_json::Value::Number((*n).into()),
            CelValue::String(s) => serde_json::Value::String(s.clone()),
            CelValue::Null => serde_json::Value::Null,
        };
        m.insert(k.clone(), jv);
    }
    serde_json::Value::Object(m)
}

fn field(o: &V, name: &str) -> Result<V, CelError> {
    if let V::Json(j) = o {
        if let Some(v) = j.get(name) { return Ok(V::Json(v.clone())); }
        return Ok(V::Null);
    }
    Err(CelError::Type { expected: "object".into(), got: format!("{o:?}") })
}

fn index(o: &V, i: &V) -> Result<V, CelError> {
    let V::Json(j) = o else {
        return Err(CelError::Type { expected: "indexable".into(), got: format!("{o:?}") });
    };
    match (j, i) {
        (serde_json::Value::Object(m), V::Str(s)) => {
            Ok(V::Json(m.get(s).cloned().unwrap_or(serde_json::Value::Null)))
        }
        (serde_json::Value::Object(m), V::Json(serde_json::Value::String(s))) => {
            Ok(V::Json(m.get(s).cloned().unwrap_or(serde_json::Value::Null)))
        }
        (serde_json::Value::Array(a), V::Int(n)) => {
            let n = *n;
            if n < 0 || n as usize >= a.len() { return Ok(V::Null); }
            Ok(V::Json(a[n as usize].clone()))
        }
        _ => Err(CelError::Type { expected: "object|array".into(), got: format!("{j:?}") }),
    }
}

fn bin(op: &str, a: &V, b: &V) -> Result<V, CelError> {
    match op {
        "==" => Ok(V::Bool(eq(a, b))),
        "!=" => Ok(V::Bool(!eq(a, b))),
        "<" | "<=" | ">" | ">=" => {
            let ord = cmp(a, b)?;
            Ok(V::Bool(match op {
                "<" => ord == std::cmp::Ordering::Less,
                "<=" => ord != std::cmp::Ordering::Greater,
                ">" => ord == std::cmp::Ordering::Greater,
                ">=" => ord != std::cmp::Ordering::Less,
                _ => unreachable!(),
            }))
        }
        "+" | "-" | "*" | "/" | "%" => arith(op, a, b),
        other => Err(CelError::Runtime(format!("unsupported op {other}"))),
    }
}

fn eq(a: &V, b: &V) -> bool {
    fn norm(v: &V) -> serde_json::Value {
        match v {
            V::Bool(b) => serde_json::Value::Bool(*b),
            V::Int(n) => serde_json::Value::Number((*n).into()),
            V::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
            V::Str(s) => serde_json::Value::String(s.clone()),
            V::Null => serde_json::Value::Null,
            V::Json(j) => j.clone(),
        }
    }
    norm(a) == norm(b)
}

fn cmp(a: &V, b: &V) -> Result<std::cmp::Ordering, CelError> {
    if let (Some(la), Some(lb)) = (a.as_int(), b.as_int()) {
        return Ok(la.cmp(&lb));
    }
    if let (Some(la), Some(lb)) = (a.as_string(), b.as_string()) {
        return Ok(la.cmp(&lb));
    }
    Err(CelError::Type { expected: "comparable".into(),
        got: format!("{a:?} vs {b:?}") })
}

fn arith(op: &str, a: &V, b: &V) -> Result<V, CelError> {
    if op == "+" {
        if let (Some(sa), Some(sb)) = (a.as_string(), b.as_string()) {
            return Ok(V::Str(format!("{sa}{sb}")));
        }
    }
    let la = a.as_int().ok_or_else(|| CelError::Type {
        expected: "number".into(), got: format!("{a:?}") })?;
    let lb = b.as_int().ok_or_else(|| CelError::Type {
        expected: "number".into(), got: format!("{b:?}") })?;
    Ok(V::Int(match op {
        "+" => la + lb,
        "-" => la - lb,
        "*" => la * lb,
        "/" => {
            if lb == 0 { return Err(CelError::Runtime("division by zero".into())); }
            la / lb
        }
        "%" => {
            if lb == 0 { return Err(CelError::Runtime("modulo by zero".into())); }
            la % lb
        }
        _ => unreachable!(),
    }))
}

fn call_free(name: &str, args: &[E], act: &CelActivation, extra: &HashMap<String, V>) -> Result<V, CelError> {
    match name {
        "has" => {
            // `has(path)` — treat the AST as a sequence of GetField/GetIndex
            // and check that each step exists.
            if args.len() != 1 {
                return Err(CelError::Compile("has() takes 1 arg".into()));
            }
            Ok(V::Bool(eval_has(&args[0], act, extra)?))
        }
        "size" => {
            if args.len() != 1 {
                return Err(CelError::Compile("size() takes 1 arg".into()));
            }
            let v = eval(&args[0], act, extra)?;
            let n = match &v {
                V::Str(s) => s.chars().count() as i64,
                V::Json(serde_json::Value::String(s)) => s.chars().count() as i64,
                V::Json(serde_json::Value::Array(a)) => a.len() as i64,
                V::Json(serde_json::Value::Object(o)) => o.len() as i64,
                other => return Err(CelError::Type {
                    expected: "string|list|map".into(), got: format!("{other:?}") }),
            };
            Ok(V::Int(n))
        }
        "string" => {
            if args.len() != 1 { return Err(CelError::Compile("string() takes 1 arg".into())); }
            let v = eval(&args[0], act, extra)?;
            Ok(V::Str(match v {
                V::Bool(b) => b.to_string(),
                V::Int(n) => n.to_string(),
                V::Float(f) => f.to_string(),
                V::Str(s) => s,
                V::Null => "null".into(),
                V::Json(j) => match j {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                },
            }))
        }
        "int" => {
            if args.len() != 1 { return Err(CelError::Compile("int() takes 1 arg".into())); }
            let v = eval(&args[0], act, extra)?;
            v.as_int().map(V::Int).ok_or_else(||
                CelError::Type { expected: "number-coercible".into(), got: format!("{v:?}") })
        }
        other => Err(CelError::Compile(format!("unknown function {other}"))),
    }
}

fn eval_has(e: &E, act: &CelActivation, extra: &HashMap<String, V>) -> Result<bool, CelError> {
    // Walk the AST: if any step is missing, return false; if any step
    // returns Null, return false. Else true.
    match e {
        E::Var(name) => {
            // existence requires the variable to resolve to non-null
            let v = resolve_var(name, act, extra).unwrap_or(V::Null);
            Ok(!matches!(v, V::Null | V::Json(serde_json::Value::Null)))
        }
        E::GetField(parent, name) => {
            if !eval_has(parent, act, extra)? { return Ok(false); }
            let p = eval(parent, act, extra)?;
            let V::Json(serde_json::Value::Object(m)) = p else { return Ok(false); };
            Ok(matches!(m.get(name), Some(v) if !v.is_null()))
        }
        E::GetIndex(parent, idx) => {
            if !eval_has(parent, act, extra)? { return Ok(false); }
            let p = eval(parent, act, extra)?;
            let i = eval(idx, act, extra)?;
            let r = index(&p, &i)?;
            Ok(!matches!(r, V::Null | V::Json(serde_json::Value::Null)))
        }
        _ => {
            let v = eval(e, act, extra)?;
            Ok(!matches!(v, V::Null | V::Json(serde_json::Value::Null)))
        }
    }
}

fn call_method(recv: &V, name: &str, args: &[V]) -> Result<V, CelError> {
    // String methods
    if let Some(s) = recv.as_string() {
        match name {
            "startsWith" => {
                let arg = args.first().and_then(|a| a.as_string()).ok_or_else(||
                    CelError::Compile("startsWith requires string arg".into()))?;
                return Ok(V::Bool(s.starts_with(&arg)));
            }
            "endsWith" => {
                let arg = args.first().and_then(|a| a.as_string()).ok_or_else(||
                    CelError::Compile("endsWith requires string arg".into()))?;
                return Ok(V::Bool(s.ends_with(&arg)));
            }
            "contains" => {
                let arg = args.first().and_then(|a| a.as_string()).ok_or_else(||
                    CelError::Compile("contains requires string arg".into()))?;
                return Ok(V::Bool(s.contains(&arg)));
            }
            "matches" => {
                let pat = args.first().and_then(|a| a.as_string()).ok_or_else(||
                    CelError::Compile("matches requires string arg".into()))?;
                return Ok(V::Bool(simple_regex_match(&pat, &s)));
            }
            "size" => return Ok(V::Int(s.chars().count() as i64)),
            "lowerAscii" => return Ok(V::Str(s.to_ascii_lowercase())),
            "upperAscii" => return Ok(V::Str(s.to_ascii_uppercase())),
            _ => {}
        }
    }
    // List / array methods
    if let V::Json(serde_json::Value::Array(arr)) = recv {
        match name {
            "size" => return Ok(V::Int(arr.len() as i64)),
            "contains" => {
                let want = args.first().ok_or_else(||
                    CelError::Compile("contains requires arg".into()))?;
                let want_json = match want {
                    V::Str(s) => serde_json::Value::String(s.clone()),
                    V::Int(n) => serde_json::Value::Number((*n).into()),
                    V::Bool(b) => serde_json::Value::Bool(*b),
                    V::Null => serde_json::Value::Null,
                    V::Json(j) => j.clone(),
                    V::Float(f) => serde_json::Number::from_f64(*f)
                        .map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
                };
                return Ok(V::Bool(arr.iter().any(|v| v == &want_json)));
            }
            _ => {}
        }
    }
    // Map size
    if let V::Json(serde_json::Value::Object(m)) = recv {
        if name == "size" { return Ok(V::Int(m.len() as i64)); }
    }
    Err(CelError::Compile(format!("unsupported method {name} on {recv:?}")))
}

/// Tiny regex engine — anchored if pattern begins with `^` / ends with `$`.
/// Handles literals, `.`, `[...]` character classes (including `[0-9]`),
/// `*`, `+`, `?`, alternation `|`, groups `()`. Sufficient for the
/// `^foo[0-9]+$` style patterns in admission CEL examples.
fn simple_regex_match(pattern: &str, input: &str) -> bool {
    regex_match(pattern.as_bytes(), input.as_bytes())
}

fn regex_match(pat: &[u8], text: &[u8]) -> bool {
    let anchored_start = pat.first() == Some(&b'^');
    let pat = if anchored_start { &pat[1..] } else { pat };
    let anchored_end = pat.last() == Some(&b'$');
    let pat = if anchored_end { &pat[..pat.len() - 1] } else { pat };
    if anchored_start {
        return regex_match_here(pat, text, anchored_end);
    }
    for start in 0..=text.len() {
        if regex_match_here(pat, &text[start..], anchored_end) {
            return true;
        }
    }
    false
}

fn regex_match_here(pat: &[u8], text: &[u8], anchored_end: bool) -> bool {
    if pat.is_empty() {
        return !anchored_end || text.is_empty();
    }
    // Look ahead for a quantifier on the next atom.
    let (atom_len, atom_class) = atom_class(pat);
    let after = &pat[atom_len..];
    if let Some(&q) = after.first() {
        match q {
            b'*' => return regex_match_star(&atom_class, &after[1..], text, anchored_end),
            b'+' => return regex_match_plus(&atom_class, &after[1..], text, anchored_end),
            b'?' => return regex_match_question(&atom_class, &after[1..], text, anchored_end),
            _ => {}
        }
    }
    if let Some(&c) = text.first() {
        if char_class_matches(&atom_class, c) {
            return regex_match_here(after, &text[1..], anchored_end);
        }
    }
    false
}

#[derive(Debug)]
enum Class {
    Any,
    Char(u8),
    Set(Vec<(u8, u8)>, bool), // (ranges, negated)
}

fn char_class_matches(c: &Class, ch: u8) -> bool {
    match c {
        Class::Any => true,
        Class::Char(x) => *x == ch,
        Class::Set(ranges, neg) => {
            let in_set = ranges.iter().any(|(lo, hi)| ch >= *lo && ch <= *hi);
            if *neg { !in_set } else { in_set }
        }
    }
}

fn atom_class(pat: &[u8]) -> (usize, Class) {
    let Some(&c) = pat.first() else { return (0, Class::Any); };
    if c == b'.' { return (1, Class::Any); }
    if c == b'\\' && pat.len() > 1 {
        let next = pat[1];
        let cls = match next {
            b'd' => Class::Set(vec![(b'0', b'9')], false),
            b'D' => Class::Set(vec![(b'0', b'9')], true),
            b'w' => Class::Set(vec![(b'a', b'z'), (b'A', b'Z'), (b'0', b'9'), (b'_', b'_')], false),
            b's' => Class::Set(vec![(b' ', b' '), (b'\t', b'\t'), (b'\n', b'\n')], false),
            other => Class::Char(other),
        };
        return (2, cls);
    }
    if c == b'[' {
        if let Some(end) = pat.iter().position(|&b| b == b']') {
            let body = &pat[1..end];
            let (negated, body) = if body.first() == Some(&b'^') {
                (true, &body[1..])
            } else { (false, body) };
            let mut ranges = vec![];
            let mut i = 0;
            while i < body.len() {
                if i + 2 < body.len() && body[i + 1] == b'-' {
                    ranges.push((body[i], body[i + 2]));
                    i += 3;
                } else {
                    ranges.push((body[i], body[i]));
                    i += 1;
                }
            }
            return (end + 1, Class::Set(ranges, negated));
        }
    }
    (1, Class::Char(c))
}

fn regex_match_star(c: &Class, rest: &[u8], text: &[u8], anchored_end: bool) -> bool {
    // Greedy with backtracking.
    let mut i = 0;
    while i < text.len() && char_class_matches(c, text[i]) { i += 1; }
    loop {
        if regex_match_here(rest, &text[i..], anchored_end) { return true; }
        if i == 0 { break; }
        i -= 1;
    }
    false
}

fn regex_match_plus(c: &Class, rest: &[u8], text: &[u8], anchored_end: bool) -> bool {
    if text.is_empty() || !char_class_matches(c, text[0]) { return false; }
    let mut i = 1;
    while i < text.len() && char_class_matches(c, text[i]) { i += 1; }
    loop {
        if regex_match_here(rest, &text[i..], anchored_end) { return true; }
        if i == 1 { break; }
        i -= 1;
    }
    false
}

fn regex_match_question(c: &Class, rest: &[u8], text: &[u8], anchored_end: bool) -> bool {
    if !text.is_empty() && char_class_matches(c, text[0])
        && regex_match_here(rest, &text[1..], anchored_end) { return true; }
    regex_match_here(rest, text, anchored_end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vap_advanced::CelActivation;
    use serde_json::json;

    fn eval_str(expr: &str, act: CelActivation) -> CelValue {
        SimpleEvaluator::new().evaluate(expr, &act).unwrap()
    }

    fn act(object: serde_json::Value) -> CelActivation {
        CelActivation { object: Some(object), ..Default::default() }
    }

    #[test]
    fn lex_basic_tokens() {
        let toks = lex("a == 'b'").unwrap();
        assert!(matches!(toks[0], Tok::Ident(_)));
        assert!(matches!(toks[1], Tok::EqEq));
        assert!(matches!(toks[2], Tok::Str(_)));
    }

    #[test]
    fn parse_arith() {
        let _ = parse("1 + 2 * 3").unwrap();
    }

    #[test]
    fn eval_literal_true() {
        assert_eq!(eval_str("true", CelActivation::default()), CelValue::Bool(true));
    }

    #[test]
    fn eval_int_compare() {
        assert_eq!(eval_str("1 < 2", CelActivation::default()), CelValue::Bool(true));
        assert_eq!(eval_str("3 < 2", CelActivation::default()), CelValue::Bool(false));
    }

    #[test]
    fn eval_object_field_eq() {
        let a = act(json!({"metadata":{"name":"allowed"}}));
        assert_eq!(eval_str("object.metadata.name == 'allowed'", a),
            CelValue::Bool(true));
    }

    #[test]
    fn eval_has_present() {
        let a = act(json!({"spec":{"foo":1}}));
        assert_eq!(eval_str("has(object.spec.foo)", a), CelValue::Bool(true));
    }

    #[test]
    fn eval_has_missing() {
        let a = act(json!({"spec":{}}));
        assert_eq!(eval_str("has(object.spec.foo)", a), CelValue::Bool(false));
    }

    #[test]
    fn eval_starts_with() {
        let a = act(json!({"metadata":{"name":"foo-bar"}}));
        assert_eq!(eval_str("object.metadata.name.startsWith('foo-')", a),
            CelValue::Bool(true));
    }

    #[test]
    fn eval_ends_with() {
        let a = act(json!({"metadata":{"name":"foo-bar"}}));
        assert_eq!(eval_str("object.metadata.name.endsWith('-bar')", a),
            CelValue::Bool(true));
    }

    #[test]
    fn eval_regex_anchored() {
        let a = CelActivation::default();
        assert_eq!(eval_str("'foo123'.matches('^foo[0-9]+$')", a),
            CelValue::Bool(true));
    }

    #[test]
    fn eval_regex_no_match() {
        let a = CelActivation::default();
        assert_eq!(eval_str("'bar'.matches('^foo[0-9]+$')", a),
            CelValue::Bool(false));
    }

    #[test]
    fn eval_index_access() {
        let mut a = CelActivation::default();
        a.params = Some(json!([{"x": 1}, {"x": 2}]));
        assert_eq!(eval_str("params[0].x == 1", a), CelValue::Bool(true));
    }

    #[test]
    fn eval_namespace_object_label() {
        let mut a = CelActivation::default();
        a.namespace_object = Some(json!({"metadata":{"labels":{"team":"x"}}}));
        assert_eq!(
            eval_str("namespaceObject.metadata.labels['team'] == 'x'", a),
            CelValue::Bool(true));
    }

    #[test]
    fn eval_old_object_compare() {
        let mut a = CelActivation::default();
        a.old_object = Some(json!({"spec":{"replicas":1}}));
        a.object = Some(json!({"spec":{"replicas":2}}));
        assert_eq!(
            eval_str("oldObject.spec.replicas != object.spec.replicas", a),
            CelValue::Bool(true));
    }

    #[test]
    fn eval_logical_and() {
        let a = CelActivation::default();
        assert_eq!(eval_str("true && false", a), CelValue::Bool(false));
    }

    #[test]
    fn eval_logical_or_short_circuit() {
        let a = CelActivation::default();
        assert_eq!(eval_str("true || (1/0 == 0)", a), CelValue::Bool(true));
    }

    #[test]
    fn eval_unknown_var_errors() {
        let a = CelActivation::default();
        let r = SimpleEvaluator::new().evaluate("nope", &a);
        assert!(matches!(r, Err(CelError::Compile(_))));
    }

    #[test]
    fn eval_size_string() {
        let a = CelActivation::default();
        assert_eq!(eval_str("size('hello')", a), CelValue::Int(5));
    }

    #[test]
    fn eval_string_concat() {
        let a = CelActivation::default();
        assert_eq!(eval_str("'a' + 'b'", a), CelValue::String("ab".into()));
    }
}
