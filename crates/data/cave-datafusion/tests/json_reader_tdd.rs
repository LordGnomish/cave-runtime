// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD RED test for the newline-delimited JSON table provider —
//! `JsonSource`, the in-memory analogue of DataFusion's NDJSON reader.
//!
//! Upstream: `apache/datafusion` `datafusion/core/src/datasource/file_format/json.rs`
//! (the `JsonFormat`/`JsonOpener` line-delimited reader). One JSON object
//! per line; each object's fields bind to schema columns by name; absent
//! fields and un-coercible values become NULL; types are coerced to the
//! declared schema type — exactly mirroring how the existing `CsvSource`
//! handles a header row. Pins the provider + its row coercion before
//! `JsonSource` exists (RED).

use cave_datafusion::data_source::{JsonSource, TableProvider};
use cave_datafusion::row::{Row, Value};
use cave_datafusion::schema::{DataType, Field, SchemaRef, TableSchema};
use cave_datafusion::SessionContext;
use std::sync::Arc;

fn schema_ab() -> SchemaRef {
    Arc::new(TableSchema::new(vec![
        Field::new("a", DataType::Int64, true),
        Field::new("b", DataType::Utf8, true),
    ]))
}

#[tokio::test]
async fn ndjson_round_trip_with_null() {
    let json = "{\"a\": 1, \"b\": \"hello\"}\n{\"a\": 2, \"b\": null}\n";
    let src = JsonSource::from_str(schema_ab(), json).unwrap();
    let rows = src.scan().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].values,
        vec![Value::Int64(1), Value::Utf8("hello".into())]
    );
    assert_eq!(rows[1].values, vec![Value::Int64(2), Value::Null]);
}

#[tokio::test]
async fn missing_field_becomes_null() {
    // `b` absent on the object → NULL for that column.
    let json = "{\"a\": 5}\n";
    let src = JsonSource::from_str(schema_ab(), json).unwrap();
    let rows = src.scan().await.unwrap();
    assert_eq!(rows[0].values, vec![Value::Int64(5), Value::Null]);
}

#[tokio::test]
async fn keys_bind_by_name_not_position() {
    // Object lists `b` before `a`; binding is by field name, so the row is
    // still [a, b] in schema order.
    let json = "{\"b\": \"x\", \"a\": 9}\n";
    let src = JsonSource::from_str(schema_ab(), json).unwrap();
    let rows = src.scan().await.unwrap();
    assert_eq!(rows[0].values, vec![Value::Int64(9), Value::Utf8("x".into())]);
}

#[tokio::test]
async fn numbers_coerce_to_float_schema() {
    let schema: SchemaRef = Arc::new(TableSchema::new(vec![Field::new(
        "c",
        DataType::Float64,
        false,
    )]));
    let json = "{\"c\": 3}\n{\"c\": 2.5}\n";
    let src = JsonSource::from_str(schema, json).unwrap();
    let rows = src.scan().await.unwrap();
    assert_eq!(rows[0].values, vec![Value::Float64(3.0)]);
    assert_eq!(rows[1].values, vec![Value::Float64(2.5)]);
}

#[tokio::test]
async fn uncoercible_value_becomes_null() {
    // A non-numeric string in an Int64 column coerces to NULL (mirrors
    // CsvSource's bad-cell → NULL behavior).
    let json = "{\"a\": \"not-a-number\", \"b\": \"z\"}\n";
    let src = JsonSource::from_str(schema_ab(), json).unwrap();
    let rows = src.scan().await.unwrap();
    assert_eq!(rows[0].values, vec![Value::Null, Value::Utf8("z".into())]);
}

#[tokio::test]
async fn json_source_drives_sql() {
    let ctx = SessionContext::new();
    let json = "{\"a\": 1, \"b\": \"x\"}\n{\"a\": 7, \"b\": \"y\"}\n";
    let src = JsonSource::from_str(schema_ab(), json).unwrap();
    ctx.register_table("t", Arc::new(src)).await.unwrap();
    let out = ctx.sql("SELECT a FROM t WHERE a > 1").await.unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].values[0], Value::Int64(7));
}

#[tokio::test]
async fn escaped_string_decodes() {
    let schema: SchemaRef = Arc::new(TableSchema::new(vec![Field::new(
        "s",
        DataType::Utf8,
        false,
    )]));
    let json = "{\"s\": \"a\\tb\\n\\\"q\\\"\"}\n";
    let src = JsonSource::from_str(schema, json).unwrap();
    let rows = src.scan().await.unwrap();
    assert_eq!(rows[0].values, vec![Value::Utf8("a\tb\n\"q\"".into())]);
}

#[allow(dead_code)]
fn _row_ctor_used(_: Row) {}
