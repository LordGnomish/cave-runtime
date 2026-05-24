// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP/1.1 wire helpers.
//!
//! Tiny parser/serializer used by the in-process integration tests; the
//! production listener uses `axum`/`hyper` and constructs `GwRequest` directly.

use crate::error::{AGwError, AGwResult};
use crate::proxy::GwRequest;

pub struct Http1Codec;

impl Http1Codec {
    /// Parse an HTTP/1.1 request line + headers from a UTF-8 buffer.
    /// Body parsing is intentionally minimal — just records what's after the headers.
    pub fn parse_request(buf: &str) -> AGwResult<GwRequest> {
        let mut lines = buf.split("\r\n");
        let first = lines.next().ok_or_else(|| AGwError::BadRequest("empty".into()))?;
        let mut parts = first.split_whitespace();
        let method = parts.next().ok_or_else(|| AGwError::BadRequest("no method".into()))?;
        let uri = parts.next().ok_or_else(|| AGwError::BadRequest("no uri".into()))?;
        let version = parts.next().unwrap_or("HTTP/1.1");
        if !version.starts_with("HTTP/") {
            return Err(AGwError::BadRequest(format!("bad version {version}")));
        }
        let mut req = GwRequest::new(method, uri, "");
        for line in &mut lines {
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim().to_lowercase();
                let v = v.trim().to_string();
                if k == "host" {
                    req.host = v.clone();
                }
                req.headers.insert(k, v);
            }
        }
        let rest: Vec<&str> = lines.collect();
        let body = rest.join("\r\n");
        if !body.is_empty() {
            req.body = body.into_bytes();
        }
        Ok(req)
    }

    /// Serialize a request back to wire format (used in fixture diffs).
    pub fn serialize_request(req: &GwRequest) -> String {
        let mut out = format!("{} {} HTTP/1.1\r\n", req.method, req.uri);
        if !req.host.is_empty() && !req.headers.contains_key("host") {
            out.push_str(&format!("Host: {}\r\n", req.host));
        }
        for (k, v) in &req.headers {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
        out.push_str("\r\n");
        if !req.body.is_empty() {
            out.push_str(std::str::from_utf8(&req.body).unwrap_or(""));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_get() {
        let buf = "GET /api/v1 HTTP/1.1\r\nHost: api.example\r\nX-Trace: 1\r\n\r\n";
        let r = Http1Codec::parse_request(buf).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.uri, "/api/v1");
        assert_eq!(r.host, "api.example");
        assert_eq!(r.headers.get("x-trace").map(|s| s.as_str()), Some("1"));
    }

    #[test]
    fn parse_post_with_body() {
        let buf = "POST /upload HTTP/1.1\r\nHost: h\r\nContent-Type: text/plain\r\n\r\nhello";
        let r = Http1Codec::parse_request(buf).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn rejects_empty_buffer() {
        assert!(Http1Codec::parse_request("").is_err());
    }

    #[test]
    fn rejects_garbage_version() {
        let buf = "GET / FOO\r\n\r\n";
        assert!(Http1Codec::parse_request(buf).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let buf = "GET /x HTTP/1.1\r\nHost: h\r\nX-Trace: abc\r\n\r\n";
        let r = Http1Codec::parse_request(buf).unwrap();
        let out = Http1Codec::serialize_request(&r);
        assert!(out.starts_with("GET /x HTTP/1.1\r\n"));
        assert!(out.contains("x-trace: abc"));
    }
}
