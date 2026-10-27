// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! TableProvider — DataFusion's storage adapter trait + impls.
//!
//! Upstream: `crates/datafusion-catalog/src/table.rs`
//!
//! A TableProvider exposes its schema + scan capability so the engine
//! can build a TableScan node. The MVP ships `MemTable` (in-memory
//! Vec<Row>) and `CsvSource` (string-parsed at construction time, one
//! row per CSV record). A Parquet impl + an Iceberg adapter live in
//! lakehouse-ray-2.

use crate::error::{Error, Result};
use crate::row::{Row, Value};
use crate::schema::{DataType, Field, SchemaRef, TableSchema};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait TableProvider: Send + Sync {
    fn schema(&self) -> SchemaRef;
    async fn scan(&self) -> Result<Vec<Row>>;
}

#[derive(Debug, Clone)]
pub struct MemTable {
    pub schema: SchemaRef,
    pub rows: Vec<Row>,
}

impl MemTable {
    pub fn new(schema: SchemaRef, rows: Vec<Row>) -> Result<Self> {
        for r in &rows {
            if r.len() != schema.fields.len() {
                return Err(Error::Schema(format!(
                    "row arity {} doesn't match schema arity {}",
                    r.len(),
                    schema.fields.len()
                )));
            }
        }
        Ok(Self { schema, rows })
    }
}

#[async_trait]
impl TableProvider for MemTable {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    async fn scan(&self) -> Result<Vec<Row>> {
        Ok(self.rows.clone())
    }
}

/// CSV data source — parses a string at construction time.
#[derive(Debug, Clone)]
pub struct CsvSource {
    schema: SchemaRef,
    rows: Vec<Row>,
}

impl CsvSource {
    /// Parse CSV from a string. The first row is the header. Field
    /// types are taken from `schema`; cells that fail to coerce
    /// become NULL.
    pub fn from_str(schema: SchemaRef, csv: &str) -> Result<Self> {
        let mut lines = csv.lines();
        let header = lines.next().ok_or_else(|| Error::Io("empty csv".into()))?;
        let header_cols: Vec<&str> = header.split(',').map(str::trim).collect();
        if header_cols.len() != schema.fields.len() {
            return Err(Error::Schema(format!(
                "csv header arity {} != schema arity {}",
                header_cols.len(),
                schema.fields.len()
            )));
        }
        // Verify columns align by name.
        for (i, c) in header_cols.iter().enumerate() {
            if *c != schema.fields[i].name {
                return Err(Error::Schema(format!(
                    "csv column #{i} '{}' != schema '{}'",
                    c, schema.fields[i].name
                )));
            }
        }

        let mut rows = Vec::new();
        for (line_no, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let cells: Vec<&str> = line.split(',').map(str::trim).collect();
            if cells.len() != schema.fields.len() {
                return Err(Error::Schema(format!(
                    "row {} arity {} != schema arity {}",
                    line_no + 2,
                    cells.len(),
                    schema.fields.len()
                )));
            }
            let mut vals = Vec::with_capacity(cells.len());
            for (cell, field) in cells.iter().zip(schema.fields.iter()) {
                vals.push(parse_cell(cell, field));
            }
            rows.push(Row::new(vals));
        }
        Ok(Self { schema, rows })
    }
}

fn parse_cell(cell: &str, field: &Field) -> Value {
    if cell.is_empty() {
        return Value::Null;
    }
    match field.data_type {
        DataType::Boolean => match cell.to_ascii_lowercase().as_str() {
            "t" | "true" | "1" | "yes" => Value::Bool(true),
            "f" | "false" | "0" | "no" => Value::Bool(false),
            _ => Value::Null,
        },
        DataType::Int32 => cell.parse::<i32>().map(Value::Int32).unwrap_or(Value::Null),
        DataType::Int64 => cell.parse::<i64>().map(Value::Int64).unwrap_or(Value::Null),
        DataType::Float32 | DataType::Float64 => cell
            .parse::<f64>()
            .map(Value::Float64)
            .unwrap_or(Value::Null),
        DataType::Utf8 => Value::Utf8(cell.to_string()),
        _ => Value::Utf8(cell.to_string()),
    }
}

#[async_trait]
impl TableProvider for CsvSource {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    async fn scan(&self) -> Result<Vec<Row>> {
        Ok(self.rows.clone())
    }
}

/// Newline-delimited JSON (NDJSON) data source — one JSON object per line.
///
/// Upstream: `crates/datafusion/src/datasource/file_format/json.rs`
/// (`JsonFormat` / `JsonOpener`). DataFusion's reader streams a
/// line-delimited JSON file into Arrow `RecordBatch`es. The cave-datafusion
/// MVP carries the same shape over the row model: each line is parsed into
/// a flat object, fields bind to schema columns *by name* (so key order in
/// the source is irrelevant), absent keys become NULL, and each value is
/// coerced to the declared column type — un-coercible values fall to NULL,
/// matching `CsvSource`'s bad-cell semantics. Nested objects/arrays are
/// outside the flat row model and coerce to NULL.
///
/// The JSON itself is parsed by a small hand-rolled scanner (the crate is
/// deliberately dependency-light, like `CsvSource`'s hand-rolled CSV
/// splitter); it covers the JSON value grammar for flat objects: strings
/// (with `\" \\ \/ \n \r \t \b \f \uXXXX` escapes), numbers, booleans, and
/// null.
#[derive(Debug, Clone)]
pub struct JsonSource {
    schema: SchemaRef,
    rows: Vec<Row>,
}

