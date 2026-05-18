// SPDX-License-Identifier: AGPL-3.0-or-later
//! TraceQL — Grafana Tempo's trace query language.
//!
//! Grammar (simplified)
//! ─────────────────────
//! query        := '{' pipeline '}'
//! pipeline     := predicate (op predicate)*
//! predicate    := attribute comparator value
//!               | 'duration' comparator duration_lit
//!               | 'status' comparator status_val
//!               | 'name' comparator string_val
//! attribute    := '.' ident ('.' ident)*          (span attr)
//!               | 'resource.' ident ('.' ident)*  (resource attr)
//!               | 'span.' ident ('.' ident)*      (same as .)
//! comparator   := '=' | '!=' | '>' | '>=' | '<' | '<=' | '=~'
//! op           := '&&' | '||'
//! duration_lit := number ('ns' | 'us' | 'µs' | 'ms' | 's' | 'm' | 'h')
//! status_val   := 'ok' | 'error' | 'unset'
//!
//! Examples
//! ────────
//!   { .service.name = "frontend" }
//!   { duration > 500ms }
//!   { status = error }
//!   { .http.status_code >= 500 && .service.name = "api" }
//!   { .http.method =~ "GET|POST" }

use std::collections::HashMap;
use crate::types::{Span, SpanStatus, TagValue};
use crate::{Result, TraceError};

// ─── Token ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LBrace,
    RBrace,
    Dot,
    DotIdent(String),    // .foo or .foo.bar
    ResourceIdent(String), // resource.foo
    SpanIdent(String),   // span.foo
    Keyword(String),     // duration, status, name
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    RegexEq,
    And,
    Or,
    StringLit(String),
    IntLit(i64),
    FloatLit(f64),
    DurationLit(u64),     // nanoseconds
    StatusOk,
    StatusError,
    StatusUnset,
    Eof,
}

// ─── Lexer ────────────────────────────────────────────────────────────────

