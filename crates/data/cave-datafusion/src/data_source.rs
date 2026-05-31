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

/// Newline-delimited JSON data source — one JSON object per line.
///
/// Upstream: `crates/datafusion/src/datasource/json.rs`
/// (`JsonFormat`/`JsonOpener`, the line-delimited reader). Each line is
/// a flat JSON object; its fields bind to schema columns *by name*
/// (object key order is irrelevant); absent fields and un-coercible
/// values become NULL; scalar values are coerced to the declared
/// schema type — exactly mirroring how `CsvSource` handles a row.
/// Dependency-free hand parser, in keeping with the rest of the MVP;
/// nested objects/arrays as column values are out of scope and coerce
/// to NULL.
#[derive(Debug, Clone)]
pub struct JsonSource {
    schema: SchemaRef,
    rows: Vec<Row>,
}

/// A parsed JSON scalar, kept type-tagged so coercion can honor the
/// declared schema type (a JSON `3` lands as `Float64(3.0)` in a float
/// column but `Int64(3)` in an int column).
#[derive(Debug, Clone)]
enum JScalar {
    Null,
    Bool(bool),
    /// Number kept as its raw lexeme so int/float coercion is exact.
    Num(String),
    /// String with escapes already decoded.
    Str(String),
}

impl JsonSource {
    /// Parse newline-delimited JSON from a string. Blank lines are
    /// skipped. Each non-blank line must be a JSON object.
    pub fn from_str(schema: SchemaRef, json: &str) -> Result<Self> {
        let mut rows = Vec::new();
        for (line_no, line) in json.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let obj = parse_json_object(line)
                .map_err(|e| Error::Io(format!("json line {}: {e}", line_no + 1)))?;
            let mut vals = Vec::with_capacity(schema.fields.len());
            for field in &schema.fields {
                let v = obj
                    .iter()
                    .find(|(k, _)| k == &field.name)
                    .map(|(_, s)| coerce_json(s, field))
                    .unwrap_or(Value::Null);
                vals.push(v);
            }
            rows.push(Row::new(vals));
        }
        Ok(Self { schema, rows })
    }
}

/// Coerce a parsed JSON scalar to a column's declared type. A JSON
/// `null`, an absent field, or any value that cannot be represented as
/// the target type becomes `Value::Null` (mirrors `CsvSource`'s
/// bad-cell → NULL rule).
fn coerce_json(s: &JScalar, field: &Field) -> Value {
    if let JScalar::Null = s {
        return Value::Null;
    }
    match field.data_type {
        DataType::Boolean => match s {
            JScalar::Bool(b) => Value::Bool(*b),
            _ => Value::Null,
        },
        DataType::Int32 => num_text(s)
            .and_then(|t| t.parse::<i32>().ok())
            .map(Value::Int32)
            .unwrap_or(Value::Null),
        DataType::Int64 => num_text(s)
            .and_then(|t| t.parse::<i64>().ok())
            .map(Value::Int64)
            .unwrap_or(Value::Null),
        DataType::Float32 | DataType::Float64 => num_text(s)
            .and_then(|t| t.parse::<f64>().ok())
            .map(Value::Float64)
            .unwrap_or(Value::Null),
        DataType::Utf8 => match s {
            JScalar::Str(t) => Value::Utf8(t.clone()),
            JScalar::Num(t) => Value::Utf8(t.clone()),
            JScalar::Bool(b) => Value::Utf8(b.to_string()),
            JScalar::Null => Value::Null,
        },
        _ => Value::Null,
    }
}

/// The lexeme to feed numeric parsing for a scalar — the raw number
/// text, or a string's contents (so a quoted `"5"` still coerces).
fn num_text(s: &JScalar) -> Option<&str> {
    match s {
        JScalar::Num(t) | JScalar::Str(t) => Some(t),
        _ => None,
    }
}

/// Parse a single flat JSON object line into `(key, scalar)` pairs.
/// Nested object/array values are consumed but yield `JScalar::Null`.
fn parse_json_object(line: &str) -> std::result::Result<Vec<(String, JScalar)>, String> {
    let b = line.as_bytes();
    let mut p = JParser { b, i: 0 };
    p.skip_ws();
    p.expect(b'{')?;
    let mut out = Vec::new();
    p.skip_ws();
    if p.peek() == Some(b'}') {
        p.i += 1;
        return Ok(out);
    }
    loop {
        p.skip_ws();
        let key = p.parse_string()?;
        p.skip_ws();
        p.expect(b':')?;
        p.skip_ws();
        let val = p.parse_value()?;
        out.push((key, val));
        p.skip_ws();
        match p.peek() {
            Some(b',') => {
                p.i += 1;
                continue;
            }
            Some(b'}') => {
                p.i += 1;
                break;
            }
            _ => return Err("expected ',' or '}'".into()),
        }
    }
    Ok(out)
}

struct JParser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> JParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8) -> std::result::Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", c as char))
        }
    }

    fn parse_value(&mut self) -> std::result::Result<JScalar, String> {
        match self.peek() {
            Some(b'"') => Ok(JScalar::Str(self.parse_string()?)),
            Some(b'{') => {
                self.skip_structure(b'{', b'}')?;
                Ok(JScalar::Null)
            }
            Some(b'[') => {
                self.skip_structure(b'[', b']')?;
                Ok(JScalar::Null)
            }
            Some(b't') => {
                self.expect_word("true")?;
                Ok(JScalar::Bool(true))
            }
            Some(b'f') => {
                self.expect_word("false")?;
                Ok(JScalar::Bool(false))
            }
            Some(b'n') => {
                self.expect_word("null")?;
                Ok(JScalar::Null)
            }
            Some(c) if c == b'-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err("unexpected value".into()),
        }
    }

    fn expect_word(&mut self, w: &str) -> std::result::Result<(), String> {
        for &c in w.as_bytes() {
            if self.peek() != Some(c) {
                return Err(format!("expected '{w}'"));
            }
            self.i += 1;
        }
        Ok(())
    }

    fn parse_number(&mut self) -> std::result::Result<JScalar, String> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit()
                || c == b'-'
                || c == b'+'
                || c == b'.'
                || c == b'e'
                || c == b'E'
            {
                self.i += 1;
            } else {
                break;
            }
        }
        let raw = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| "invalid number".to_string())?;
        Ok(JScalar::Num(raw.to_string()))
    }

    /// Consume a balanced `{..}` or `[..]` (string-aware) and discard it.
    fn skip_structure(&mut self, open: u8, close: u8) -> std::result::Result<(), String> {
        self.expect(open)?;
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek() {
                Some(b'"') => {
                    self.parse_string()?;
                }
                Some(c) if c == open => {
                    depth += 1;
                    self.i += 1;
                }
                Some(c) if c == close => {
                    depth -= 1;
                    self.i += 1;
                }
                Some(_) => self.i += 1,
                None => return Err("unterminated structure".into()),
            }
        }
        Ok(())
    }

    fn parse_string(&mut self) -> std::result::Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000C}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let mut cp = 0u32;
                            for _ in 0..4 {
                                self.i += 1;
                                let h = self.peek().ok_or("bad \\u escape")?;
                                let d =
                                    (h as char).to_digit(16).ok_or("bad \\u hex digit")?;
                                cp = cp * 16 + d;
                            }
                            out.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err("bad escape".into()),
                    }
                    self.i += 1;
                }
                Some(_) => {
                    // Copy one UTF-8 char starting at self.i.
                    let rest = std::str::from_utf8(&self.b[self.i..])
                        .map_err(|_| "invalid utf-8".to_string())?;
                    let ch = rest.chars().next().unwrap();
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
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