impl JsonSource {
    /// Parse NDJSON from a string against `schema`. Blank lines are skipped.
    pub fn from_str(schema: SchemaRef, ndjson: &str) -> Result<Self> {
        let mut rows = Vec::new();
        for (line_no, line) in ndjson.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let obj = parse_json_object(line).map_err(|e| {
                Error::Io(format!("json line {}: {}", line_no + 1, e))
            })?;
            let mut vals = Vec::with_capacity(schema.fields.len());
            for field in &schema.fields {
                let scalar = obj.iter().find(|(k, _)| k == &field.name).map(|(_, v)| v);
                vals.push(match scalar {
                    Some(s) => coerce_json(s, field),
                    None => Value::Null,
                });
            }
            rows.push(Row::new(vals));
        }
        Ok(Self { schema, rows })
    }
}

#[async_trait]
impl TableProvider for JsonSource {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    async fn scan(&self) -> Result<Vec<Row>> {
        Ok(self.rows.clone())
    }
}

/// A parsed scalar JSON value (the flat-object subset the row model holds).
#[derive(Debug, Clone, PartialEq)]
enum JsonScalar {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A nested object/array — unsupported in the flat row model.
    Unsupported,
}

/// Coerce a parsed JSON scalar into the declared column type. Un-coercible
/// combinations fall to NULL (mirrors `CsvSource`'s bad-cell behavior).
fn coerce_json(s: &JsonScalar, field: &Field) -> Value {
    match s {
        JsonScalar::Null | JsonScalar::Unsupported => Value::Null,
        _ => match field.data_type {
            DataType::Boolean => match s {
                JsonScalar::Bool(b) => Value::Bool(*b),
                JsonScalar::Str(t) => match t.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => Value::Bool(true),
                    "false" | "0" | "no" => Value::Bool(false),
                    _ => Value::Null,
                },
                JsonScalar::Int(i) => Value::Bool(*i != 0),
                _ => Value::Null,
            },
            DataType::Int32 => match s {
                JsonScalar::Int(i) => Value::Int32(*i as i32),
                JsonScalar::Str(t) => t.parse::<i32>().map(Value::Int32).unwrap_or(Value::Null),
                _ => Value::Null,
            },
            DataType::Int64 => match s {
                JsonScalar::Int(i) => Value::Int64(*i),
                JsonScalar::Str(t) => t.parse::<i64>().map(Value::Int64).unwrap_or(Value::Null),
                _ => Value::Null,
            },
            DataType::Float32 | DataType::Float64 => match s {
                JsonScalar::Float(f) => Value::Float64(*f),
                JsonScalar::Int(i) => Value::Float64(*i as f64),
                JsonScalar::Str(t) => t.parse::<f64>().map(Value::Float64).unwrap_or(Value::Null),
                _ => Value::Null,
            },
            DataType::Utf8 => match s {
                JsonScalar::Str(t) => Value::Utf8(t.clone()),
                JsonScalar::Int(i) => Value::Utf8(i.to_string()),
                JsonScalar::Float(f) => Value::Utf8(f.to_string()),
                JsonScalar::Bool(b) => Value::Utf8(b.to_string()),
                _ => Value::Null,
            },
            _ => Value::Null,
        },
    }
}

/// Parse a single flat JSON object `{ "k": v, ... }` into key/scalar pairs.
fn parse_json_object(line: &str) -> std::result::Result<Vec<(String, JsonScalar)>, String> {
    let bytes: Vec<char> = line.chars().collect();
    let mut p = JsonParser { chars: &bytes, pos: 0 };
    p.skip_ws();
    p.expect('{')?;
    let mut out = Vec::new();
    p.skip_ws();
    if p.peek() == Some('}') {
        p.pos += 1;
        return Ok(out);
    }
    loop {
        p.skip_ws();
        let key = p.parse_string()?;
        p.skip_ws();
        p.expect(':')?;
        p.skip_ws();
        let val = p.parse_value()?;
        out.push((key, val));
        p.skip_ws();
        match p.peek() {
            Some(',') => {
                p.pos += 1;
                continue;
            }
            Some('}') => {
                p.pos += 1;
                break;
            }
            other => return Err(format!("expected ',' or '}}', got {:?}", other)),
        }
    }
    Ok(out)
}

struct JsonParser<'a> {
    chars: &'a [char],
    pos: usize,
}