struct Lexer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self { Lexer { input, pos: 0 } }

    fn peek(&self) -> Option<char> { self.input[self.pos..].chars().next() }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
            self.advance();
        }
    }

    fn read_while(&mut self, pred: impl Fn(char) -> bool) -> &'a str {
        let start = self.pos;
        while self.peek().map(&pred).unwrap_or(false) {
            self.advance();
        }
        &self.input[start..self.pos]
    }

    fn read_string(&mut self) -> Result<String> {
        // consume opening quote (already consumed)
        let mut s = String::new();
        loop {
            match self.advance() {
                Some('"') => return Ok(s),
                Some('\\') => {
                    match self.advance() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some(c) => { s.push('\\'); s.push(c); }
                        None => return Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "unterminated escape".into() }),
                    }
                }
                Some(c) => s.push(c),
                None => return Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "unterminated string".into() }),
            }
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                tokens.push(Token::Eof);
                break;
            }
            let tok = match self.peek().unwrap() {
                '{' => { self.advance(); Token::LBrace }
                '}' => { self.advance(); Token::RBrace }
                '=' => {
                    self.advance();
                    if self.peek() == Some('~') { self.advance(); Token::RegexEq }
                    else { Token::Eq }
                }
                '!' => {
                    self.advance();
                    if self.peek() != Some('=') {
                        return Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "expected '!='".into() });
                    }
                    self.advance(); Token::NotEq
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); Token::Gte } else { Token::Gt }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); Token::Lte } else { Token::Lt }
                }
                '&' => {
                    self.advance();
                    if self.peek() != Some('&') {
                        return Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "expected '&&'".into() });
                    }
                    self.advance(); Token::And
                }
                '|' => {
                    self.advance();
                    if self.peek() != Some('|') {
                        return Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "expected '||'".into() });
                    }
                    self.advance(); Token::Or
                }
                '"' => { self.advance(); let s = self.read_string()?; Token::StringLit(s) }
                '.' => {
                    self.advance();
                    // read dotted path
                    let start = self.pos;
                    let ident = self.read_while(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '-');
                    Token::DotIdent(ident.to_owned())
                }
                c if c.is_alphabetic() || c == '_' => {
                    // Read only the first identifier segment (stop at dot for prefix detection)
                    let first_word = self.read_while(|c| c.is_alphanumeric() || c == '_');
                    match first_word {
                        "resource" | "span" => {
                            let is_resource = first_word == "resource";
                            // consume the dot separator
                            if self.peek() == Some('.') { self.advance(); }
                            // read the attribute path (may contain dots like service.name)
                            let attr = self.read_while(|c| c.is_alphanumeric() || c == '_' || c == '.');
                            if is_resource {
                                Token::ResourceIdent(attr.to_owned())
                            } else {
                                Token::SpanIdent(attr.to_owned())
                            }
                        }
                        "ok"    => Token::StatusOk,
                        "error" => Token::StatusError,
                        "unset" => Token::StatusUnset,
                        other   => {
                            // It may be a dotted keyword like "duration" — read any remaining dot segments
                            let rest = self.read_while(|c| c == '.' || c.is_alphanumeric() || c == '_');
                            let full = format!("{}{}", other, rest);
                            Token::Keyword(full)
                        }
                    }
                }
                c if c.is_ascii_digit() || (c == '-' && self.input[self.pos+1..].starts_with(|d: char| d.is_ascii_digit())) => {
                    let start = self.pos;
                    if c == '-' { self.advance(); }
                    let int_part = self.read_while(|c| c.is_ascii_digit());
                    let is_float = self.peek() == Some('.');
                    if is_float {
                        self.advance();
                        self.read_while(|c| c.is_ascii_digit());
                    }
                    let num_str = &self.input[start..self.pos];

                    // Check for duration suffix
                    let suffix_start = self.pos;
                    let suffix = self.read_while(|c| c.is_alphabetic() || c == 'µ');
                    match suffix {
                        "ns"  => Token::DurationLit(num_str.parse::<f64>().unwrap_or(0.0) as u64),
                        "us" | "µs" => Token::DurationLit((num_str.parse::<f64>().unwrap_or(0.0) * 1_000.0) as u64),
                        "ms"  => Token::DurationLit((num_str.parse::<f64>().unwrap_or(0.0) * 1_000_000.0) as u64),
                        "s"   => Token::DurationLit((num_str.parse::<f64>().unwrap_or(0.0) * 1_000_000_000.0) as u64),
                        "m"   => Token::DurationLit((num_str.parse::<f64>().unwrap_or(0.0) * 60_000_000_000.0) as u64),
                        "h"   => Token::DurationLit((num_str.parse::<f64>().unwrap_or(0.0) * 3_600_000_000_000.0) as u64),
                        "" if is_float => Token::FloatLit(num_str.parse().unwrap_or(0.0)),
                        "" => Token::IntLit(num_str.parse().unwrap_or(0)),
                        _ => {
                            // Put suffix back
                            self.pos = suffix_start;
                            if is_float { Token::FloatLit(num_str.parse().unwrap_or(0.0)) }
                            else { Token::IntLit(num_str.parse().unwrap_or(0)) }
                        }
                    }
                }
                c => {
                    return Err(TraceError::TraceQlSyntax {
                        pos: self.pos,
                        msg: format!("unexpected character: {:?}", c),
                    });
                }
            };
            tokens.push(tok);
        }
        Ok(tokens)
    }
}

// ─── AST ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Predicate {
    SpanAttr    { attr: String, op: CmpOp, value: Value },
    ResourceAttr { attr: String, op: CmpOp, value: Value },
    Duration    { op: CmpOp, nanos: u64 },
    Status      { op: CmpOp, status: SpanStatus },
    SpanName    { op: CmpOp, pattern: String },
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>,  Box<Predicate>),
}

#[derive(Debug, Clone)]
pub enum CmpOp { Eq, NotEq, Gt, Gte, Lt, Lte, Regex }

#[derive(Debug, Clone)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Status(SpanStatus),
}

