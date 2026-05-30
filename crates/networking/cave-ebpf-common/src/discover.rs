// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Protocol auto-detection — the userspace heart of grafana/beyla's
//! auto-instrumentation.
//!
//! Beyla's kernel probes capture the first bytes of each connection's
//! payload and userspace classifies the L7 protocol from them
//! (`pkg/internal/ebpf/common` protocol detection + `pkg/internal/request`
//! parsing). This module ports that classification: HTTP/1.x request and
//! response lines, the HTTP/2 connection preface, gRPC (HTTP/2 + a
//! `application/grpc` content-type), and SQL framing for Postgres
//! simple-query and MySQL `COM_QUERY`.
//!
//! No kernel is involved — these are pure byte-buffer parsers, identical
//! to what Beyla runs once the bytes reach userspace.

/// Detected L7 protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Http,
    Http2,
    Grpc,
    Sql,
    Unknown,
}

/// HTTP request method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
    Patch,
    Connect,
    Trace,
}

impl HttpMethod {
    fn parse(tok: &str) -> Option<HttpMethod> {
        Some(match tok {
            "GET" => HttpMethod::Get,
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "DELETE" => HttpMethod::Delete,
            "HEAD" => HttpMethod::Head,
            "OPTIONS" => HttpMethod::Options,
            "PATCH" => HttpMethod::Patch,
            "CONNECT" => HttpMethod::Connect,
            "TRACE" => HttpMethod::Trace,
            _ => return None,
        })
    }
}

/// Parsed HTTP/1.x request line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub path: String,
    pub version: String,
}

/// Parsed HTTP/1.x status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub version: String,
    pub status: u16,
}

/// SQL verb classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlCommand {
    Select,
    Insert,
    Update,
    Delete,
    Create,
    Drop,
    Other,
}

/// The HTTP/2 client connection preface.
const HTTP2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// First line of `buf` as a `&str`, up to CRLF (or end).
fn first_line(buf: &[u8]) -> Option<&str> {
    let end = buf
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(buf.len());
    std::str::from_utf8(&buf[..end]).ok()
}

/// Parse an HTTP/1.x request line, e.g. `GET /path HTTP/1.1`.
pub fn parse_http_request(buf: &[u8]) -> Option<HttpRequest> {
    let line = first_line(buf)?;
    let mut parts = line.splitn(3, ' ');
    let method = HttpMethod::parse(parts.next()?)?;
    let path = parts.next()?.to_string();
    let version = parts.next()?.to_string();
    if !version.starts_with("HTTP/1.") || path.is_empty() {
        return None;
    }
    Some(HttpRequest {
        method,
        path,
        version,
    })
}

/// Parse an HTTP/1.x status line, e.g. `HTTP/1.1 200 OK`.
pub fn parse_http_response(buf: &[u8]) -> Option<HttpResponse> {
    let line = first_line(buf)?;
    let mut parts = line.splitn(3, ' ');
    let version = parts.next()?.to_string();
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let status: u16 = parts.next()?.parse().ok()?;
    if !(100..=599).contains(&status) {
        return None;
    }
    Some(HttpResponse { version, status })
}

/// Split a gRPC `:path` of the form `/service/method` into its parts.
pub fn parse_grpc_path(path: &str) -> Option<(String, String)> {
    let trimmed = path.strip_prefix('/')?;
    let (svc, method) = trimmed.split_once('/')?;
    if svc.is_empty() || method.is_empty() {
        return None;
    }
    Some((svc.to_string(), method.to_string()))
}

/// Classify an HTTP/2 stream as gRPC given its content-type. gRPC requires
/// HTTP/2 transport and an `application/grpc` content-type (optionally with
/// a `+proto` / `+json` suffix).
pub fn classify_grpc(is_http2: bool, content_type: &str) -> Protocol {
    let is_grpc_ct = content_type == "application/grpc"
        || content_type.starts_with("application/grpc+")
        || content_type.starts_with("application/grpc;");
    match (is_http2, is_grpc_ct) {
        (true, true) => Protocol::Grpc,
        (true, false) => Protocol::Http2,
        (false, _) => Protocol::Unknown,
    }
}

/// Extract the query text from a Postgres frontend simple-query (`Q`)
/// message: `'Q' | Int32 len | string | NUL`.
pub fn parse_postgres_query(buf: &[u8]) -> Option<String> {
    if buf.first() != Some(&b'Q') || buf.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    // len covers the Int32 (4) + string + trailing NUL (1).
    if len < 5 {
        return None;
    }
    let str_len = len - 4 - 1;
    let start: usize = 5;
    let end = start.checked_add(str_len)?;
    if end > buf.len() {
        return None;
    }
    std::str::from_utf8(&buf[start..end]).ok().map(str::to_string)
}

/// Extract the query text from a MySQL `COM_QUERY` packet:
/// `u24-le payload-len | u8 seq | 0x03 | query`.
pub fn parse_mysql_query(buf: &[u8]) -> Option<String> {
    if buf.len() < 5 {
        return None;
    }
    let payload_len = (buf[0] as usize) | ((buf[1] as usize) << 8) | ((buf[2] as usize) << 16);
    if buf[4] != 0x03 || payload_len < 1 {
        return None;
    }
    let query_len = payload_len - 1;
    let start: usize = 5;
    let end = start.checked_add(query_len)?;
    if end > buf.len() {
        return None;
    }
    std::str::from_utf8(&buf[start..end]).ok().map(str::to_string)
}

/// Classify a SQL statement by its leading verb (case-insensitive).
pub fn sql_command(sql: &str) -> SqlCommand {
    let verb = sql.trim_start().split_whitespace().next().unwrap_or("");
    match verb.to_ascii_uppercase().as_str() {
        "SELECT" => SqlCommand::Select,
        "INSERT" => SqlCommand::Insert,
        "UPDATE" => SqlCommand::Update,
        "DELETE" => SqlCommand::Delete,
        "CREATE" => SqlCommand::Create,
        "DROP" => SqlCommand::Drop,
        _ => SqlCommand::Other,
    }
}

/// Top-level protocol detection from a connection's first bytes — the
/// dispatch Beyla runs per captured buffer.
pub fn detect_protocol(buf: &[u8]) -> Protocol {
    if buf.is_empty() {
        return Protocol::Unknown;
    }
    if buf.starts_with(HTTP2_PREFACE) {
        return Protocol::Http2;
    }
    if parse_http_request(buf).is_some() || parse_http_response(buf).is_some() {
        return Protocol::Http;
    }
    if parse_postgres_query(buf).is_some() || parse_mysql_query(buf).is_some() {
        return Protocol::Sql;
    }
    Protocol::Unknown
}
