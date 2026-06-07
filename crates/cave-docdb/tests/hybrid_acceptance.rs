// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Acceptance tests for the FerretDB-style hybrid strategy.
//!
//! Covers the headline acceptance criteria:
//!   * MongoDB wire-protocol roundtrip (driver byte compatibility), incl.
//!     legacy OP_INSERT decode → SQL translation.
//!   * aggregation pipeline → PostgreSQL SQL conversion.
//!   * GridFS chunked upload/download roundtrip.
//!   * end-to-end document CRUD through the pluggable StorageBackend, and SQL
//!     emission through the SqlBackend executor boundary.

use cave_docdb::backend::{ExecOutcome, MemoryBackend, RecordingExecutor, SqlBackend, StorageBackend};
use cave_docdb::bson::{self, Document};
use cave_docdb::gridfs::GridFs;
use cave_docdb::oplog::{OpLog, OpType};
use cave_docdb::sql;
use cave_docdb::wire::{self, LegacyOp, OpMsg, OP_INSERT};
use serde_json::{json, Value};

fn doc(pairs: &[(&str, Value)]) -> Document {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// A modern OP_MSG `insert` command survives an encode→decode roundtrip with
/// the body intact — the byte contract a real MongoDB driver relies on.
#[test]
fn op_msg_command_roundtrip() {
    let mut cmd = Document::new();
    cmd.insert("insert".into(), json!("users"));
    cmd.insert("$db".into(), json!("test"));
    cmd.insert("ordered".into(), json!(true));

    let msg = OpMsg::new(cmd.clone());
    let bytes = wire::encode_op_msg(&msg, 11, 0).unwrap();
    let (req_id, decoded) = wire::decode_op_msg(&bytes).unwrap();

    assert_eq!(req_id, 11);
    let body = decoded.body().expect("body");
    assert_eq!(body.get("insert"), Some(&json!("users")));
    assert_eq!(body.get("$db"), Some(&json!("test")));
    assert_eq!(body.get("ordered"), Some(&json!(true)));
}

/// A legacy OP_INSERT frame from an old driver decodes and translates straight
/// into the hybrid SQL path — wire compatibility meets the SQL backend.
#[test]
fn legacy_op_insert_to_sql() {
    let d1 = bson::encode_doc(&doc(&[("_id", json!("1")), ("n", json!(7))])).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(&0i32.to_le_bytes()); // flags
    body.extend_from_slice(b"test.users\0");
    body.extend_from_slice(&d1);

    let mut frame = Vec::new();
    let len = (16 + body.len()) as u32;
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&1i32.to_le_bytes());
    frame.extend_from_slice(&0i32.to_le_bytes());
    frame.extend_from_slice(&OP_INSERT.to_le_bytes());
    frame.extend_from_slice(&body);

    let LegacyOp::Insert(ins) = wire::decode_legacy(&frame).unwrap() else {
        panic!("expected OP_INSERT");
    };
    assert_eq!(ins.collection, "test.users");
    assert_eq!(ins.documents.len(), 1);

    // Translate the decoded document into the hybrid INSERT statement.
    let doc_value = Value::Object(ins.documents[0].clone().into_iter().collect());
    let q = sql::insert_to_sql("users", &doc_value);
    assert_eq!(q.sql, "INSERT INTO \"users\" (_jsonb) VALUES ($1::jsonb)");
    assert_eq!(q.params, vec!["{\"_id\":\"1\",\"n\":7}"]);
}

/// A multi-stage aggregation pipeline converts to a single SQL statement.
#[test]
fn aggregation_pipeline_to_sql() {
    let pipeline = vec![
        json!({"$match": {"status": "active"}}),
        json!({"$group": {"_id": "$dept", "total": {"$sum": "$salary"}, "headcount": {"$sum": 1}}}),
    ];
    let q = sql::pipeline_to_sql("emp", &pipeline).expect("translatable");
    assert_eq!(
        q.sql,
        "SELECT jsonb_build_object('_id', _jsonb -> 'dept', \
'headcount', to_jsonb(count(*)), \
'total', to_jsonb(sum((_jsonb ->> 'salary')::numeric))) AS _jsonb \
FROM \"emp\" WHERE _jsonb -> 'status' = $1::jsonb GROUP BY _jsonb -> 'dept'"
    );
    assert_eq!(q.params, vec!["\"active\""]);
}

