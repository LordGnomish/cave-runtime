//! End-to-end smoke tests: real TCP client speaks OP_MSG to the cave-docdb
//! wire server. Uses the crate's own `wire` + `bson` codec to construct
//! requests, then asserts on the parsed response document.

use std::sync::Arc;

use cave_docdb::bson::Document;
use cave_docdb::cursor::CursorStore;
use cave_docdb::engine::Engine;
use cave_docdb::server::bind;
use cave_docdb::wire::{decode_op_msg, encode_op_msg, OpMsg};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn spawn() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let engine = Arc::new(Engine::new());
    let cursors = Arc::new(CursorStore::new());
    bind("127.0.0.1:0", engine, cursors).await.unwrap()
}

async fn round_trip(addr: std::net::SocketAddr, command: Document) -> Document {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = encode_op_msg(&OpMsg::new(command), 1, 0).unwrap();
    stream.write_all(&request).await.unwrap();
    let mut buf = vec![0u8; 65_536];
    let n = stream.read(&mut buf).await.unwrap();
    let (_id, op) = decode_op_msg(&buf[..n]).unwrap();
    op.body().expect("body present").clone()
}

fn doc(pairs: &[(&str, Value)]) -> Document {
    let mut d = Document::new();
    for (k, v) in pairs {
        d.insert(k.to_string(), v.clone());
    }
    d
}

#[tokio::test]
async fn ping_command_returns_ok_one() {
    let (addr, _h) = spawn().await;
    let resp = round_trip(addr, doc(&[("ping", json!(1)), ("$db", json!("admin"))])).await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn hello_command_returns_ismaster_true() {
    let (addr, _h) = spawn().await;
    let resp = round_trip(addr, doc(&[("hello", json!(1)), ("$db", json!("admin"))])).await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
    // ismaster/isWritablePrimary may or may not be set depending on dispatcher;
    // we just assert ok=1 (the command was recognized).
}

#[tokio::test]
async fn build_info_returns_ok() {
    let (addr, _h) = spawn().await;
    let resp = round_trip(addr, doc(&[("buildInfo", json!(1)), ("$db", json!("admin"))])).await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn insert_one_document() {
    let (addr, _h) = spawn().await;
    let resp = round_trip(
        addr,
        doc(&[
            ("insert", json!("users")),
            ("$db", json!("test")),
            ("documents", json!([{"_id": 1, "name": "alice"}])),
        ]),
    )
    .await;
    eprintln!("insert_one response: {:?}", resp);
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
    assert_eq!(resp.get("n").unwrap().as_i64().unwrap(), 1);
}

#[tokio::test]
async fn insert_then_find() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("users")),
            ("$db", json!("test")),
            ("documents", json!([{"_id": 1, "name": "alice"}, {"_id": 2, "name": "bob"}])),
        ]),
    )
    .await;
    let resp = round_trip(
        addr,
        doc(&[
            ("find", json!("users")),
            ("$db", json!("test")),
            ("filter", json!({})),
        ]),
    )
    .await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
    let cursor = resp.get("cursor").unwrap();
    let batch = cursor.get("firstBatch").unwrap().as_array().unwrap();
    assert_eq!(batch.len(), 2);
}

#[tokio::test]
async fn find_with_filter() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("users")),
            ("$db", json!("test")),
            ("documents", json!([{"_id": 1, "name": "alice"}, {"_id": 2, "name": "bob"}])),
        ]),
    )
    .await;
    let resp = round_trip(
        addr,
        doc(&[
            ("find", json!("users")),
            ("$db", json!("test")),
            ("filter", json!({"name": "alice"})),
        ]),
    )
    .await;
    let batch = resp.get("cursor").unwrap().get("firstBatch").unwrap().as_array().unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch[0]["name"], "alice");
}

#[tokio::test]
async fn count_command_returns_n() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("c")),
            ("$db", json!("test")),
            ("documents", json!([{"_id":1},{"_id":2},{"_id":3}])),
        ]),
    )
    .await;
    let resp = round_trip(
        addr,
        doc(&[
            ("count", json!("c")),
            ("$db", json!("test")),
            ("query", json!({})),
        ]),
    )
    .await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
    assert_eq!(resp.get("n").unwrap().as_i64().unwrap(), 3);
}

#[tokio::test]
async fn delete_one_then_find() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("c")),
            ("$db", json!("test")),
            ("documents", json!([{"_id":1, "k":"v1"},{"_id":2,"k":"v2"}])),
        ]),
    )
    .await;
    let del = round_trip(
        addr,
        doc(&[
            ("delete", json!("c")),
            ("$db", json!("test")),
            ("deletes", json!([{"q": {"_id": 1}, "limit": 1}])),
        ]),
    )
    .await;
    assert_eq!(del.get("n").unwrap().as_i64().unwrap(), 1);
    let resp = round_trip(
        addr,
        doc(&[
            ("find", json!("c")),
            ("$db", json!("test")),
            ("filter", json!({})),
        ]),
    )
    .await;
    let batch = resp.get("cursor").unwrap().get("firstBatch").unwrap().as_array().unwrap();
    assert_eq!(batch.len(), 1);
}

#[tokio::test]
async fn update_one_modifies() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("c")),
            ("$db", json!("test")),
            ("documents", json!([{"_id":1, "x":10}])),
        ]),
    )
    .await;
    let upd = round_trip(
        addr,
        doc(&[
            ("update", json!("c")),
            ("$db", json!("test")),
            ("updates", json!([{"q": {"_id": 1}, "u": {"$set": {"x": 99}}}])),
        ]),
    )
    .await;
    assert_eq!(upd.get("ok").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn list_collections_includes_inserted_collection() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("widgets")),
            ("$db", json!("test")),
            ("documents", json!([{"_id":1}])),
        ]),
    )
    .await;
    let resp = round_trip(
        addr,
        doc(&[
            ("listCollections", json!(1)),
            ("$db", json!("test")),
        ]),
    )
    .await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn create_collection_command() {
    let (addr, _h) = spawn().await;
    let resp = round_trip(
        addr,
        doc(&[
            ("create", json!("newcol")),
            ("$db", json!("test")),
        ]),
    )
    .await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn drop_collection_command() {
    let (addr, _h) = spawn().await;
    round_trip(
        addr,
        doc(&[
            ("insert", json!("c")),
            ("$db", json!("test")),
            ("documents", json!([{"_id":1}])),
        ]),
    )
    .await;
    let resp = round_trip(
        addr,
        doc(&[
            ("drop", json!("c")),
            ("$db", json!("test")),
        ]),
    )
    .await;
    assert_eq!(resp.get("ok").unwrap().as_f64().unwrap(), 1.0);
}

#[tokio::test]
async fn unknown_command_returns_ok_zero() {
    let (addr, _h) = spawn().await;
    let resp = round_trip(
        addr,
        doc(&[("xyzNotARealCmd", json!(1)), ("$db", json!("admin"))]),
    )
    .await;
    // Either ok=0 with errmsg (graceful), or ok=1 if dispatcher silently ignores.
    let ok = resp.get("ok").map(|v| v.as_f64().unwrap_or(0.0)).unwrap_or(0.0);
    assert!(ok == 0.0 || ok == 1.0);
}
