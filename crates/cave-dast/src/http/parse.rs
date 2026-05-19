// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: parosproxy/paros/network/HttpRequestHeader.java
//         parosproxy/paros/network/HttpResponseHeader.java
//
//! Wire-level parsers for HTTP request and response messages. RFC 7230
//! conformant — request-line, header block, optional body separated by
//! `\r\n\r\n`. Lenient about line endings (accepts bare `\n`).

use super::{HeaderMap, HttpMethod, HttpRequest, HttpResponse};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("empty message")]
    Empty,
    #[error("malformed request-line: {0}")]
    MalformedRequestLine(String),
    #[error("malformed status-line: {0}")]
    MalformedStatusLine(String),
    #[error("malformed header: {0}")]
    MalformedHeader(String),
    #[error("invalid status code: {0}")]
    InvalidStatus(String),
}

fn split_headers_and_body(input: &str) -> (&str, &str) {
    if let Some(i) = input.find("\r\n\r\n") {
        return (&input[..i], &input[i + 4..]);
    }
    if let Some(i) = input.find("\n\n") {
        return (&input[..i], &input[i + 2..]);
    }
    (input, "")
}

fn parse_headers(block: &str) -> Result<HeaderMap, ParseError> {
    let mut headers = HeaderMap::new();
    let mut lines = block.split('\n');
    // First line is request/status line — skip in caller.
    let _ = lines.next();
    for raw in lines {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ParseError::MalformedHeader(line.to_string()))?;
        headers.insert(name.trim().to_string(), value.trim().to_string());
    }
    Ok(headers)
}

pub fn parse_request(input: &str) -> Result<HttpRequest, ParseError> {
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    let (head, body) = split_headers_and_body(input);
    let first = head.split('\n').next().unwrap_or("").trim_end_matches('\r');
    let mut parts = first.splitn(3, ' ');
    let method = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::MalformedRequestLine(first.to_string()))?;
    let url = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::MalformedRequestLine(first.to_string()))?;
    let version = parts
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("HTTP/1.1");

    let headers = parse_headers(head)?;
    Ok(HttpRequest {
        method: HttpMethod::parse(method),
        url: url.to_string(),
        version: version.to_string(),
        headers,
        body: body.as_bytes().to_vec(),
    })
}

pub fn parse_response(input: &str) -> Result<HttpResponse, ParseError> {
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    let (head, body) = split_headers_and_body(input);
    let first = head.split('\n').next().unwrap_or("").trim_end_matches('\r');
    let mut parts = first.splitn(3, ' ');
    let version = parts
        .next()
        .filter(|s| s.starts_with("HTTP/"))
        .ok_or_else(|| ParseError::MalformedStatusLine(first.to_string()))?;
    let status_str = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::MalformedStatusLine(first.to_string()))?;
    let status: u16 = status_str
        .parse()
        .map_err(|_| ParseError::InvalidStatus(status_str.to_string()))?;
    let reason = parts.next().unwrap_or("").to_string();

    let headers = parse_headers(head)?;
    Ok(HttpResponse {
        version: version.to_string(),
        status,
        reason,
        headers,
        body: body.as_bytes().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_get() {
        let raw = "GET /api/v1 HTTP/1.1\r\nHost: x.com\r\nAccept: */*\r\n\r\n";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.method, HttpMethod::Get);
        assert_eq!(req.url, "/api/v1");
        assert_eq!(req.headers.first("Host"), Some("x.com"));
        assert!(req.body.is_empty());
    }

    #[test]
    fn parse_post_with_body() {
        let raw = "POST /login HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\n\r\nuser=a&pw=b";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.method, HttpMethod::Post);
        assert_eq!(req.body_str(), Some("user=a&pw=b"));
    }

    #[test]
    fn parse_accepts_bare_lf() {
        let raw = "GET / HTTP/1.0\nHost: x\n\n";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.version, "HTTP/1.0");
        assert_eq!(req.headers.first("Host"), Some("x"));
    }

    #[test]
    fn parse_response_simple() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nhello";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.reason, "OK");
        assert_eq!(resp.body_str(), Some("hello"));
        assert_eq!(resp.headers.first("Content-Length"), Some("5"));
    }

    #[test]
    fn parse_response_redirect() {
        let raw = "HTTP/1.1 302 Found\r\nLocation: /next\r\n\r\n";
        let resp = parse_response(raw).unwrap();
        assert_eq!(resp.status, 302);
        assert_eq!(resp.headers.first("Location"), Some("/next"));
    }

    #[test]
    fn parse_request_empty_errors() {
        assert!(matches!(parse_request(""), Err(ParseError::Empty)));
    }

    #[test]
    fn parse_request_bad_first_line() {
        assert!(parse_request("BROKEN\r\n\r\n").is_err());
    }

    #[test]
    fn parse_response_bad_status() {
        let raw = "HTTP/1.1 ABC Bad\r\n\r\n";
        assert!(matches!(
            parse_response(raw),
            Err(ParseError::InvalidStatus(_))
        ));
    }
}
