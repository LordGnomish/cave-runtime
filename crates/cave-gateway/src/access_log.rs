//! Access log — Envoy `access_log.v3.AccessLog` parity (text + JSON formats).
//!
//! Models the format-string DSL Envoy uses (`%REQ(:METHOD)%`, `%RESP(...)%`,
//! `%DURATION%` etc.) plus the JSON formatter that emits a typed map. Also
//! ships the runtime hook used by `proxy.rs` to record an entry per request.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fmt::Write;
use std::time::Duration;

/// One line in the operator-supplied format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatToken {
    Literal(String),
    /// `%REQ(:METHOD)%`, `%REQ(X-Request-Id)%`.
    Req(String),
    /// `%RESP(:STATUS)%`, `%RESP(content-length)%`.
    Resp(String),
    /// `%DURATION%` — total request time in ms.
    Duration,
    /// `%REQUEST_DURATION%` — time waiting on upstream.
    RequestDuration,
    /// `%RESPONSE_DURATION%`.
    ResponseDuration,
    /// `%RESPONSE_FLAGS%` — UH (no healthy upstream), UT (upstream timeout)…
    ResponseFlags,
    /// `%BYTES_RECEIVED%`, `%BYTES_SENT%`.
    BytesReceived,
    BytesSent,
    /// `%PROTOCOL%` — HTTP/1.1, HTTP/2, HTTP/3.
    Protocol,
    /// `%START_TIME%` — RFC3339.
    StartTime,
    /// `%DOWNSTREAM_REMOTE_ADDRESS%`.
    DownstreamRemoteAddress,
    /// `%UPSTREAM_HOST%`.
    UpstreamHost,
    /// `%UPSTREAM_CLUSTER%`.
    UpstreamCluster,
    /// `%REQUESTED_SERVER_NAME%` — TLS SNI.
    RequestedServerName,
    /// `%CONNECTION_ID%`.
    ConnectionId,
    /// `%STREAM_ID%`.
    StreamId,
    /// `%ROUTE_NAME%`.
    RouteName,
    /// `%TRACE_ID%`.
    TraceId,
    /// `%SPAN_ID%`.
    SpanId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatString(pub Vec<FormatToken>);

impl FormatString {
    pub fn parse(s: &str) -> Result<FormatString, String> {
        let mut tokens = Vec::new();
        let mut buf = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                buf.push(c);
                continue;
            }
            // Try to consume a placeholder.
            let mut name = String::new();
            let mut closed = false;
            for n in chars.by_ref() {
                if n == '%' {
                    closed = true;
                    break;
                }
                name.push(n);
            }
            if !closed {
                return Err(format!("unterminated %…% at: {}", name));
            }
            if !buf.is_empty() {
                tokens.push(FormatToken::Literal(std::mem::take(&mut buf)));
            }
            tokens.push(parse_token(&name)?);
        }
        if !buf.is_empty() {
            tokens.push(FormatToken::Literal(buf));
        }
        Ok(FormatString(tokens))
    }
}

fn parse_token(name: &str) -> Result<FormatToken, String> {
    if let Some(rest) = name.strip_prefix("REQ(").and_then(|r| r.strip_suffix(')')) {
        return Ok(FormatToken::Req(rest.into()));
    }
    if let Some(rest) = name.strip_prefix("RESP(").and_then(|r| r.strip_suffix(')')) {
        return Ok(FormatToken::Resp(rest.into()));
    }
    Ok(match name {
        "DURATION" => FormatToken::Duration,
        "REQUEST_DURATION" => FormatToken::RequestDuration,
        "RESPONSE_DURATION" => FormatToken::ResponseDuration,
        "RESPONSE_FLAGS" => FormatToken::ResponseFlags,
        "BYTES_RECEIVED" => FormatToken::BytesReceived,
        "BYTES_SENT" => FormatToken::BytesSent,
        "PROTOCOL" => FormatToken::Protocol,
        "START_TIME" => FormatToken::StartTime,
        "DOWNSTREAM_REMOTE_ADDRESS" => FormatToken::DownstreamRemoteAddress,
        "UPSTREAM_HOST" => FormatToken::UpstreamHost,
        "UPSTREAM_CLUSTER" => FormatToken::UpstreamCluster,
        "REQUESTED_SERVER_NAME" => FormatToken::RequestedServerName,
        "CONNECTION_ID" => FormatToken::ConnectionId,
        "STREAM_ID" => FormatToken::StreamId,
        "ROUTE_NAME" => FormatToken::RouteName,
        "TRACE_ID" => FormatToken::TraceId,
        "SPAN_ID" => FormatToken::SpanId,
        other => return Err(format!("unknown format token: {}", other)),
    })
}