// ─── Parser ───────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self { Parser { tokens, pos: 0 } }

    fn peek(&self) -> &Token { &self.tokens[self.pos] }

    fn consume(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() { self.pos += 1; }
        t
    }

    fn expect(&mut self, expected: &Token) -> Result<()>
    where Token: PartialEq
    {
        if self.peek() == expected {
            self.consume();
            Ok(())
        } else {
            Err(TraceError::TraceQlSyntax {
                pos: self.pos,
                msg: format!("expected {:?}, got {:?}", expected, self.peek()),
            })
        }
    }

    fn parse_query(&mut self) -> Result<Predicate> {
        self.expect(&Token::LBrace)?;
        let pred = self.parse_or()?;
        self.expect(&Token::RBrace)?;
        Ok(pred)
    }

    fn parse_or(&mut self) -> Result<Predicate> {
        let mut left = self.parse_and()?;
        while *self.peek() == Token::Or {
            self.consume();
            let right = self.parse_and()?;
            left = Predicate::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Predicate> {
        let mut left = self.parse_predicate()?;
        while *self.peek() == Token::And {
            self.consume();
            let right = self.parse_predicate()?;
            left = Predicate::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_predicate(&mut self) -> Result<Predicate> {
        match self.peek().clone() {
            Token::DotIdent(attr) => {
                self.consume();
                let op = self.parse_cmp_op()?;
                let val = self.parse_value()?;
                Ok(Predicate::SpanAttr { attr, op, value: val })
            }
            Token::ResourceIdent(attr) => {
                self.consume();
                let op = self.parse_cmp_op()?;
                let val = self.parse_value()?;
                Ok(Predicate::ResourceAttr { attr, op, value: val })
            }
            Token::SpanIdent(attr) => {
                self.consume();
                let op = self.parse_cmp_op()?;
                let val = self.parse_value()?;
                Ok(Predicate::SpanAttr { attr, op, value: val })
            }
            Token::Keyword(ref kw) => {
                match kw.as_str() {
                    "duration" => {
                        self.consume();
                        let op = self.parse_cmp_op()?;
                        if let Token::DurationLit(ns) = self.consume().clone() {
                            Ok(Predicate::Duration { op, nanos: ns })
                        } else {
                            Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "expected duration literal".into() })
                        }
                    }
                    "status" => {
                        self.consume();
                        let op = self.parse_cmp_op()?;
                        let status = match self.consume().clone() {
                            Token::StatusOk    => SpanStatus::Ok,
                            Token::StatusError => SpanStatus::Error,
                            Token::StatusUnset => SpanStatus::Unset,
                            other => return Err(TraceError::TraceQlSyntax { pos: self.pos, msg: format!("expected status, got {:?}", other) }),
                        };
                        Ok(Predicate::Status { op, status })
                    }
                    "name" => {
                        self.consume();
                        let op = self.parse_cmp_op()?;
                        if let Token::StringLit(s) = self.consume().clone() {
                            Ok(Predicate::SpanName { op, pattern: s })
                        } else {
                            Err(TraceError::TraceQlSyntax { pos: self.pos, msg: "expected string after name".into() })
                        }
                    }
                    other => Err(TraceError::TraceQlSyntax { pos: self.pos, msg: format!("unknown keyword: {}", other) }),
                }
            }
            other => Err(TraceError::TraceQlSyntax { pos: self.pos, msg: format!("unexpected token: {:?}", other) }),
        }
    }

    fn parse_cmp_op(&mut self) -> Result<CmpOp> {
        match self.consume().clone() {
            Token::Eq      => Ok(CmpOp::Eq),
            Token::NotEq   => Ok(CmpOp::NotEq),
            Token::Gt      => Ok(CmpOp::Gt),
            Token::Gte     => Ok(CmpOp::Gte),
            Token::Lt      => Ok(CmpOp::Lt),
            Token::Lte     => Ok(CmpOp::Lte),
            Token::RegexEq => Ok(CmpOp::Regex),
            other => Err(TraceError::TraceQlSyntax { pos: self.pos, msg: format!("expected comparator, got {:?}", other) }),
        }
    }

    fn parse_value(&mut self) -> Result<Value> {
        match self.consume().clone() {
            Token::StringLit(s) => Ok(Value::Str(s)),
            Token::IntLit(i)    => Ok(Value::Int(i)),
            Token::FloatLit(f)  => Ok(Value::Float(f)),
            Token::StatusOk     => Ok(Value::Status(SpanStatus::Ok)),
            Token::StatusError  => Ok(Value::Status(SpanStatus::Error)),
            Token::StatusUnset  => Ok(Value::Status(SpanStatus::Unset)),
            other => Err(TraceError::TraceQlSyntax { pos: self.pos, msg: format!("expected value, got {:?}", other) }),
        }
    }
}

// ─── Public parse entry point ─────────────────────────────────────────────

pub fn parse(query: &str) -> Result<Predicate> {
    let mut lexer = Lexer::new(query);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse_query()
}

// ─── Evaluator ────────────────────────────────────────────────────────────

/// Returns `true` if the span satisfies the predicate.
pub fn eval_span(pred: &Predicate, span: &Span) -> bool {
    match pred {
        Predicate::SpanAttr { attr, op, value } => {
            let tag_val = span.tags.get(attr.as_str());
            compare_tag(tag_val, op, value)
        }
        Predicate::ResourceAttr { attr, op, value } => {
            let tag_val = span.resource_attributes.get(attr.as_str());
            compare_tag(tag_val, op, value)
        }
        Predicate::Duration { op, nanos } => {
            compare_u64(span.duration_ns, op, *nanos)
        }
        Predicate::Status { op, status } => {
            compare_status(span.status, op, *status)
        }
        Predicate::SpanName { op, pattern } => {
            compare_str_val(&span.operation_name, op, pattern)
        }
        Predicate::And(a, b) => eval_span(a, span) && eval_span(b, span),
        Predicate::Or(a, b)  => eval_span(a, span) || eval_span(b, span),
    }
}

/// Filter a list of spans to those matching the query.
pub fn filter_spans<'a>(pred: &Predicate, spans: &'a [Span]) -> Vec<&'a Span> {
    spans.iter().filter(|s| eval_span(pred, s)).collect()
}

