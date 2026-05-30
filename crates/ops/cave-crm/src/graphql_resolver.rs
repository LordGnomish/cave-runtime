// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! GraphQL query resolver runtime (read path).
//!
//! Twenty auto-generates a resolver pair per object from its
//! ObjectMetadata — `findOne` (singular, `filter` → a single record) and
//! `findMany` (plural, returns an `IConnection<T>` envelope
//! `{ edges:[{ node, cursor }], pageInfo, totalCount }`). See
//! `packages/twenty-server/src/engine/api/graphql/workspace-resolver-builder/`.
//!
//! This module ports the *execution* contract: a hand-rolled parser for the
//! query subset cave-crm serves (root fields + args + nested selection set)
//! and an executor that filters / orders / paginates a record set and
//! projects the requested columns into the Connection envelope. It is the
//! runtime that `graphql_schema.rs` previously only described — closing the
//! `[[partial]] graphql-resolvers` gap.

use serde_json::{json, Map, Value};
use std::cmp::Ordering;

/// One object collection the resolver can serve, identified by its
/// `findOne` (singular) and `findMany` (plural) root-field names. Rows are
/// snake_case-keyed JSON (as produced by the model `Serialize` impls); the
/// resolver projects them to camelCase per the selection set.
#[derive(Debug, Clone)]
pub struct ObjectData {
    pub singular: String,
    pub plural: String,
    pub rows: Vec<Value>,
}

impl ObjectData {
    pub fn new(singular: impl Into<String>, plural: impl Into<String>, rows: Vec<Value>) -> Self {
        Self {
            singular: singular.into(),
            plural: plural.into(),
            rows,
        }
    }
}

/// In-memory GraphQL executor over a fixed set of object collections.
#[derive(Debug, Clone)]
pub struct GraphQlResolver {
    objects: Vec<ObjectData>,
}

impl GraphQlResolver {
    pub fn new(objects: Vec<ObjectData>) -> Self {
        Self { objects }
    }

    /// Execute a query document and return a GraphQL response envelope
    /// `{ "data": { ... }, "errors": [ ... ] }`. `errors` is omitted when
    /// empty (spec behaviour).
    pub fn execute(&self, query: &str) -> Value {
        let roots = match parse_document(query) {
            Ok(r) => r,
            Err(e) => {
                return json!({ "data": Value::Null, "errors": [{ "message": e }] });
            }
        };

        let mut data = Map::new();
        let mut errors: Vec<Value> = Vec::new();

        for field in &roots {
            match self.resolve_root(field) {
                Ok(v) => {
                    data.insert(field.name.clone(), v);
                }
                Err(msg) => {
                    data.insert(field.name.clone(), Value::Null);
                    errors.push(json!({ "message": msg }));
                }
            }
        }

        let mut out = Map::new();
        out.insert("data".into(), Value::Object(data));
        if !errors.is_empty() {
            out.insert("errors".into(), Value::Array(errors));
        }
        Value::Object(out)
    }

    fn resolve_root(&self, field: &Field) -> Result<Value, String> {
        // findMany (plural) takes precedence over a same-named singular.
        if let Some(obj) = self.objects.iter().find(|o| o.plural == field.name) {
            return Ok(self.find_many(obj, field));
        }
        if let Some(obj) = self.objects.iter().find(|o| o.singular == field.name) {
            return Ok(self.find_one(obj, field));
        }
        Err(format!("Cannot query field \"{}\"", field.name))
    }

    fn find_one(&self, obj: &ObjectData, field: &Field) -> Value {
        let filter = field.args.get("filter");
        match obj.rows.iter().find(|r| matches_filter(r, filter)) {
            Some(row) => project(row, &field.selection),
            None => Value::Null,
        }
    }