// ── Response flags (Envoy semantics) ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ResponseFlag {
    /// No healthy upstream hosts.
    Uh,
    /// Upstream connection failure (connect error).
    Uf,
    /// Upstream connection terminated mid-request.
    Ut,
    /// Local origin connection failure.
    Lr,
    /// Rate limited.
    Rl,
    /// Upstream overflow / circuit breaker open.
    Uo,
    /// No route configured.
    Nr,
    /// External authorization denied.
    Ua,
    /// Stream idle timeout.
    Si,
    /// Downstream request timeout.
    Dt,
    /// Downstream connection terminated.
    Dc,
    /// Upstream request timeout.
    Urx,
}

impl ResponseFlag {
    pub fn short(self) -> &'static str {
        match self {
            ResponseFlag::Uh => "UH",
            ResponseFlag::Uf => "UF",
            ResponseFlag::Ut => "UT",
            ResponseFlag::Lr => "LR",
            ResponseFlag::Rl => "RL",
            ResponseFlag::Uo => "UO",
            ResponseFlag::Nr => "NR",
            ResponseFlag::Ua => "UA",
            ResponseFlag::Si => "SI",
            ResponseFlag::Dt => "DT",
            ResponseFlag::Dc => "DC",
            ResponseFlag::Urx => "URX",
        }
    }
}

pub fn render_flags(flags: &[ResponseFlag]) -> String {
    if flags.is_empty() {
        return "-".into();
    }
    let mut out = String::new();
    for (i, f) in flags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(f.short());
    }
    out
}

// ── AccessLogEntry — what proxy.rs hands the formatter ──────────────────────

#[derive(Debug, Clone, Default)]
pub struct AccessLogEntry {
    pub start_time: Option<DateTime<Utc>>,
    pub method: String,
    pub path: String,
    pub protocol: String,
    pub request_headers: HashMap<String, String>,
    pub response_headers: HashMap<String, String>,
    pub status: Option<u16>,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub duration: Duration,
    pub upstream_request_duration: Duration,
    pub upstream_response_duration: Duration,
    pub response_flags: Vec<ResponseFlag>,
    pub downstream_remote: String,
    pub upstream_host: Option<String>,
    pub upstream_cluster: Option<String>,
    pub requested_server_name: Option<String>,
    pub connection_id: Option<String>,
    pub stream_id: Option<String>,
    pub route_name: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

impl AccessLogEntry {
    pub fn render_text(&self, fmt: &FormatString) -> String {
        let mut out = String::new();
        for t in &fmt.0 {
            match t {
                FormatToken::Literal(s) => out.push_str(s),
                FormatToken::Req(name) => self.write_req(name, &mut out),
                FormatToken::Resp(name) => self.write_resp(name, &mut out),
                FormatToken::Duration => write!(out, "{}", self.duration.as_millis()).ok().unwrap(),
                FormatToken::RequestDuration => write!(out, "{}", self.upstream_request_duration.as_millis()).ok().unwrap(),
                FormatToken::ResponseDuration => write!(out, "{}", self.upstream_response_duration.as_millis()).ok().unwrap(),
                FormatToken::ResponseFlags => out.push_str(&render_flags(&self.response_flags)),
                FormatToken::BytesReceived => write!(out, "{}", self.bytes_received).ok().unwrap(),
                FormatToken::BytesSent => write!(out, "{}", self.bytes_sent).ok().unwrap(),
                FormatToken::Protocol => out.push_str(&self.protocol),
                FormatToken::StartTime => {
                    if let Some(t) = self.start_time {
                        out.push_str(&t.to_rfc3339());
                    } else {
                        out.push('-');
                    }
                }
                FormatToken::DownstreamRemoteAddress => out.push_str(&self.downstream_remote),
                FormatToken::UpstreamHost => out.push_str(self.upstream_host.as_deref().unwrap_or("-")),
                FormatToken::UpstreamCluster => out.push_str(self.upstream_cluster.as_deref().unwrap_or("-")),
                FormatToken::RequestedServerName => out.push_str(self.requested_server_name.as_deref().unwrap_or("-")),
                FormatToken::ConnectionId => out.push_str(self.connection_id.as_deref().unwrap_or("-")),
                FormatToken::StreamId => out.push_str(self.stream_id.as_deref().unwrap_or("-")),
                FormatToken::RouteName => out.push_str(self.route_name.as_deref().unwrap_or("-")),
                FormatToken::TraceId => out.push_str(self.trace_id.as_deref().unwrap_or("-")),
                FormatToken::SpanId => out.push_str(self.span_id.as_deref().unwrap_or("-")),
            }
        }
        out
    }