/// Returns trace IDs of traces that have at least one matching span.
pub fn matching_trace_ids(pred: &Predicate, spans: &[Span]) -> Vec<crate::types::TraceId> {
    use std::collections::HashSet;
    let mut ids: HashSet<crate::types::TraceId> = HashSet::new();
    for span in spans {
        if eval_span(pred, span) {
            ids.insert(span.trace_id);
        }
    }
    ids.into_iter().collect()
}

// ─── Comparison helpers ───────────────────────────────────────────────────

fn compare_tag(tag: Option<&TagValue>, op: &CmpOp, value: &Value) -> bool {
    match (tag, value) {
        (None, _) => matches!(op, CmpOp::NotEq),
        (Some(tv), Value::Str(s)) => compare_str_val(&tv.display(), op, s),
        (Some(tv), Value::Int(i)) => {
            if let Some(n) = tv.as_i64() { compare_i64(n, op, *i) }
            else { compare_str_val(&tv.display(), op, &i.to_string()) }
        }
        (Some(tv), Value::Float(f)) => {
            if let Some(n) = tv.as_f64() { compare_f64(n, op, *f) }
            else { false }
        }
        (Some(_), Value::Status(_)) => false,
    }
}

fn compare_str_val(actual: &str, op: &CmpOp, expected: &str) -> bool {
    match op {
        CmpOp::Eq    => actual == expected,
        CmpOp::NotEq => actual != expected,
        CmpOp::Gt    => actual > expected,
        CmpOp::Gte   => actual >= expected,
        CmpOp::Lt    => actual < expected,
        CmpOp::Lte   => actual <= expected,
        CmpOp::Regex => regex_match(actual, expected),
    }
}

fn compare_u64(actual: u64, op: &CmpOp, expected: u64) -> bool {
    match op {
        CmpOp::Eq    => actual == expected,
        CmpOp::NotEq => actual != expected,
        CmpOp::Gt    => actual >  expected,
        CmpOp::Gte   => actual >= expected,
        CmpOp::Lt    => actual <  expected,
        CmpOp::Lte   => actual <= expected,
        CmpOp::Regex => false,
    }
}

fn compare_i64(actual: i64, op: &CmpOp, expected: i64) -> bool {
    match op {
        CmpOp::Eq    => actual == expected,
        CmpOp::NotEq => actual != expected,
        CmpOp::Gt    => actual >  expected,
        CmpOp::Gte   => actual >= expected,
        CmpOp::Lt    => actual <  expected,
        CmpOp::Lte   => actual <= expected,
        CmpOp::Regex => false,
    }
}

fn compare_f64(actual: f64, op: &CmpOp, expected: f64) -> bool {
    match op {
        CmpOp::Eq    => (actual - expected).abs() < f64::EPSILON,
        CmpOp::NotEq => (actual - expected).abs() >= f64::EPSILON,
        CmpOp::Gt    => actual >  expected,
        CmpOp::Gte   => actual >= expected,
        CmpOp::Lt    => actual <  expected,
        CmpOp::Lte   => actual <= expected,
        CmpOp::Regex => false,
    }
}

