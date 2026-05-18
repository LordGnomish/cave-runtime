// SPDX-License-Identifier: AGPL-3.0-or-later
//! Falco condition language — tokenizer, recursive-descent parser, and evaluator.
//!
//! Supports the full Falco condition grammar:
//!   condition ::= disjunction
//!   disjunction ::= conjunction ("or" conjunction)*
//!   conjunction ::= negation ("and" negation)*
//!   negation ::= "not" negation | atom
//!   atom ::= "(" condition ")" | comparison | macro_ref
//!   comparison ::= field op value
//!   op ::= "=" | "!=" | "<" | ">" | "<=" | ">=" | "contains" | "icontains"
//!        | "startswith" | "endswith" | "in" | "pmatch" | "glob"
//!   value ::= string_literal | number | "(" list_values ")"

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(String),
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    LParen,
    RParen,
    StringLit(String),
    NumberLit(f64),
    Comma,
}

pub fn tokenize(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    let mut tokens = Vec::new();

    while pos < chars.len() {
        match chars[pos] {
            ' ' | '\t' | '\n' | '\r' => pos += 1,
            '(' => { tokens.push(Token::LParen); pos += 1; }
            ')' => { tokens.push(Token::RParen); pos += 1; }
            ',' => { tokens.push(Token::Comma); pos += 1; }
            '\'' | '"' => {
                let q = chars[pos];
                pos += 1;
                let mut s = String::new();
                while pos < chars.len() && chars[pos] != q {
                    if chars[pos] == '\\' && pos + 1 < chars.len() {
                        pos += 1;
                        s.push(match chars[pos] {
                            'n' => '\n', 't' => '\t', c => c,
                        });
                    } else {
                        s.push(chars[pos]);
                    }
                    pos += 1;
                }
                if pos < chars.len() { pos += 1; } // closing quote
                tokens.push(Token::StringLit(s));
            }
            '=' => { tokens.push(Token::Eq); pos += 1; }
            '!' => {
                pos += 1;
                if pos < chars.len() && chars[pos] == '=' {
                    pos += 1;
                    tokens.push(Token::Neq);
                }
                // bare `!` unsupported; skip
            }
            '<' => {
                pos += 1;
                if pos < chars.len() && chars[pos] == '=' {
                    pos += 1;
                    tokens.push(Token::Lte);
                } else {
                    tokens.push(Token::Lt);
                }
            }
            '>' => {
                pos += 1;
                if pos < chars.len() && chars[pos] == '=' {
                    pos += 1;
                    tokens.push(Token::Gte);
                } else {
                    tokens.push(Token::Gt);
                }
            }
            c if c.is_alphanumeric() || c == '_' || c == '.' || c == '/' || c == '-' => {
                let start = pos;
                while pos < chars.len()
                    && (chars[pos].is_alphanumeric()
                        || chars[pos] == '_'
                        || chars[pos] == '.'
                        || chars[pos] == '/'
                        || chars[pos] == '-')
                {
                    pos += 1;
                }
                let word: String = chars[start..pos].iter().collect();
                if let Ok(n) = word.parse::<f64>() {
                    tokens.push(Token::NumberLit(n));
                } else {
                    tokens.push(Token::Word(word));
                }
            }
            _ => pos += 1,
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CompareOp {
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    Contains,
    Icontains,
    Startswith,
    Endswith,
    Pmatch,
    Glob,
}

#[derive(Debug, Clone)]
pub enum Value {
    Str(String),
    Num(f64),
    /// Reference to a named list (resolved during eval)
    ListRef(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Compare {
        field: String,
        op: CompareOp,
        value: Value,
    },
    InList {
        field: String,
        negated: bool,
        /// Inline values (from parenthesised literal list)
        values: Vec<String>,
        /// OR reference to a named list
        list_ref: Option<String>,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    /// Bare identifier — resolved as macro at eval time.
    MacroRef(String),
    Literal(bool),
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        self.pos += 1;
        t
    }

    fn peek_word(&self) -> Option<&str> {
        match self.peek() {
            Some(Token::Word(w)) => Some(w.as_str()),
            _ => None,
        }
    }

    fn peek_word_is(&self, s: &str) -> bool {
        self.peek_word() == Some(s)
    }

    fn consume_word(&mut self) -> Option<String> {
        match self.tokens.get(self.pos) {
            Some(Token::Word(_)) => {
                if let Some(Token::Word(w)) = self.advance().cloned().as_ref() {
                    return Some(w.clone());
                }
                None
            }
            _ => None,
        }
    }

    fn consume_word_if(&mut self, s: &str) -> bool {
        if self.peek_word_is(s) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub fn parse(&mut self) -> Expr {
        let e = self.parse_or();
        e
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        while self.consume_word_if("or") {
            let right = self.parse_and();
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_not();
        while self.consume_word_if("and") {
            let right = self.parse_not();
            left = Expr::And(Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_not(&mut self) -> Expr {
        if self.consume_word_if("not") {
            let inner = self.parse_not();
            return Expr::Not(Box::new(inner));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Expr {
        // Parenthesised sub-expression
        if matches!(self.peek(), Some(Token::LParen)) {
            self.advance();
            let e = self.parse_or();
            if matches!(self.peek(), Some(Token::RParen)) {
                self.advance();
            }
            return e;
        }

        // Word: field, macro reference, or keyword
        if let Some(Token::Word(w)) = self.peek().cloned() {
            self.advance();

            // look for a binary operator
            match self.peek() {
                Some(Token::Eq) => {
                    self.advance();
                    let val = self.parse_rhs_value();
                    return Expr::Compare { field: w, op: CompareOp::Eq, value: val };
                }
                Some(Token::Neq) => {
                    self.advance();
                    let val = self.parse_rhs_value();
                    return Expr::Compare { field: w, op: CompareOp::Neq, value: val };
                }
                Some(Token::Lt) => {
                    self.advance();
                    let val = self.parse_rhs_value();
                    return Expr::Compare { field: w, op: CompareOp::Lt, value: val };
                }
                Some(Token::Gt) => {
                    self.advance();
                    let val = self.parse_rhs_value();
                    return Expr::Compare { field: w, op: CompareOp::Gt, value: val };
                }
                Some(Token::Lte) => {
                    self.advance();
                    let val = self.parse_rhs_value();
                    return Expr::Compare { field: w, op: CompareOp::Lte, value: val };
                }
                Some(Token::Gte) => {
                    self.advance();
                    let val = self.parse_rhs_value();
                    return Expr::Compare { field: w, op: CompareOp::Gte, value: val };
                }
                Some(Token::Word(op_word)) => {
                    let op_word = op_word.clone();
                    match op_word.as_str() {
                        "contains" => {
                            self.advance();
                            let val = self.parse_rhs_value();
                            return Expr::Compare { field: w, op: CompareOp::Contains, value: val };
                        }
                        "icontains" => {
                            self.advance();
                            let val = self.parse_rhs_value();
                            return Expr::Compare { field: w, op: CompareOp::Icontains, value: val };
                        }
                        "startswith" => {
                            self.advance();
                            let val = self.parse_rhs_value();
                            return Expr::Compare { field: w, op: CompareOp::Startswith, value: val };
                        }
                        "endswith" => {
                            self.advance();
                            let val = self.parse_rhs_value();
                            return Expr::Compare { field: w, op: CompareOp::Endswith, value: val };
                        }
                        "pmatch" => {
                            self.advance();
                            let val = self.parse_rhs_value();
                            return Expr::Compare { field: w, op: CompareOp::Pmatch, value: val };
                        }
                        "glob" => {
                            self.advance();
                            let val = self.parse_rhs_value();
                            return Expr::Compare { field: w, op: CompareOp::Glob, value: val };
                        }
                        "in" => {
                            self.advance();
                            return self.parse_in_expr(w, false);
                        }
                        _ => {
                            // It's a bare macro reference
                            return Expr::MacroRef(w);
                        }
                    }
                }
                _ => {
                    // Bare word with no operator → macro reference
                    return Expr::MacroRef(w);
                }
            }
        }

        Expr::Literal(true)
    }

    /// Parse the RHS of `field in (...)` or `field in list_name`
    fn parse_in_expr(&mut self, field: String, negated: bool) -> Expr {
        // `in (v1, v2, ...)` — inline list
        if matches!(self.peek(), Some(Token::LParen)) {
            self.advance();
            let mut values = Vec::new();
            loop {
                match self.peek() {
                    Some(Token::RParen) | None => {
                        self.advance();
                        break;
                    }
                    Some(Token::Comma) => {
                        self.advance();
                    }
                    Some(Token::StringLit(_)) => {
                        if let Some(Token::StringLit(s)) = self.advance().cloned() {
                            values.push(s);
                        }
                    }
                    Some(Token::Word(_)) => {
                        // Could be a bare identifier (list name) — use first one as list_ref
                        if let Some(name) = self.consume_word() {
                            if values.is_empty() && matches!(self.peek(), Some(Token::RParen)) {
                                self.advance(); // consume ')'
                                return Expr::InList {
                                    field,
                                    negated,
                                    values: vec![],
                                    list_ref: Some(name),
                                };
                            }
                            values.push(name);
                        }
                    }
                    Some(Token::NumberLit(_)) => {
                        if let Some(Token::NumberLit(n)) = self.advance().cloned() {
                            values.push(n.to_string());
                        }
                    }
                    _ => { self.advance(); }
                }
            }
            Expr::InList { field, negated, values, list_ref: None }
        } else if let Some(list_name) = self.consume_word() {
            // `in list_name` — named list reference
            Expr::InList { field, negated, values: vec![], list_ref: Some(list_name) }
        } else {
            Expr::Literal(false)
        }
    }

    fn parse_rhs_value(&mut self) -> Value {
        match self.peek().cloned() {
            Some(Token::StringLit(s)) => {
                self.advance();
                Value::Str(s)
            }
            Some(Token::NumberLit(n)) => {
                self.advance();
                Value::Num(n)
            }
            Some(Token::Word(w)) => {
                self.advance();
                // In a comparison RHS position, bare words are string literals.
                // List references only appear inside `in (list_name)` expressions,
                // which are handled separately in parse_in_expr.
                Value::Str(w)
            }
            // Falco uses unquoted `>` and `<` as string values for evt.dir
            Some(Token::Gt) => { self.advance(); Value::Str(">".into()) }
            Some(Token::Lt) => { self.advance(); Value::Str("<".into()) }
            _ => Value::Str(String::new()),
        }
    }
}

/// Parse a condition string into an AST Expr.
pub fn parse_condition(condition: &str) -> Expr {
    let tokens = tokenize(condition);
    let mut parser = Parser::new(tokens);
    parser.parse()
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

pub struct EvalContext<'a> {
    /// Resolved event fields (field name → string value).
    pub fields: &'a HashMap<String, String>,
    /// Named lists (list name → items).
    pub lists: &'a HashMap<String, Vec<String>>,
    /// Compiled macro expressions (macro name → AST).
    pub macros: &'a HashMap<String, Expr>,
}

pub fn eval(expr: &Expr, ctx: &EvalContext<'_>) -> bool {
    match expr {
        Expr::Literal(b) => *b,
        Expr::MacroRef(name) => {
            if let Some(macro_expr) = ctx.macros.get(name) {
                eval(macro_expr, ctx)
            } else {
                false
            }
        }
        Expr::And(l, r) => eval(l, ctx) && eval(r, ctx),
        Expr::Or(l, r) => eval(l, ctx) || eval(r, ctx),
        Expr::Not(e) => !eval(e, ctx),
        Expr::InList { field, negated, values, list_ref } => {
            let fv = ctx.fields.get(field.as_str()).map(String::as_str).unwrap_or("");
            let matched = if let Some(lref) = list_ref {
                ctx.lists
                    .get(lref.as_str())
                    .map(|items| items.iter().any(|i| i == fv))
                    .unwrap_or(false)
            } else {
                values.iter().any(|v| v == fv)
            };
            if *negated { !matched } else { matched }
        }
        Expr::Compare { field, op, value } => {
            let fv = ctx.fields.get(field.as_str()).map(String::as_str).unwrap_or("");
            match value {
                Value::Num(n) => eval_numeric(fv, *n, op),
                Value::Str(s) => eval_string(fv, s.as_str(), op),
                Value::ListRef(lref) => {
                    // Treat as "fv in list"
                    ctx.lists
                        .get(lref.as_str())
                        .map(|items| items.iter().any(|i| i == fv))
                        .unwrap_or(false)
                }
            }
        }
    }
}

fn eval_string(field_val: &str, rhs: &str, op: &CompareOp) -> bool {
    match op {
        CompareOp::Eq => field_val == rhs,
        CompareOp::Neq => field_val != rhs,
        CompareOp::Lt => field_val < rhs,
        CompareOp::Gt => field_val > rhs,
        CompareOp::Lte => field_val <= rhs,
        CompareOp::Gte => field_val >= rhs,
        CompareOp::Contains => field_val.contains(rhs),
        CompareOp::Icontains => field_val.to_lowercase().contains(&rhs.to_lowercase()),
        CompareOp::Startswith => field_val.starts_with(rhs),
        CompareOp::Endswith => field_val.ends_with(rhs),
        CompareOp::Pmatch => pmatch(field_val, rhs),
        CompareOp::Glob => glob_match(rhs, field_val),
    }
}

fn eval_numeric(field_val: &str, rhs: f64, op: &CompareOp) -> bool {
    let lhs: f64 = field_val.parse().unwrap_or(f64::NAN);
    match op {
        CompareOp::Eq => (lhs - rhs).abs() < f64::EPSILON,
        CompareOp::Neq => (lhs - rhs).abs() >= f64::EPSILON,
        CompareOp::Lt => lhs < rhs,
        CompareOp::Gt => lhs > rhs,
        CompareOp::Lte => lhs <= rhs,
        CompareOp::Gte => lhs >= rhs,
        _ => false,
    }
}

/// Simple pattern match: `*` as wildcard, `?` for single char.
fn pmatch(s: &str, pattern: &str) -> bool {
    glob_match(pattern, s)
}

/// Glob pattern matching (`*` = any substring, `?` = single char).
pub fn glob_match(pattern: &str, s: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    glob_dp(&p, &t, 0, 0)
}

fn glob_dp(p: &[char], t: &[char], pi: usize, ti: usize) -> bool {
    if pi == p.len() {
        return ti == t.len();
    }
    if p[pi] == '*' {
        // skip consecutive stars
        let next_pi = pi + 1;
        if next_pi == p.len() {
            return true; // trailing star matches rest
        }
        for i in ti..=t.len() {
            if glob_dp(p, t, next_pi, i) {
                return true;
            }
        }
        return false;
    }
    if ti == t.len() {
        return false;
    }
    if p[pi] == '?' || p[pi] == t[ti] {
        return glob_dp(p, t, pi + 1, ti + 1);
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx<'a>(
        fields: &'a HashMap<String, String>,
        lists: &'a HashMap<String, Vec<String>>,
        macros: &'a HashMap<String, Expr>,
    ) -> EvalContext<'a> {
        EvalContext { fields, lists, macros }
    }

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn simple_eq() {
        let expr = parse_condition("evt.type = \"execve\"");
        let f = fields(&[("evt.type", "execve")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn boolean_and() {
        let expr = parse_condition("evt.type = \"execve\" and proc.name = \"bash\"");
        let f = fields(&[("evt.type", "execve"), ("proc.name", "bash")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn boolean_not() {
        let expr = parse_condition("not proc.name = \"root\"");
        let f = fields(&[("proc.name", "bash")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn in_list() {
        let expr = parse_condition("proc.name in (bash, sh, zsh)");
        let f = fields(&[("proc.name", "bash")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn named_list() {
        let expr = parse_condition("proc.name in (shell_binaries)");
        let f = fields(&[("proc.name", "zsh")]);
        let mut lists: HashMap<String, Vec<String>> = HashMap::new();
        lists.insert(
            "shell_binaries".into(),
            vec!["bash".into(), "sh".into(), "zsh".into()],
        );
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn startswith_op() {
        let expr = parse_condition("fd.name startswith /etc");
        let f = fields(&[("fd.name", "/etc/passwd")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn icontains_op() {
        let expr = parse_condition("proc.cmdline icontains CURL");
        let f = fields(&[("proc.cmdline", "curl http://example.com")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn glob_match_test() {
        assert!(glob_match("*.log", "access.log"));
        assert!(glob_match("/etc/*", "/etc/passwd"));
        assert!(!glob_match("*.log", "access.txt"));
        assert!(glob_match("proc.?ame", "proc.name"));
    }

    #[test]
    fn numeric_compare() {
        let expr = parse_condition("user.uid != 0");
        let f = fields(&[("user.uid", "1000")]);
        let lists = HashMap::new();
        let macros = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }

    #[test]
    fn macro_ref() {
        let mut macros = HashMap::new();
        macros.insert(
            "is_shell".into(),
            parse_condition("proc.name in (bash, sh)"),
        );
        let expr = parse_condition("is_shell");
        let f = fields(&[("proc.name", "bash")]);
        let lists = HashMap::new();
        assert!(eval(&expr, &make_ctx(&f, &lists, &macros)));
    }
}