    fn write_req(&self, name: &str, out: &mut String) {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            ":method" => out.push_str(&self.method),
            ":path" => out.push_str(&self.path),
            ":authority" => out.push_str(self.request_headers.get("host").map(|s| s.as_str()).unwrap_or("-")),
            other => out.push_str(self.request_headers.get(other).map(|s| s.as_str()).unwrap_or("-")),
        }
    }

    fn write_resp(&self, name: &str, out: &mut String) {
        let lower = name.to_ascii_lowercase();
        if lower == ":status" {
            match self.status {
                Some(s) => write!(out, "{}", s).ok().unwrap(),
                None => out.push('-'),
            }
            return;
        }
        out.push_str(self.response_headers.get(&lower).map(|s| s.as_str()).unwrap_or("-"));
    }

    pub fn render_json(&self, schema: &JsonLogSchema) -> Value {
        let mut map = Map::new();
        for (key, token) in &schema.0 {
            let val = match token {
                FormatToken::Literal(s) => Value::String(s.clone()),
                FormatToken::Req(name) => {
                    let mut s = String::new();
                    self.write_req(name, &mut s);
                    Value::String(s)
                }
                FormatToken::Resp(name) => {
                    let mut s = String::new();
                    self.write_resp(name, &mut s);
                    Value::String(s)
                }
                FormatToken::Duration => json!(self.duration.as_millis() as u64),
                FormatToken::RequestDuration => json!(self.upstream_request_duration.as_millis() as u64),
                FormatToken::ResponseDuration => json!(self.upstream_response_duration.as_millis() as u64),
                FormatToken::ResponseFlags => Value::String(render_flags(&self.response_flags)),
                FormatToken::BytesReceived => json!(self.bytes_received),
                FormatToken::BytesSent => json!(self.bytes_sent),
                FormatToken::Protocol => Value::String(self.protocol.clone()),
                FormatToken::StartTime => match self.start_time {
                    Some(t) => Value::String(t.to_rfc3339()),
                    None => Value::Null,
                },
                FormatToken::DownstreamRemoteAddress => Value::String(self.downstream_remote.clone()),
                FormatToken::UpstreamHost => option_to_value(&self.upstream_host),
                FormatToken::UpstreamCluster => option_to_value(&self.upstream_cluster),
                FormatToken::RequestedServerName => option_to_value(&self.requested_server_name),
                FormatToken::ConnectionId => option_to_value(&self.connection_id),
                FormatToken::StreamId => option_to_value(&self.stream_id),
                FormatToken::RouteName => option_to_value(&self.route_name),
                FormatToken::TraceId => option_to_value(&self.trace_id),
                FormatToken::SpanId => option_to_value(&self.span_id),
            };
            map.insert(key.clone(), val);
        }
        Value::Object(map)
    }
}

fn option_to_value(s: &Option<String>) -> Value {
    match s {
        Some(v) => Value::String(v.clone()),
        None => Value::Null,
    }
}

// ── JsonLogSchema — operator-supplied key→token mapping ─────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonLogSchema(pub Vec<(String, FormatToken)>);

impl JsonLogSchema {
    pub fn envoy_default() -> Self {
        Self(vec![
            ("start_time".into(), FormatToken::StartTime),
            ("method".into(), FormatToken::Req(":method".into())),
            ("path".into(), FormatToken::Req(":path".into())),
            ("protocol".into(), FormatToken::Protocol),
            ("status".into(), FormatToken::Resp(":status".into())),
            ("response_flags".into(), FormatToken::ResponseFlags),
            ("bytes_received".into(), FormatToken::BytesReceived),
            ("bytes_sent".into(), FormatToken::BytesSent),
            ("duration_ms".into(), FormatToken::Duration),
            ("upstream_host".into(), FormatToken::UpstreamHost),
            ("upstream_cluster".into(), FormatToken::UpstreamCluster),
            ("downstream_remote".into(), FormatToken::DownstreamRemoteAddress),
            ("trace_id".into(), FormatToken::TraceId),
        ])
    }