fn compare_status(actual: SpanStatus, op: &CmpOp, expected: SpanStatus) -> bool {
    match op {
        CmpOp::Eq    => actual == expected,
        CmpOp::NotEq => actual != expected,
        _ => false,
    }
}

/// Simple regex-like matching using '.*' wildcards and literal '|' alternation.
/// For full regex support a regex crate would be needed; this covers common cases.
fn regex_match(text: &str, pattern: &str) -> bool {
    // Support alternation via |
    for alt in pattern.split('|') {
        if glob_match(text, alt.trim()) {
            return true;
        }
    }
    false
}

fn glob_match(text: &str, pattern: &str) -> bool {
    let (mut ti, mut pi) = (0usize, 0usize);
    let tb = text.as_bytes();
    let pb = pattern.as_bytes();
    let mut star_pi = usize::MAX;
    let mut star_ti = usize::MAX;

    while ti < tb.len() {
        if pi < pb.len() && (pb[pi] == b'.' && pi + 1 < pb.len() && pb[pi + 1] == b'*') {
            // .* wildcard
            star_pi = pi;
            star_ti = ti;
            pi += 2;
        } else if pi < pb.len() && pb[pi] != b'.' && (pb[pi] == tb[ti] || pb[pi] == b'?') {
            ti += 1; pi += 1;
        } else if star_pi != usize::MAX {
            star_ti += 1;
            ti = star_ti;
            pi = star_pi + 2;
        } else {
            return false;
        }
    }

    while pi + 1 < pb.len() && pb[pi] == b'.' && pb[pi + 1] == b'*' {
        pi += 2;
    }
    pi >= pb.len()
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::collections::HashMap;

    fn make_span() -> Span {
        let mut tags = HashMap::new();
        tags.insert("http.method".into(), TagValue::String("GET".into()));
        tags.insert("http.status_code".into(), TagValue::Int(200));
        Span {
            trace_id: 1,
            span_id: 1,
            parent_span_id: None,
            operation_name: "GET /users".into(),
            service_name: "frontend".into(),
            start_time_unix_nano: 0,
            end_time_unix_nano: 10_000_000,
            duration_ns: 10_000_000,
            status: SpanStatus::Ok,
            kind: SpanKind::Server,
            tags,
            events: vec![],
            links: vec![],
            resource_attributes: {
                let mut m = HashMap::new();
                m.insert("service.name".into(), TagValue::String("frontend".into()));
                m
            },
            tenant_id: "default".into(),
            baggage: HashMap::new(),
            log_labels: HashMap::new(),
        }
    }

    #[test]
    fn parse_span_attr_eq() {
        let pred = parse(r#"{ .http.method = "GET" }"#).unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn parse_duration_gt() {
        let pred = parse("{ duration > 5ms }").unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span)); // 10ms > 5ms
    }

    #[test]
    fn parse_duration_lt() {
        let pred = parse("{ duration < 5ms }").unwrap();
        let span = make_span();
        assert!(!eval_span(&pred, &span)); // 10ms not < 5ms
    }

    #[test]
    fn parse_status_ok() {
        let pred = parse("{ status = ok }").unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn parse_status_error_no_match() {
        let pred = parse("{ status = error }").unwrap();
        let span = make_span();
        assert!(!eval_span(&pred, &span));
    }

    #[test]
    fn parse_and_condition() {
        let pred = parse(r#"{ .http.method = "GET" && status = ok }"#).unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn parse_or_condition() {
        let pred = parse(r#"{ status = error || .http.method = "GET" }"#).unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn parse_resource_attr() {
        let pred = parse(r#"{ resource.service.name = "frontend" }"#).unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn parse_int_comparison() {
        let pred = parse("{ .http.status_code >= 200 }").unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn parse_name_match() {
        let pred = parse(r#"{ name = "GET /users" }"#).unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn missing_attribute_not_eq() {
        let pred = parse(r#"{ .missing != "anything" }"#).unwrap();
        let span = make_span();
        assert!(eval_span(&pred, &span));
    }

    #[test]
    fn filter_spans_subset() {
        let spans = vec![make_span(), {
            let mut s = make_span();
            s.status = SpanStatus::Error;
            s
        }];
        let pred = parse("{ status = error }").unwrap();
        let filtered = filter_spans(&pred, &spans);
        assert_eq!(filtered.len(), 1);
    }
}