impl JsonParser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, c: char) -> std::result::Result<(), String> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected {:?}, got {:?}", c, self.peek()))
        }
    }

    fn parse_value(&mut self) -> std::result::Result<JsonScalar, String> {
        match self.peek() {
            Some('"') => Ok(JsonScalar::Str(self.parse_string()?)),
            Some('{') | Some('[') => {
                self.skip_nested()?;
                Ok(JsonScalar::Unsupported)
            }
            Some('t') | Some('f') => self.parse_bool(),
            Some('n') => self.parse_null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            other => Err(format!("unexpected value start {:?}", other)),
        }
    }

    fn parse_string(&mut self) -> std::result::Result<String, String> {
        self.expect('"')?;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some('"') => {
                    self.pos += 1;
                    return Ok(s);
                }
                Some('\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some('/') => s.push('/'),
                        Some('n') => s.push('\n'),
                        Some('r') => s.push('\r'),
                        Some('t') => s.push('\t'),
                        Some('b') => s.push('\u{0008}'),
                        Some('f') => s.push('\u{000C}'),
                        Some('u') => {
                            let mut code = 0u32;
                            for _ in 0..4 {
                                self.pos += 1;
                                let h = self.peek().ok_or("truncated \\u escape")?;
                                let d = h.to_digit(16).ok_or("invalid \\u hex digit")?;
                                code = code * 16 + d;
                            }
                            s.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        other => return Err(format!("invalid escape \\{:?}", other)),
                    }
                    self.pos += 1;
                }
                Some(c) => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_number(&mut self) -> std::result::Result<JsonScalar, String> {
        let start = self.pos;
        let mut is_float = false;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            match c {
                '0'..='9' => self.pos += 1,
                '.' | 'e' | 'E' | '+' | '-' => {
                    is_float = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if is_float {
            text.parse::<f64>()
                .map(JsonScalar::Float)
                .map_err(|e| format!("bad number {:?}: {}", text, e))
        } else {
            text.parse::<i64>()
                .map(JsonScalar::Int)
                .or_else(|_| text.parse::<f64>().map(JsonScalar::Float))
                .map_err(|e| format!("bad number {:?}: {}", text, e))
        }
    }

    fn parse_bool(&mut self) -> std::result::Result<JsonScalar, String> {
        if self.matches_keyword("true") {
            Ok(JsonScalar::Bool(true))
        } else if self.matches_keyword("false") {
            Ok(JsonScalar::Bool(false))
        } else {
            Err("invalid boolean literal".into())
        }
    }

    fn parse_null(&mut self) -> std::result::Result<JsonScalar, String> {
        if self.matches_keyword("null") {
            Ok(JsonScalar::Null)
        } else {
            Err("invalid null literal".into())
        }
    }

    fn matches_keyword(&mut self, kw: &str) -> bool {
        let end = self.pos + kw.len();
        if end <= self.chars.len() && self.chars[self.pos..end].iter().copied().eq(kw.chars()) {
            self.pos = end;
            true
        } else {
            false
        }
    }

    /// Skip a nested object/array (balanced braces/brackets), respecting
    /// strings so braces inside a string don't unbalance the count.
    fn skip_nested(&mut self) -> std::result::Result<(), String> {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => return Err("unterminated nested value".into()),
                Some('"') => {
                    let _ = self.parse_string()?;
                }
                Some(c @ ('{' | '[')) => {
                    let _ = c;
                    depth += 1;
                    self.pos += 1;
                }
                Some(c @ ('}' | ']')) => {
                    let _ = c;
                    depth -= 1;
                    self.pos += 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Some(_) => self.pos += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> SchemaRef {
        Arc::new(TableSchema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, true),
        ]))
    }

    #[tokio::test]
    async fn mem_table_scan_returns_rows() {
        let rows = vec![Row::new(vec![Value::Int64(1), Value::Utf8("x".into())])];
        let t = MemTable::new(schema(), rows.clone()).unwrap();
        assert_eq!(t.scan().await.unwrap(), rows);
    }

    #[tokio::test]
    async fn mem_table_rejects_arity_mismatch() {
        let bad = vec![Row::new(vec![Value::Int64(1)])];
        let r = MemTable::new(schema(), bad);
        assert!(matches!(r, Err(Error::Schema(_))));
    }

    #[tokio::test]
    async fn csv_round_trip() {
        let csv = "a,b\n1,hello\n2,\n";
        let t = CsvSource::from_str(schema(), csv).unwrap();
        let rows = t.scan().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].values,
            vec![Value::Int64(1), Value::Utf8("hello".into())]
        );
        assert_eq!(rows[1].values, vec![Value::Int64(2), Value::Null]);
    }

    #[test]
    fn csv_header_mismatch_errors() {
        let r = CsvSource::from_str(schema(), "x,y\n");
        assert!(matches!(r, Err(Error::Schema(_))));
    }

    #[test]
    fn csv_bad_cell_becomes_null() {
        let csv = "a,b\nnot-a-number,hi\n";
        let t = CsvSource::from_str(schema(), csv).unwrap();
        assert_eq!(t.rows[0].values[0], Value::Null);
    }
}
