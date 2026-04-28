//! End-to-end smoke tests: real `tokio_postgres` client against the in-process
//! cave-pg server. Verifies the TCP wire path is fully wired and the query
//! engine returns rows in the expected text format.

use std::sync::Arc;

use cave_pg::engine::SqlEngine;
use cave_pg::server::{PgServer, ServerConfig};

async fn spawn_server() -> (std::net::SocketAddr, cave_pg::server::BoundServer) {
    let engine = SqlEngine::new();
    let server = PgServer::new(
        engine,
        ServerConfig {
            bind_addr: "127.0.0.1:0".into(),
            ..Default::default()
        },
    );
    server.bind().await.unwrap()
}

async fn connect(
    addr: std::net::SocketAddr,
) -> tokio_postgres::Client {
    let conn_str = format!("host={} port={} user=cave dbname=cave", addr.ip(), addr.port());
    let (client, connection) = tokio_postgres::connect(&conn_str, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

#[tokio::test]
async fn select_one_returns_one() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    let rows = client.simple_query("SELECT 1").await.unwrap();
    let mut found = false;
    for row in rows {
        if let tokio_postgres::SimpleQueryMessage::Row(r) = row {
            assert_eq!(r.get(0).unwrap(), "1");
            found = true;
        }
    }
    assert!(found);
}

#[tokio::test]
async fn create_insert_select_round_trip() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client
        .simple_query("CREATE TABLE users (id INT, name TEXT)")
        .await
        .unwrap();
    client
        .simple_query("INSERT INTO users (id, name) VALUES (1, 'alice'), (2, 'bob')")
        .await
        .unwrap();
    let rows = client
        .simple_query("SELECT id, name FROM users ORDER BY id")
        .await
        .unwrap();
    let row_data: Vec<(String, String)> = rows
        .into_iter()
        .filter_map(|r| match r {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some((
                row.get(0).unwrap().to_string(),
                row.get(1).unwrap().to_string(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(row_data, vec![
        ("1".into(), "alice".into()),
        ("2".into(), "bob".into()),
    ]);
}

#[tokio::test]
async fn update_then_select_reflects_change() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client
        .simple_query("CREATE TABLE t (id INT, x INT)")
        .await
        .unwrap();
    client
        .simple_query("INSERT INTO t (id, x) VALUES (1, 10), (2, 20)")
        .await
        .unwrap();
    client
        .simple_query("UPDATE t SET x = 99 WHERE id = 1")
        .await
        .unwrap();
    let rows = client
        .simple_query("SELECT x FROM t WHERE id = 1")
        .await
        .unwrap();
    let mut got = None;
    for r in rows {
        if let tokio_postgres::SimpleQueryMessage::Row(row) = r {
            got = Some(row.get(0).unwrap().to_string());
        }
    }
    assert_eq!(got.as_deref(), Some("99"));
}

#[tokio::test]
async fn delete_removes_rows() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client
        .simple_query("CREATE TABLE t (id INT)")
        .await
        .unwrap();
    client
        .simple_query("INSERT INTO t VALUES (1), (2), (3)")
        .await
        .unwrap();
    client.simple_query("DELETE FROM t WHERE id = 2").await.unwrap();
    let rows = client.simple_query("SELECT id FROM t ORDER BY id").await.unwrap();
    let ids: Vec<String> = rows
        .into_iter()
        .filter_map(|r| match r {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(row.get(0).unwrap().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(ids, vec!["1", "3"]);
}

#[tokio::test]
async fn syntax_error_returned_as_error_response() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    let res = client.simple_query("NOT VALID SQL").await;
    assert!(res.is_err(), "expected ErrorResponse to surface");
}

#[tokio::test]
async fn select_unknown_table_errors() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    let res = client.simple_query("SELECT * FROM ghost").await;
    assert!(res.is_err());
}

#[tokio::test]
async fn limit_caps_returned_rows() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client.simple_query("CREATE TABLE t (id INT)").await.unwrap();
    for i in 1..=10 {
        client
            .simple_query(&format!("INSERT INTO t VALUES ({})", i))
            .await
            .unwrap();
    }
    let rows = client
        .simple_query("SELECT * FROM t ORDER BY id LIMIT 3")
        .await
        .unwrap();
    let count = rows
        .iter()
        .filter(|r| matches!(r, tokio_postgres::SimpleQueryMessage::Row(_)))
        .count();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn order_by_desc_returns_descending() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client.simple_query("CREATE TABLE t (id INT)").await.unwrap();
    client.simple_query("INSERT INTO t VALUES (1), (2), (3)").await.unwrap();
    let rows = client.simple_query("SELECT id FROM t ORDER BY id DESC").await.unwrap();
    let first_row = rows
        .iter()
        .find_map(|r| match r {
            tokio_postgres::SimpleQueryMessage::Row(row) => Some(row.get(0).unwrap().to_string()),
            _ => None,
        });
    assert_eq!(first_row.as_deref(), Some("3"));
}

#[tokio::test]
async fn drop_table_then_reuse_name() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client.simple_query("CREATE TABLE t (id INT)").await.unwrap();
    client.simple_query("DROP TABLE t").await.unwrap();
    client.simple_query("CREATE TABLE t (id INT, name TEXT)").await.unwrap();
    let rows = client.simple_query("SELECT * FROM t").await.unwrap();
    let any_row = rows.iter().any(|r| matches!(r, tokio_postgres::SimpleQueryMessage::Row(_)));
    assert!(!any_row);
}

#[tokio::test]
async fn multiple_statements_in_one_query() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    let rows = client
        .simple_query(
            "CREATE TABLE t (id INT); INSERT INTO t VALUES (1), (2); SELECT * FROM t ORDER BY id",
        )
        .await
        .unwrap();
    let row_count = rows
        .iter()
        .filter(|r| matches!(r, tokio_postgres::SimpleQueryMessage::Row(_)))
        .count();
    assert_eq!(row_count, 2);
}

#[tokio::test]
async fn select_with_where_filter() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    client.simple_query("CREATE TABLE t (a INT, b INT)").await.unwrap();
    client.simple_query("INSERT INTO t VALUES (1, 10), (2, 20), (3, 30)").await.unwrap();
    let rows = client.simple_query("SELECT a FROM t WHERE b > 15 ORDER BY a").await.unwrap();
    let ids: Vec<String> = rows.into_iter().filter_map(|r| match r {
        tokio_postgres::SimpleQueryMessage::Row(row) => Some(row.get(0).unwrap().to_string()),
        _ => None,
    }).collect();
    assert_eq!(ids, vec!["2", "3"]);
}

#[tokio::test]
async fn empty_query_returns_empty_response() {
    let (addr, _server) = spawn_server().await;
    let client = connect(addr).await;
    let rows = client.simple_query(";").await.unwrap();
    assert!(rows.is_empty() || rows.iter().all(|r| !matches!(r, tokio_postgres::SimpleQueryMessage::Row(_))));
}