/// GridFS round-trips arbitrary bytes across multiple chunks.
#[test]
fn gridfs_chunked_upload_download_roundtrip() {
    let mut fs = GridFs::new();
    fs.chunk_size = Some(4); // force several chunks
    let payload: Vec<u8> = (0u8..=200).collect();

    let file = fs.upload("blob.bin", &payload, Some("application/octet-stream".into())).unwrap();
    assert_eq!(file.length, payload.len() as u64);
    assert_eq!(file.chunk_size, 4);

    let got = fs.download("blob.bin").unwrap();
    assert_eq!(got, payload);

    assert!(fs.delete("blob.bin"));
    assert!(fs.download("blob.bin").is_err());
}

/// End-to-end CRUD through the in-memory StorageBackend.
#[tokio::test]
async fn memory_backend_full_crud() {
    let be = MemoryBackend::new();
    for i in 0..5 {
        be.insert("c", json!({"_id": i.to_string(), "v": i})).await.unwrap();
    }
    assert_eq!(be.count("c", &json!({})).await.unwrap(), 5);

    let big = be.find("c", &json!({"v": {"$gte": 3}})).await.unwrap();
    assert_eq!(big.len(), 2);

    be.update("c", &json!({"v": {"$gte": 3}}), &json!({"$set": {"tag": "hi"}}))
        .await
        .unwrap();
    let tagged = be.find("c", &json!({"tag": "hi"})).await.unwrap();
    assert_eq!(tagged.len(), 2);

    let deleted = be.delete("c", &json!({"v": {"$lt": 2}})).await.unwrap();
    assert_eq!(deleted, 2);
    assert_eq!(be.count("c", &json!({})).await.unwrap(), 3);
}

/// SqlBackend emits correct SQL for each op through the executor boundary,
/// and the recorded statements prove the translation path is exercised.
#[tokio::test]
async fn sql_backend_emits_expected_statements() {
    let exec = RecordingExecutor::new();
    exec.push_outcome(ExecOutcome::Affected(1)); // insert
    exec.push_outcome(ExecOutcome::Rows(vec![json!({"_id": "1", "v": 9})])); // find
    let be = SqlBackend::new(exec);

    be.insert("c", json!({"_id": "1", "v": 9})).await.unwrap();
    let rows = be.find("c", &json!({"v": {"$gt": 5}})).await.unwrap();
    assert_eq!(rows, vec![json!({"_id": "1", "v": 9})]);

    assert_eq!(be.executor().call_count(), 2);
    assert_eq!(
        be.executor().last().unwrap().sql,
        "SELECT _jsonb FROM \"c\" WHERE _jsonb -> 'v' > $1::jsonb"
    );
}

/// Oplog replication converges a fresh secondary backend to a primary's state.
#[tokio::test]
async fn oplog_replication_end_to_end() {
    let primary = MemoryBackend::new();
    let log = OpLog::new();

    primary.insert("c", json!({"_id": "1", "n": 1})).await.unwrap();
    log.append(OpType::Insert, "test.c", json!({"_id": "1", "n": 1}), None, 1);
    primary.update("c", &json!({"_id": "1"}), &json!({"$set": {"n": 42}})).await.unwrap();
    log.append(OpType::Update, "test.c", json!({"$set": {"n": 42}}), Some(json!({"_id": "1"})), 2);

    let secondary = MemoryBackend::new();
    let n = cave_docdb::oplog::replicate(&secondary, &log.tail(None)).await.unwrap();
    assert_eq!(n, 2);
    let docs = secondary.find("c", &json!({})).await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["n"], json!(42));
}
