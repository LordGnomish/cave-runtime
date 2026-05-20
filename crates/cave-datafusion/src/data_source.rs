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
        let header = lines
            .next()
            .ok_or_else(|| Error::Io("empty csv".into()))?;
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
        DataType::Float32 | DataType::Float64 => {
            cell.parse::<f64>().map(Value::Float64).unwrap_or(Value::Null)
        }
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
        assert_eq!(rows[0].values, vec![Value::Int64(1), Value::Utf8("hello".into())]);
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
