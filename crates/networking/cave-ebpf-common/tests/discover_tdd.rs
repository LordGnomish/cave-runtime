// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — protocol auto-detection (the heart of grafana/beyla's
// auto-instrumentation: pkg/internal/ebpf/common protocol classification +
// pkg/internal/request parsing). Detects HTTP/1.x, HTTP/2 preface, gRPC,
// and SQL (Postgres simple-query + MySQL COM_QUERY) straight from the raw
// first bytes of a connection, the way Beyla's userspace decoder does.

use cave_ebpf_common::discover::{
    classify_grpc, detect_protocol, parse_grpc_path, parse_http_request, parse_http_response,
    parse_mysql_query, parse_postgres_query, sql_command, HttpMethod, Protocol, SqlCommand,
};

// ── HTTP/1.x ────────────────────────────────────────────────────────────────

#[test]
fn test_detect_http_request() {
    let buf = b"GET /api/users?page=2 HTTP/1.1\r\nHost: x\r\n\r\n";
    assert_eq!(detect_protocol(buf), Protocol::Http);
    let req = parse_http_request(buf).unwrap();
    assert_eq!(req.method, HttpMethod::Get);
    assert_eq!(req.path, "/api/users?page=2");
    assert_eq!(req.version, "HTTP/1.1");
}

#[test]
fn test_parse_post_request() {
    let buf = b"POST /login HTTP/1.0\r\n\r\n";
    let req = parse_http_request(buf).unwrap();
    assert_eq!(req.method, HttpMethod::Post);
    assert_eq!(req.path, "/login");
    assert_eq!(req.version, "HTTP/1.0");
}

#[test]
fn test_detect_http_response_and_status() {
    let buf = b"HTTP/1.1 404 Not Found\r\n\r\n";
    assert_eq!(detect_protocol(buf), Protocol::Http);
    let resp = parse_http_response(buf).unwrap();
    assert_eq!(resp.status, 404);
    assert_eq!(resp.version, "HTTP/1.1");
}

#[test]
fn test_invalid_method_is_not_http() {
    let buf = b"FOOBAR / HTTP/1.1\r\n";
    assert_eq!(parse_http_request(buf), None);
    assert_eq!(detect_protocol(buf), Protocol::Unknown);
}

// ── HTTP/2 + gRPC ─────────────────────────────────────────────────────────────

#[test]
fn test_detect_http2_preface() {
    let buf = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    assert_eq!(detect_protocol(buf), Protocol::Http2);
}

#[test]
fn test_parse_grpc_path() {
    let (svc, method) = parse_grpc_path("/helloworld.Greeter/SayHello").unwrap();
    assert_eq!(svc, "helloworld.Greeter");
    assert_eq!(method, "SayHello");
    assert!(parse_grpc_path("/no-method").is_none());
}

#[test]
fn test_classify_grpc_requires_http2_and_grpc_content_type() {
    assert_eq!(
        classify_grpc(true, "application/grpc"),
        Protocol::Grpc
    );
    assert_eq!(
        classify_grpc(true, "application/grpc+proto"),
        Protocol::Grpc
    );
    // HTTP/2 but plain JSON -> still HTTP/2, not gRPC.
    assert_eq!(classify_grpc(true, "application/json"), Protocol::Http2);
    // gRPC content-type without HTTP/2 is not valid gRPC.
    assert_eq!(classify_grpc(false, "application/grpc"), Protocol::Unknown);
}

// ── SQL ───────────────────────────────────────────────────────────────────────

fn postgres_query(sql: &str) -> Vec<u8> {
    // 'Q' | Int32 length (self + string + NUL) | string | NUL
    let mut buf = vec![b'Q'];
    let len = (4 + sql.len() + 1) as u32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(sql.as_bytes());
    buf.push(0);
    buf
}

fn mysql_query(sql: &str) -> Vec<u8> {
    // 3-byte LE payload length | 1-byte seq | 0x03 COM_QUERY | query
    let payload_len = (1 + sql.len()) as u32;
    let mut buf = vec![
        (payload_len & 0xff) as u8,
        ((payload_len >> 8) & 0xff) as u8,
        ((payload_len >> 16) & 0xff) as u8,
        0, // seq id
        0x03, // COM_QUERY
    ];
    buf.extend_from_slice(sql.as_bytes());
    buf
}

#[test]
fn test_detect_and_parse_postgres_query() {
    let buf = postgres_query("SELECT * FROM users;");
    assert_eq!(detect_protocol(&buf), Protocol::Sql);
    assert_eq!(parse_postgres_query(&buf).unwrap(), "SELECT * FROM users;");
}

#[test]
fn test_detect_and_parse_mysql_query() {
    let buf = mysql_query("UPDATE t SET a=1");
    assert_eq!(detect_protocol(&buf), Protocol::Sql);
    assert_eq!(parse_mysql_query(&buf).unwrap(), "UPDATE t SET a=1");
}

#[test]
fn test_sql_command_classification_is_case_insensitive() {
    assert_eq!(sql_command("  select 1"), SqlCommand::Select);
    assert_eq!(sql_command("INSERT INTO t VALUES (1)"), SqlCommand::Insert);
    assert_eq!(sql_command("Update t set x=1"), SqlCommand::Update);
    assert_eq!(sql_command("DELETE FROM t"), SqlCommand::Delete);
    assert_eq!(sql_command("CREATE TABLE t(x int)"), SqlCommand::Create);
    assert_eq!(sql_command("DROP TABLE t"), SqlCommand::Drop);
    assert_eq!(sql_command("VACUUM"), SqlCommand::Other);
}

#[test]
fn test_empty_buffer_is_unknown() {
    assert_eq!(detect_protocol(b""), Protocol::Unknown);
    assert_eq!(parse_http_request(b""), None);
}
