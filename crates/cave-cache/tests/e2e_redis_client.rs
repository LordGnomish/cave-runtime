//! End-to-end smoke tests: real `redis-rs` client against the in-process
//! cave-cache server. Verifies the RESP TCP wire path is fully wired and the
//! command dispatcher returns expected values.

use cave_cache::db::ServerState;
use cave_cache::server::bind;

async fn spawn_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let state = ServerState::new(cave_cache::config::Config::default());
    bind(state, "127.0.0.1:0").await.unwrap()
}

async fn client(addr: std::net::SocketAddr) -> redis::aio::MultiplexedConnection {
    let url = format!("redis://{}", addr);
    let client = redis::Client::open(url).unwrap();
    client.get_multiplexed_async_connection().await.unwrap()
}

#[tokio::test]
async fn ping_returns_pong() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let pong: String = redis::cmd("PING").query_async(&mut conn).await.unwrap();
    assert_eq!(pong, "PONG");
}

#[tokio::test]
async fn set_then_get_returns_value() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET")
        .arg("k1")
        .arg("hello")
        .query_async(&mut conn)
        .await
        .unwrap();
    let v: String = redis::cmd("GET").arg("k1").query_async(&mut conn).await.unwrap();
    assert_eq!(v, "hello");
}

#[tokio::test]
async fn get_missing_returns_nil() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let v: Option<String> = redis::cmd("GET").arg("missing").query_async(&mut conn).await.unwrap();
    assert_eq!(v, None);
}

#[tokio::test]
async fn incr_and_decr_round_trip() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("c").arg("10").query_async(&mut conn).await.unwrap();
    let n1: i64 = redis::cmd("INCR").arg("c").query_async(&mut conn).await.unwrap();
    let n2: i64 = redis::cmd("DECR").arg("c").query_async(&mut conn).await.unwrap();
    assert_eq!(n1, 11);
    assert_eq!(n2, 10);
}

#[tokio::test]
async fn del_returns_count() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("a").arg("1").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("SET").arg("b").arg("2").query_async(&mut conn).await.unwrap();
    let n: i64 = redis::cmd("DEL").arg("a").arg("b").arg("c").query_async(&mut conn).await.unwrap();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn exists_returns_count() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("k").arg("v").query_async(&mut conn).await.unwrap();
    let n: i64 = redis::cmd("EXISTS").arg("k").arg("nope").query_async(&mut conn).await.unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn hash_set_then_get() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("HSET")
        .arg("h").arg("f1").arg("v1")
        .query_async(&mut conn).await.unwrap();
    let v: String = redis::cmd("HGET").arg("h").arg("f1").query_async(&mut conn).await.unwrap();
    assert_eq!(v, "v1");
}

#[tokio::test]
async fn list_lpush_lrange() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("LPUSH").arg("l").arg("c").arg("b").arg("a").query_async(&mut conn).await.unwrap();
    let v: Vec<String> = redis::cmd("LRANGE").arg("l").arg(0).arg(-1).query_async(&mut conn).await.unwrap();
    assert_eq!(v, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn set_sadd_smembers() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SADD").arg("s").arg("a").arg("b").arg("c").query_async(&mut conn).await.unwrap();
    let mut members: Vec<String> = redis::cmd("SMEMBERS").arg("s").query_async(&mut conn).await.unwrap();
    members.sort();
    assert_eq!(members, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn zset_zadd_zrange() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("ZADD").arg("z").arg(1).arg("a").arg(2).arg("b").arg(3).arg("c").query_async(&mut conn).await.unwrap();
    let v: Vec<String> = redis::cmd("ZRANGE").arg("z").arg(0).arg(-1).query_async(&mut conn).await.unwrap();
    assert_eq!(v, vec!["a", "b", "c"]);
}

#[tokio::test]
async fn expire_then_ttl() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("k").arg("v").query_async(&mut conn).await.unwrap();
    let _: i64 = redis::cmd("EXPIRE").arg("k").arg(100).query_async(&mut conn).await.unwrap();
    let ttl: i64 = redis::cmd("TTL").arg("k").query_async(&mut conn).await.unwrap();
    assert!(ttl > 0 && ttl <= 100);
}