    fn find_many(&self, obj: &ObjectData, field: &Field) -> Value {
        // 1. filter
        let mut rows: Vec<Value> = obj
            .rows
            .iter()
            .filter(|r| matches_filter(r, field.args.get("filter")))
            .cloned()
            .collect();

        // 2. order (cascading, left-to-right)
        let order = parse_order_by(field.args.get("orderBy"));
        if !order.is_empty() {
            rows.sort_by(|a, b| {
                for (col, desc) in &order {
                    let o = cmp_values(field_value(a, col), field_value(b, col));
                    let o = if *desc { o.reverse() } else { o };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                Ordering::Equal
            });
        }

        let total = rows.len();

        // 3. paginate (offset / after-cursor + first)
        let start = field
            .args
            .get("after")
            .and_then(|c| c.as_str())
            .and_then(decode_cursor)
            .map(|i| i + 1)
            .or_else(|| field.args.get("offset").and_then(|v| v.as_u64()).map(|v| v as usize))
            .unwrap_or(0);
        let limit = field
            .args
            .get("first")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let end = match limit {
            Some(n) => (start + n).min(total),
            None => total,
        };
        let window: Vec<(usize, &Value)> = rows
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();

        let node_sel = field
            .selection
            .iter()
            .find(|f| f.name == "edges")
            .and_then(|e| e.selection.iter().find(|f| f.name == "node"))
            .map(|n| n.selection.clone())
            .unwrap_or_default();

        let edges: Vec<Value> = window
            .iter()
            .map(|(idx, row)| {
                json!({
                    "node": project(row, &node_sel),
                    "cursor": encode_cursor(*idx),
                })
            })
            .collect();

        let has_next = end < total;
        let has_prev = start > 0;
        let start_cursor = window.first().map(|(i, _)| encode_cursor(*i));
        let end_cursor = window.last().map(|(i, _)| encode_cursor(*i));

        let connection = json!({
            "edges": edges,
            "totalCount": total,
            "pageInfo": {
                "hasNextPage": has_next,
                "hasPreviousPage": has_prev,
                "startCursor": start_cursor,
                "endCursor": end_cursor,
            }
        });

        // Project the connection down to exactly the selected sub-fields.
        project(&connection, &field.selection)
    }
}

// ─── projection ──────────────────────────────────────────────────────────────

/// Project `source` to the selection set, renaming to the requested
/// (camelCase) keys and recursing into objects / arrays. A record's `id` is
/// always included (Twenty's implicit-id behaviour).
fn project(source: &Value, selection: &[Field]) -> Value {
    if selection.is_empty() {
        return source.clone();
    }
    match source {
        Value::Array(items) => {
            Value::Array(items.iter().map(|it| project(it, selection)).collect())
        }
        Value::Object(_) => {
            let mut out = Map::new();
            for f in selection {
                let v = field_value(source, &f.name).clone();
                let projected = if f.selection.is_empty() {
                    v
                } else {
                    project(&v, &f.selection)
                };
                out.insert(f.name.clone(), projected);
            }
            // implicit id
            if let Some(id) = source.get("id") {
                out.entry("id".to_string()).or_insert_with(|| id.clone());
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Look up a (camelCase) field in a snake_case-or-camel-keyed object.
fn field_value<'a>(source: &'a Value, name: &str) -> &'a Value {
    if let Some(v) = source.get(name) {
        return v;
    }
    let snake = to_snake(name);
    source.get(&snake).unwrap_or(&Value::Null)
}

// ─── filtering ─────────────────────────────────────────────────────────────

fn matches_filter(row: &Value, filter: Option<&Value>) -> bool {
    let Some(Value::Object(f)) = filter else {
        return true;
    };
    f.iter().all(|(k, matcher)| match matcher {
        Value::Object(ops) => ops.iter().all(|(op, expected)| {
            let actual = field_value(row, k);
            apply_operator(op, actual, expected)
        }),
        scalar => field_value(row, k) == scalar,
    })
}

fn apply_operator(op: &str, actual: &Value, expected: &Value) -> bool {
    match op {
        "eq" => actual == expected,
        "neq" => actual != expected,
        "gt" => cmp_values(actual, expected) == Ordering::Greater,
        "gte" => cmp_values(actual, expected) != Ordering::Less,
        "lt" => cmp_values(actual, expected) == Ordering::Less,
        "lte" => cmp_values(actual, expected) != Ordering::Greater,
        "in" => expected.as_array().map(|a| a.contains(actual)).unwrap_or(false),
        "ilike" => {
            let hay = actual.as_str().unwrap_or("").to_lowercase();
            let needle = expected.as_str().unwrap_or("").to_lowercase();
            hay.contains(&needle)
        }
        _ => false,
    }
}

// ─── ordering ────────────────────────────────────────────────────────────────

/// Parse `orderBy` into `(column, descending)` pairs. Accepts a single
/// object `{ field: ASC }` or an array `[{ field: ASC }, ...]`.
fn parse_order_by(arg: Option<&Value>) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let push_obj = |obj: &Map<String, Value>, out: &mut Vec<(String, bool)>| {
        for (k, dir) in obj {
            let desc = dir
                .as_str()
                .map(|s| s.starts_with("DESC"))
                .unwrap_or(false);
            out.push((to_snake(k), desc));
        }
    };
    match arg {
        Some(Value::Object(o)) => push_obj(o, &mut out),
        Some(Value::Array(items)) => {
            for it in items {
                if let Value::Object(o) = it {
                    push_obj(o, &mut out);
                }
            }
        }
        _ => {}
    }
    out
}

fn cmp_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&y.as_f64().unwrap_or(0.0))
            .unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        _ => a.to_string().cmp(&b.to_string()),
    }
}