    pub fn parse_value_template(template: HashMap<String, String>) -> Result<Self, String> {
        let mut out = Vec::with_capacity(template.len());
        for (k, v) in template {
            // Each value may itself be a single %TOKEN% or a literal.
            if v.starts_with('%') && v.ends_with('%') && v.len() >= 2 {
                let name = &v[1..v.len() - 1];
                out.push((k, parse_token(name)?));
            } else {
                out.push((k, FormatToken::Literal(v)));
            }
        }
        Ok(Self(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> AccessLogEntry {
        let mut req = HashMap::new();
        req.insert("host".into(), "example.com".into());
        req.insert("user-agent".into(), "curl/8".into());
        let mut resp = HashMap::new();
        resp.insert("content-type".into(), "application/json".into());
        AccessLogEntry {
            start_time: Some(Utc::now()),
            method: "GET".into(),
            path: "/api/v1/users".into(),
            protocol: "HTTP/1.1".into(),
            request_headers: req,
            response_headers: resp,
            status: Some(200),
            bytes_received: 256,
            bytes_sent: 1024,
            duration: Duration::from_millis(15),
            upstream_request_duration: Duration::from_millis(8),
            upstream_response_duration: Duration::from_millis(7),
            response_flags: vec![],
            downstream_remote: "1.2.3.4:11234".into(),
            upstream_host: Some("10.0.0.1:8080".into()),
            upstream_cluster: Some("svc-a".into()),
            requested_server_name: Some("example.com".into()),
            connection_id: Some("c-1".into()),
            stream_id: Some("s-1".into()),
            route_name: Some("api-v1".into()),
            trace_id: Some("0102030405060708090a0b0c0d0e0f10".into()),
            span_id: Some("0102030405060708".into()),
        }
    }

    // ── Format parser ──

    #[test]
    fn parse_literal_only() {
        let f = FormatString::parse("hello world").unwrap();
        assert_eq!(f.0, vec![FormatToken::Literal("hello world".into())]);
    }

    #[test]
    fn parse_simple_token() {
        let f = FormatString::parse("%PROTOCOL%").unwrap();
        assert_eq!(f.0, vec![FormatToken::Protocol]);
    }

    #[test]
    fn parse_req_token() {
        let f = FormatString::parse("%REQ(:METHOD)%").unwrap();
        assert_eq!(f.0, vec![FormatToken::Req(":METHOD".into())]);
    }

    #[test]
    fn parse_resp_token() {
        let f = FormatString::parse("%RESP(:STATUS)%").unwrap();
        assert_eq!(f.0, vec![FormatToken::Resp(":STATUS".into())]);
    }

    #[test]
    fn parse_mixed() {
        let f = FormatString::parse("[%START_TIME%] %REQ(:METHOD)% %REQ(:PATH)% %RESP(:STATUS)%").unwrap();
        // 6 literal/token pieces (with trailing literal absent here).
        assert!(f.0.iter().any(|t| matches!(t, FormatToken::StartTime)));
        assert!(f.0.iter().any(|t| matches!(t, FormatToken::Req(_))));
    }

    #[test]
    fn parse_unterminated_rejects() {
        assert!(FormatString::parse("%PROTOCOL").is_err());
    }

    #[test]
    fn parse_unknown_token_rejects() {
        assert!(FormatString::parse("%UNKNOWN_THING%").is_err());
    }

    // ── Render: text ──

    #[test]
    fn render_method_path_status() {
        let e = entry();
        let f = FormatString::parse("%REQ(:METHOD)% %REQ(:PATH)% %RESP(:STATUS)%").unwrap();
        assert_eq!(e.render_text(&f), "GET /api/v1/users 200");
    }

    #[test]
    fn render_duration_ms() {
        let e = entry();
        let f = FormatString::parse("dur=%DURATION%").unwrap();
        assert_eq!(e.render_text(&f), "dur=15");
    }

    #[test]
    fn render_bytes_in_out() {
        let e = entry();
        let f = FormatString::parse("%BYTES_RECEIVED% %BYTES_SENT%").unwrap();
        assert_eq!(e.render_text(&f), "256 1024");
    }

    #[test]
    fn render_response_flags_empty_dash() {
        let e = entry();
        let f = FormatString::parse("%RESPONSE_FLAGS%").unwrap();
        assert_eq!(e.render_text(&f), "-");
    }

    #[test]
    fn render_response_flags_concat() {
        let mut e = entry();
        e.response_flags = vec![ResponseFlag::Uh, ResponseFlag::Uo];
        let f = FormatString::parse("%RESPONSE_FLAGS%").unwrap();
        assert_eq!(e.render_text(&f), "UH,UO");
    }

    #[test]
    fn render_authority_pseudo_header() {
        let e = entry();
        let f = FormatString::parse("%REQ(:AUTHORITY)%").unwrap();
        assert_eq!(e.render_text(&f), "example.com");
    }

    #[test]
    fn render_missing_request_header_yields_dash() {
        let e = entry();
        let f = FormatString::parse("%REQ(X-Trace-Id)%").unwrap();
        assert_eq!(e.render_text(&f), "-");
    }

    #[test]
    fn render_missing_response_header_yields_dash() {
        let e = entry();
        let f = FormatString::parse("%RESP(X-Build-Id)%").unwrap();
        assert_eq!(e.render_text(&f), "-");
    }

    #[test]
    fn render_status_present() {
        let e = entry();
        let f = FormatString::parse("%RESP(:STATUS)%").unwrap();
        assert_eq!(e.render_text(&f), "200");
    }

    #[test]
    fn render_status_missing_dash() {
        let mut e = entry();
        e.status = None;
        let f = FormatString::parse("%RESP(:STATUS)%").unwrap();
        assert_eq!(e.render_text(&f), "-");
    }

    #[test]
    fn render_upstream_optional_dashes() {
        let mut e = entry();
        e.upstream_host = None;
        e.upstream_cluster = None;
        let f = FormatString::parse("%UPSTREAM_HOST% %UPSTREAM_CLUSTER%").unwrap();
        assert_eq!(e.render_text(&f), "- -");
    }

    #[test]
    fn render_combined_log_format_envoy_style() {
        let e = entry();
        let f = FormatString::parse(
            r#"[%START_TIME%] "%REQ(:METHOD)% %REQ(:PATH)% %PROTOCOL%" %RESP(:STATUS)% %RESPONSE_FLAGS% %BYTES_RECEIVED% %BYTES_SENT% %DURATION%"#,
        )
        .unwrap();
        let s = e.render_text(&f);
        assert!(s.contains("\"GET /api/v1/users HTTP/1.1\""));
        assert!(s.contains(" 200 "));
    }

    // ── Render: JSON ──

    #[test]
    fn render_json_envoy_default() {
        let e = entry();
        let s = JsonLogSchema::envoy_default();
        let v = e.render_json(&s);
        assert_eq!(v["method"], "GET");
        assert_eq!(v["path"], "/api/v1/users");
        assert_eq!(v["status"], "200");
        assert_eq!(v["upstream_cluster"], "svc-a");
    }

    #[test]
    fn render_json_response_flags_empty() {
        let e = entry();
        let s = JsonLogSchema::envoy_default();
        let v = e.render_json(&s);
        assert_eq!(v["response_flags"], "-");
    }

    #[test]
    fn parse_value_template_token() {
        let mut t = HashMap::new();
        t.insert("status".into(), "%RESP(:STATUS)%".into());
        t.insert("custom".into(), "literal-value".into());
        let s = JsonLogSchema::parse_value_template(t).unwrap();
        let entries: HashMap<String, FormatToken> = s.0.into_iter().collect();
        assert!(matches!(entries.get("status").unwrap(), FormatToken::Resp(_)));
        assert!(matches!(
            entries.get("custom").unwrap(),
            FormatToken::Literal(s) if s == "literal-value"
        ));
    }

    #[test]
    fn parse_value_template_invalid_token_rejects() {
        let mut t = HashMap::new();
        t.insert("x".into(), "%NO_SUCH_TOKEN%".into());
        assert!(JsonLogSchema::parse_value_template(t).is_err());
    }

    // ── ResponseFlag ──

    #[test]
    fn response_flag_short_values() {
        assert_eq!(ResponseFlag::Uh.short(), "UH");
        assert_eq!(ResponseFlag::Urx.short(), "URX");
    }

    #[test]
    fn response_flag_serde_uppercase() {
        let s = serde_json::to_string(&ResponseFlag::Ut).unwrap();
        assert_eq!(s, r#""UT""#);
    }
}
