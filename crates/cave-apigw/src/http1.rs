// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP/1.1 codec helpers — used by tests; production listener uses hyper.

use crate::error::{AGwError, AGwResult};
use crate::proxy::GwRequest;

pub struct Http1Codec;
impl Http1Codec {
    pub fn parse_request(buf: &str) -> AGwResult<GwRequest> {
        let mut lines = buf.split("\r\n");
        let first = lines.next().ok_or_else(|| AGwError::BadRequest("empty".into()))?;
        let mut parts = first.split_whitespace();
        let method = parts.next().ok_or_else(|| AGwError::BadRequest("no method".into()))?;
        let uri = parts.next().ok_or_else(|| AGwError::BadRequest("no uri".into()))?;
        let version = parts.next().unwrap_or("HTTP/1.1");
        if !version.starts_with("HTTP/") { return Err(AGwError::BadRequest(format!("bad version {version}"))); }
        let mut req = GwRequest::new(method, uri, "");
        for line in &mut lines {
            if line.is_empty() { break; }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim().to_lowercase();
                let v = v.trim().to_string();
                if k == "host" { req.host = v.clone(); }
                req.headers.insert(k, v);
            }
        }
        let body = lines.collect::<Vec<&str>>().join("\r\n");
        if !body.is_empty() { req.body = body.into_bytes(); }
        Ok(req)
    }

    pub fn serialize_request(req: &GwRequest) -> String {
        let mut out = format!("{} {} HTTP/1.1\r\n", req.method, req.uri);
        if !req.host.is_empty() && !req.headers.contains_key("host") {
            out.push_str(&format!("Host: {}\r\n", req.host));
        }
        for (k, v) in &req.headers { out.push_str(&format!("{k}: {v}\r\n")); }
        out.push_str("\r\n");
        if !req.body.is_empty() { out.push_str(std::str::from_utf8(&req.body).unwrap_or("")); }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parse_get() {
        let r = Http1Codec::parse_request("GET /api HTTP/1.1\r\nHost: api.example\r\nX-Trace: 1\r\n\r\n").unwrap();
        assert_eq!(r.method, "GET"); assert_eq!(r.uri, "/api"); assert_eq!(r.host, "api.example");
        assert_eq!(r.headers.get("x-trace").map(|s| s.as_str()), Some("1"));
    }
    #[test] fn parse_post_with_body() {
        let r = Http1Codec::parse_request("POST /up HTTP/1.1\r\nHost: h\r\n\r\nhello").unwrap();
        assert_eq!(r.method, "POST"); assert_eq!(r.body, b"hello");
    }
    #[test] fn rejects_empty() { assert!(Http1Codec::parse_request("").is_err()); }
    #[test] fn rejects_bad_version() { assert!(Http1Codec::parse_request("GET / FOO\r\n\r\n").is_err()); }
    #[test] fn round_trip() {
        let r = Http1Codec::parse_request("GET /x HTTP/1.1\r\nHost: h\r\nX-Trace: abc\r\n\r\n").unwrap();
        let s = Http1Codec::serialize_request(&r);
        assert!(s.starts_with("GET /x HTTP/1.1\r\n"));
        assert!(s.contains("x-trace: abc"));
    }
}