// ─── cursors (opaque base64 of the absolute index) ──────────────────────────

fn encode_cursor(idx: usize) -> String {
    base64_encode(format!("cursor:{}", idx).as_bytes())
}

fn decode_cursor(c: &str) -> Option<usize> {
    let bytes = base64_decode(c)?;
    let s = String::from_utf8(bytes).ok()?;
    s.strip_prefix("cursor:")?.parse().ok()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
    let clean: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        let mut bits = 0;
        for &c in chunk {
            n = (n << 6) | val(c)?;
            bits += 6;
        }
        n <<= 24 - bits;
        let nbytes = bits / 8;
        for i in 0..nbytes {
            out.push(((n >> (16 - i * 8)) & 0xff) as u8);
        }
    }
    Some(out)
}

// ─── parser ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Field {
    name: String,
    args: Map<String, Value>,
    selection: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Colon,
    Name(String),
    Str(String),
    Num(serde_json::Number),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let bytes: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut toks = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            ws if ws.is_whitespace() || ws == ',' => i += 1,
            '{' => {
                toks.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                toks.push(Tok::RBrace);
                i += 1;
            }
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '[' => {
                toks.push(Tok::LBracket);
                i += 1;
            }
            ']' => {
                toks.push(Tok::RBracket);
                i += 1;
            }
            ':' => {
                toks.push(Tok::Colon);
                i += 1;
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                while i < bytes.len() && bytes[i] != '"' {
                    if bytes[i] == '\\' && i + 1 < bytes.len() {
                        i += 1;
                        s.push(match bytes[i] {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                    } else {
                        s.push(bytes[i]);
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err("unterminated string".into());
                }
                i += 1; // closing quote
                toks.push(Tok::Str(s));
            }
            d if d.is_ascii_digit() || d == '-' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                    i += 1;
                }
                let num: String = bytes[start..i].iter().collect();
                let n: serde_json::Number = num
                    .parse()
                    .map_err(|_| format!("bad number: {}", num))?;
                toks.push(Tok::Num(n));
            }
            a if a.is_alphabetic() || a == '_' => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                toks.push(Tok::Name(bytes[start..i].iter().collect()));
            }
            other => return Err(format!("unexpected char: {:?}", other)),
        }
    }
    Ok(toks)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn expect(&mut self, t: &Tok) -> Result<(), String> {
        match self.next() {
            Some(ref got) if got == t => Ok(()),
            other => Err(format!("expected {:?}, got {:?}", t, other)),
        }
    }

    fn parse_selection_set(&mut self) -> Result<Vec<Field>, String> {
        self.expect(&Tok::LBrace)?;
        let mut fields = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return Err("unexpected end of selection set".into());
            }
            fields.push(self.parse_field()?);
        }
        self.expect(&Tok::RBrace)?;
        Ok(fields)
    }

    fn parse_field(&mut self) -> Result<Field, String> {
        let name = match self.next() {
            Some(Tok::Name(n)) => n,
            other => return Err(format!("expected field name, got {:?}", other)),
        };
        let args = if self.peek() == Some(&Tok::LParen) {
            self.parse_args()?
        } else {
            Map::new()
        };
        let selection = if self.peek() == Some(&Tok::LBrace) {
            self.parse_selection_set()?
        } else {
            Vec::new()
        };
        Ok(Field {
            name,
            args,
            selection,
        })
    }

    fn parse_args(&mut self) -> Result<Map<String, Value>, String> {
        self.expect(&Tok::LParen)?;
        let mut args = Map::new();
        while self.peek() != Some(&Tok::RParen) {
            let key = match self.next() {
                Some(Tok::Name(n)) => n,
                other => return Err(format!("expected arg name, got {:?}", other)),
            };
            self.expect(&Tok::Colon)?;
            let val = self.parse_value()?;
            args.insert(key, val);
        }
        self.expect(&Tok::RParen)?;
        Ok(args)
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(Value::String(s)),
            Some(Tok::Num(n)) => Ok(Value::Number(n)),
            Some(Tok::Name(n)) => Ok(match n.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "null" => Value::Null,
                // bare enum value (ASC / DESC / EMAIL …) → string
                _ => Value::String(n),
            }),
            Some(Tok::LBrace) => {
                let mut obj = Map::new();
                while self.peek() != Some(&Tok::RBrace) {
                    let key = match self.next() {
                        Some(Tok::Name(n)) => n,
                        other => return Err(format!("expected object key, got {:?}", other)),
                    };
                    self.expect(&Tok::Colon)?;
                    let v = self.parse_value()?;
                    obj.insert(key, v);
                }
                self.expect(&Tok::RBrace)?;
                Ok(Value::Object(obj))
            }
            Some(Tok::LBracket) => {
                let mut arr = Vec::new();
                while self.peek() != Some(&Tok::RBracket) {
                    arr.push(self.parse_value()?);
                }
                self.expect(&Tok::RBracket)?;
                Ok(Value::Array(arr))
            }
            other => Err(format!("expected value, got {:?}", other)),
        }
    }
}