#[tokio::test]
async fn type_command_reports_kinds() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("s").arg("v").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("LPUSH").arg("l").arg("x").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("HSET").arg("h").arg("f").arg("v").query_async(&mut conn).await.unwrap();
    let s: String = redis::cmd("TYPE").arg("s").query_async(&mut conn).await.unwrap();
    let l: String = redis::cmd("TYPE").arg("l").query_async(&mut conn).await.unwrap();
    let h: String = redis::cmd("TYPE").arg("h").query_async(&mut conn).await.unwrap();
    assert_eq!(s, "string");
    assert_eq!(l, "list");
    assert_eq!(h, "hash");
}

#[tokio::test]
async fn select_db_switches() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("k").arg("db0").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("SELECT").arg(1).query_async(&mut conn).await.unwrap();
    let v: Option<String> = redis::cmd("GET").arg("k").query_async(&mut conn).await.unwrap();
    assert_eq!(v, None);
    let _: () = redis::cmd("SELECT").arg(0).query_async(&mut conn).await.unwrap();
    let v: String = redis::cmd("GET").arg("k").query_async(&mut conn).await.unwrap();
    assert_eq!(v, "db0");
}

#[tokio::test]
async fn dbsize_counts_keys() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("a").arg("1").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("SET").arg("b").arg("2").query_async(&mut conn).await.unwrap();
    let n: i64 = redis::cmd("DBSIZE").query_async(&mut conn).await.unwrap();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn flushdb_clears() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("a").arg("1").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await.unwrap();
    let n: i64 = redis::cmd("DBSIZE").query_async(&mut conn).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn append_extends_value() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("k").arg("hello").query_async(&mut conn).await.unwrap();
    let len: i64 = redis::cmd("APPEND").arg("k").arg(" world").query_async(&mut conn).await.unwrap();
    assert_eq!(len, 11);
    let v: String = redis::cmd("GET").arg("k").query_async(&mut conn).await.unwrap();
    assert_eq!(v, "hello world");
}

#[tokio::test]
async fn strlen_returns_byte_count() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("k").arg("héllo").query_async(&mut conn).await.unwrap();
    let n: i64 = redis::cmd("STRLEN").arg("k").query_async(&mut conn).await.unwrap();
    assert_eq!(n, 6); // 'é' is 2 UTF-8 bytes
}

#[tokio::test]
async fn keys_with_pattern_matches() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("user:1").arg("alice").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("SET").arg("user:2").arg("bob").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("SET").arg("other").arg("x").query_async(&mut conn).await.unwrap();
    let mut keys: Vec<String> = redis::cmd("KEYS").arg("user:*").query_async(&mut conn).await.unwrap();
    keys.sort();
    assert_eq!(keys, vec!["user:1", "user:2"]);
}

#[tokio::test]
async fn rename_moves_key() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("SET").arg("k1").arg("v").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("RENAME").arg("k1").arg("k2").query_async(&mut conn).await.unwrap();
    let v: String = redis::cmd("GET").arg("k2").query_async(&mut conn).await.unwrap();
    assert_eq!(v, "v");
    let v1: Option<String> = redis::cmd("GET").arg("k1").query_async(&mut conn).await.unwrap();
    assert_eq!(v1, None);
}

#[tokio::test]
async fn mset_mget_round_trip() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let _: () = redis::cmd("MSET").arg("a").arg("1").arg("b").arg("2").arg("c").arg("3").query_async(&mut conn).await.unwrap();
    let v: Vec<Option<String>> = redis::cmd("MGET").arg("a").arg("b").arg("missing").arg("c").query_async(&mut conn).await.unwrap();
    assert_eq!(v, vec![Some("1".into()), Some("2".into()), None, Some("3".into())]);
}

#[tokio::test]
async fn echo_returns_input() {
    let (addr, _h) = spawn_server().await;
    let mut conn = client(addr).await;
    let v: String = redis::cmd("ECHO").arg("hello").query_async(&mut conn).await.unwrap();
    assert_eq!(v, "hello");
}