fn parse_document(src: &str) -> Result<Vec<Field>, String> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    // Skip an optional `query`/`mutation` keyword + operation name + variable
    // definitions, landing on the root selection set.
    while let Some(Tok::Name(_)) = p.peek() {
        p.next();
        if p.peek() == Some(&Tok::LParen) {
            // skip balanced variable-definition group
            let mut depth = 0;
            loop {
                match p.next() {
                    Some(Tok::LParen) => depth += 1,
                    Some(Tok::RParen) => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    None => return Err("unbalanced ( in operation header".into()),
                    _ => {}
                }
            }
        }
    }
    p.parse_selection_set()
}

/// Snake-case a (possibly camelCase) identifier.
fn to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn people() -> ObjectData {
        ObjectData::new(
            "person",
            "people",
            vec![
                json!({"id": "11111111-1111-1111-1111-111111111111", "first_name": "Ada", "last_name": "Lovelace", "position": 1}),
                json!({"id": "22222222-2222-2222-2222-222222222222", "first_name": "Bob", "last_name": "Smith", "position": 0}),
                json!({"id": "33333333-3333-3333-3333-333333333333", "first_name": "Cleo", "last_name": "Vance", "position": 2}),
            ],
        )
    }

    fn resolver() -> GraphQlResolver {
        GraphQlResolver::new(vec![people()])
    }

    #[test]
    fn parse_rejects_unknown_root_field() {
        let r = resolver();
        let out = r.execute("{ widgets { edges { node { id } } } }");
        // Unknown root field → GraphQL `errors` array, `data` null for it.
        assert!(out["errors"].is_array());
    }

    #[test]
    fn find_many_wraps_connection_envelope() {
        let r = resolver();
        let out = r.execute("{ people { edges { node { id firstName } cursor } totalCount } }");
        let conn = &out["data"]["people"];
        assert_eq!(conn["totalCount"], 3);
        assert!(conn["edges"].is_array());
        assert_eq!(conn["edges"].as_array().unwrap().len(), 3);
        // Projection is camelCase and only the selected columns.
        let node0 = &conn["edges"][0]["node"];
        assert!(node0["id"].is_string());
        assert!(node0["firstName"].is_string());
        assert!(node0["lastName"].is_null()); // not selected → absent/null
        // Every edge carries an opaque cursor.
        assert!(conn["edges"][0]["cursor"].is_string());
    }

    #[test]
    fn find_many_orders_ascending_by_field() {
        let r = resolver();
        let out = r.execute("{ people(orderBy: {position: ASC}) { edges { node { firstName } } } }");
        let edges = out["data"]["people"]["edges"].as_array().unwrap();
        assert_eq!(edges[0]["node"]["firstName"], "Bob"); // position 0
        assert_eq!(edges[1]["node"]["firstName"], "Ada"); // position 1
        assert_eq!(edges[2]["node"]["firstName"], "Cleo"); // position 2
    }

    #[test]
    fn find_many_orders_descending_by_field() {
        let r = resolver();
        let out = r.execute("{ people(orderBy: {position: DESC}) { edges { node { firstName } } } }");
        let edges = out["data"]["people"]["edges"].as_array().unwrap();
        assert_eq!(edges[0]["node"]["firstName"], "Cleo");
        assert_eq!(edges[2]["node"]["firstName"], "Bob");
    }

    #[test]
    fn find_many_filters_by_field_equality() {
        let r = resolver();
        let out = r.execute("{ people(filter: {firstName: \"Ada\"}) { edges { node { id } } totalCount } }");
        assert_eq!(out["data"]["people"]["totalCount"], 1);
    }

    #[test]
    fn find_many_filters_with_ilike_operator() {
        let r = resolver();
        let out = r.execute("{ people(filter: {lastName: {ilike: \"va\"}}) { totalCount } }");
        // "Vance" matches case-insensitively.
        assert_eq!(out["data"]["people"]["totalCount"], 1);
    }

    #[test]
    fn find_many_paginates_first_and_sets_has_next_page() {
        let r = resolver();
        let out = r.execute("{ people(first: 2, orderBy: {position: ASC}) { edges { node { firstName } } pageInfo { hasNextPage hasPreviousPage endCursor } } }");
        let conn = &out["data"]["people"];
        assert_eq!(conn["edges"].as_array().unwrap().len(), 2);
        assert_eq!(conn["pageInfo"]["hasNextPage"], true);
        assert_eq!(conn["pageInfo"]["hasPreviousPage"], false);
        assert!(conn["pageInfo"]["endCursor"].is_string());
    }

    #[test]
    fn find_one_returns_unwrapped_record_by_id_filter() {
        let r = resolver();
        let out = r.execute(
            "{ person(filter: {id: \"22222222-2222-2222-2222-222222222222\"}) { id firstName } }",
        );
        let rec = &out["data"]["person"];
        assert_eq!(rec["firstName"], "Bob");
        // Single record — NOT wrapped in a Connection.
        assert!(rec.get("edges").is_none());
    }

    #[test]
    fn find_one_missing_record_is_null() {
        let r = resolver();
        let out =
            r.execute("{ person(filter: {id: \"00000000-0000-0000-0000-000000000000\"}) { id } }");
        assert!(out["data"]["person"].is_null());
    }

    #[test]
    fn multiple_root_fields_resolve_independently() {
        let r = GraphQlResolver::new(vec![
            people(),
            ObjectData::new(
                "company",
                "companies",
                vec![json!({"id": "aaaaaaaa-0000-0000-0000-000000000000", "name": "Acme"})],
            ),
        ]);
        let out = r.execute("{ people { totalCount } companies { totalCount } }");
        assert_eq!(out["data"]["people"]["totalCount"], 3);
        assert_eq!(out["data"]["companies"]["totalCount"], 1);
    }
}
