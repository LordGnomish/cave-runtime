// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use anyhow::{Context, Result};
use clap::ValueEnum;
use colored::Colorize;
use reqwest::{Client, Method, StatusCode};
use serde_json::Value;
use std::time::Duration;

#[derive(ValueEnum, Clone, Debug)]
pub enum Format {
    /// ASCII table (default)
    Table,
    /// Pretty-printed JSON
    Json,
    /// YAML
    Yaml,
}

/// Max number of follow-the-leader retries on `503 + Location`.
/// Matches the production sweep-011 `Backoff::Exponential` shape
/// (base 100 ms, cap 2 s) inlined to avoid a cave-kernel dep just
/// for this hop.
const RAFT_LEADER_REDIRECT_MAX_RETRIES: u8 = 3;
const RAFT_LEADER_REDIRECT_BASE: Duration = Duration::from_millis(100);
const RAFT_LEADER_REDIRECT_CAP: Duration = Duration::from_secs(2);

fn raft_backoff_delay(attempt: u8) -> Duration {
    // 100ms, 200ms, 400ms, 800ms, ... capped at 2s.
    let factor = 1u32 << attempt.min(6);
    let raw = RAFT_LEADER_REDIRECT_BASE
        .checked_mul(factor)
        .unwrap_or(RAFT_LEADER_REDIRECT_CAP);
    raw.min(RAFT_LEADER_REDIRECT_CAP)
}

/// Strip the path/query off `leader_url` so we can stitch the
/// caller's original path onto it. A leader header normally looks
/// like `https://10.0.0.2:6443` (origin only); we tolerate trailing
/// slashes and paths defensively.
fn leader_origin(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw).ok()?;
    let scheme = url.scheme();
    let host = url.host_str()?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}"))
}

// ── HTTP client ───────────────────────────────────────────────────────────────

pub struct ApiClient {
    client: Client,
    base_url: String,
    token: Option<String>,
    format: Format,
}

impl ApiClient {
    pub fn new(base_url: String, token: Option<String>, format: Format) -> Self {
        Self {
            client: Client::new(),
            base_url,
            token,
            format,
        }
    }

    fn build(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let req = self.client.request(method, url);
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    async fn finish(&self, resp: reqwest::Response) -> Result<()> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            match status {
                StatusCode::NOT_FOUND => {
                    eprintln!("{} Endpoint not yet available (404)", "✗".red().bold());
                }
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    eprintln!(
                        "{} {} — set CAVE_TOKEN or --token",
                        "✗".red().bold(),
                        status.as_u16()
                    );
                }
                _ => {
                    eprintln!(
                        "{} HTTP {}: {}",
                        "✗".red().bold(),
                        status.as_u16(),
                        body.trim()
                    );
                }
            }
            return Ok(());
        }

        let body = resp.text().await.context("Failed to read response body")?;
        if body.is_empty() {
            println!("{} Done", "✓".green().bold());
            return Ok(());
        }

        match serde_json::from_str::<Value>(&body) {
            Ok(val) => self.print(&val),
            Err(_) => println!("{}", body),
        }
        Ok(())
    }

    pub async fn get(&self, path: &str) -> Result<()> {
        let resp = self
            .build(Method::GET, path)
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}{}", self.base_url, path))?;
        self.finish(resp).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<()> {
        let resp = self
            .send_with_leader_redirect(Method::POST, path, Some(body), None::<Vec<u8>>)
            .await?;
        self.finish(resp).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let resp = self
            .send_with_leader_redirect(Method::DELETE, path, None, None::<Vec<u8>>)
            .await?;
        self.finish(resp).await
    }

    /// PUT raw bytes (used for Nexus raw asset upload).
    pub async fn put_bytes(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let resp = self
            .send_with_leader_redirect(Method::PUT, path, None, Some(body))
            .await?;
        self.finish(resp).await
    }

    /// Issue a write request with automatic follow-the-leader on
    /// `503 + Location`. Per the cave-etcd RaftBridge contract a
    /// follower returns `503 SERVICE_UNAVAILABLE` with the leader's
    /// origin in the `Location` header; cavectl retries against the
    /// leader instead of bubbling the 503 up. Exponential backoff
    /// (100ms / 200ms / 400ms / 800ms / …, capped at 2s) before each
    /// retry. After [`RAFT_LEADER_REDIRECT_MAX_RETRIES`] hops the
    /// last response is returned unchanged so `finish` can surface
    /// it as a normal error.
    async fn send_with_leader_redirect(
        &self,
        method: Method,
        path: &str,
        json_body: Option<Value>,
        raw_body: Option<Vec<u8>>,
    ) -> Result<reqwest::Response> {
        let mut base = self.base_url.clone();
        let mut attempt: u8 = 0;
        loop {
            let url = format!("{base}{path}");
            let mut req = self.client.request(method.clone(), &url);
            if let Some(t) = &self.token {
                req = req.bearer_auth(t);
            }
            if let Some(b) = json_body.as_ref() {
                req = req.json(b);
            } else if let Some(b) = raw_body.as_ref() {
                req = req.body(b.clone());
            }
            let resp = req
                .send()
                .await
                .with_context(|| format!("Failed to connect to {url}"))?;
            // Only redirect on 503 + a parseable Location header.
            if resp.status() == StatusCode::SERVICE_UNAVAILABLE
                && attempt < RAFT_LEADER_REDIRECT_MAX_RETRIES
            {
                if let Some(loc) = resp.headers().get(reqwest::header::LOCATION) {
                    if let Ok(loc_str) = loc.to_str() {
                        if let Some(new_base) = leader_origin(loc_str) {
                            tokio::time::sleep(raft_backoff_delay(attempt)).await;
                            attempt += 1;
                            base = new_base;
                            continue;
                        }
                    }
                }
            }
            return Ok(resp);
        }
    }

    fn print(&self, value: &Value) {
        match &self.format {
            Format::Json => println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_default()
            ),
            Format::Yaml => println!("{}", serde_yaml::to_string(value).unwrap_or_default()),
            Format::Table => println!("{}", render_table(value)),
        }
    }
}

// ── Table renderer ────────────────────────────────────────────────────────────

fn render_table(value: &Value) -> String {
    match value {
        Value::Array(arr) if arr.is_empty() => format!("  {}", "(empty)".dimmed()),
        Value::Array(arr) => render_array(arr),
        Value::Object(map) if map.is_empty() => format!("  {}", "(empty)".dimmed()),
        Value::Object(map) => render_kv(map),
        Value::Null => format!("  {}", "(null)".dimmed()),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn render_array(arr: &[Value]) -> String {
    // Array of primitives
    let first_obj = match arr.first() {
        Some(Value::Object(o)) => o,
        _ => {
            return arr
                .iter()
                .enumerate()
                .map(|(i, v)| format!("  {}  {}", format!("{}", i + 1).dimmed(), cell_str(v)))
                .collect::<Vec<_>>()
                .join("\n");
        }
    };

    let keys: Vec<&str> = first_obj.keys().map(String::as_str).collect();
    if keys.is_empty() {
        return format!("  {}", "(empty)".dimmed());
    }

    // Calculate column widths (header width vs data width, capped at 60)
    let mut widths: Vec<usize> = keys.iter().map(|k| k.len()).collect();
    for row in arr {
        if let Value::Object(obj) = row {
            for (i, key) in keys.iter().enumerate() {
                let w = char_len(&cell_str(obj.get(*key).unwrap_or(&Value::Null))).min(60);
                widths[i] = widths[i].max(w);
            }
        }
    }

    let mut out = String::new();

    // Header row
    let header: String = keys
        .iter()
        .zip(&widths)
        .map(|(k, w)| format!("{:<width$}", k.to_uppercase(), width = w))
        .collect::<Vec<_>>()
        .join("  ");
    out.push_str(&format!("  {}\n", header.bold()));

    // Separator
    let sep: String = widths
        .iter()
        .map(|w| "-".repeat(*w))
        .collect::<Vec<_>>()
        .join("--");
    out.push_str(&format!("  {}\n", sep.dimmed()));

    // Data rows
    for row in arr {
        if let Value::Object(obj) = row {
            let cells: String = keys
                .iter()
                .zip(&widths)
                .map(|(k, w)| {
                    let s = cell_str(obj.get(*k).unwrap_or(&Value::Null));
                    let t = trunc(&s, 60);
                    format!("{:<width$}", t, width = w)
                })
                .collect::<Vec<_>>()
                .join("  ");
            out.push_str(&format!("  {}\n", cells));
        }
    }

    // Row count footer
    let n = arr.len();
    out.push_str(&format!(
        "\n  {} {}\n",
        n.to_string().cyan().bold(),
        if n == 1 { "item" } else { "items" }
    ));
    out
}

fn render_kv(map: &serde_json::Map<String, Value>) -> String {
    let key_width = map.keys().map(|k| k.len()).max().unwrap_or(0);
    map.iter()
        .map(|(k, v)| {
            format!(
                "  {:<width$}  {}",
                k.bold(),
                cell_str(v).cyan(),
                width = key_width
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cell_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => format!("[{}]", a.len()),
        Value::Object(_) => "{…}".to_string(),
    }
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn trunc(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > max {
        let t: String = chars[..max - 1].iter().collect();
        format!("{}…", t)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[test]
    fn backoff_doubles_until_capped() {
        assert_eq!(raft_backoff_delay(0), Duration::from_millis(100));
        assert_eq!(raft_backoff_delay(1), Duration::from_millis(200));
        assert_eq!(raft_backoff_delay(2), Duration::from_millis(400));
        assert_eq!(raft_backoff_delay(3), Duration::from_millis(800));
        // Higher attempts hit the 2s cap.
        assert_eq!(raft_backoff_delay(5), Duration::from_secs(2));
        assert_eq!(raft_backoff_delay(99), Duration::from_secs(2));
    }

    #[test]
    fn leader_origin_strips_path_and_keeps_port() {
        assert_eq!(
            leader_origin("https://10.0.0.2:6443").as_deref(),
            Some("https://10.0.0.2:6443"),
        );
        assert_eq!(
            leader_origin("https://10.0.0.2:6443/").as_deref(),
            Some("https://10.0.0.2:6443"),
        );
        assert_eq!(
            leader_origin("https://10.0.0.2:6443/v3/kv/put?x=1").as_deref(),
            Some("https://10.0.0.2:6443"),
        );
        assert_eq!(leader_origin("not a url").as_deref(), None);
    }

    #[tokio::test]
    async fn cavectl_post_follows_503_location_to_leader() {
        // Two mock servers — follower returns 503 + Location pointing
        // at the leader; leader returns 200. cavectl should
        // automatically retry against the leader and report success.
        let follower = MockServer::start_async().await;
        let leader = MockServer::start_async().await;
        let leader_url = leader.base_url();

        let _f = follower
            .mock_async(|when, then| {
                when.method(POST).path("/api/etcd/v3/kv/put");
                then.status(503)
                    .header("location", &leader_url)
                    .body("not leader");
            })
            .await;
        let leader_mock = leader
            .mock_async(|when, then| {
                when.method(POST).path("/api/etcd/v3/kv/put");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"header":{"revision":7}}"#);
            })
            .await;

        let c = ApiClient::new(follower.base_url(), None, Format::Json);
        let body = serde_json::json!({"key": "x", "value": "y"});
        c.post("/api/etcd/v3/kv/put", body).await.unwrap();
        leader_mock.assert_async().await; // leader saw the retry
    }

    #[tokio::test]
    async fn cavectl_post_gives_up_after_three_retries() {
        // Follower returns 503 + Location pointing at ITSELF — so
        // every retry comes back here. After 3 retries cavectl
        // surfaces the final 503 instead of looping forever.
        let server = MockServer::start_async().await;
        let self_url = server.base_url();
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/p");
                then.status(503)
                    .header("location", &self_url)
                    .body("still not leader");
            })
            .await;

        let c = ApiClient::new(server.base_url(), None, Format::Json);
        // post() consumes the 503 and prints — but we want to assert
        // the request count, not the print output. The mock counts
        // each hit.
        let body = serde_json::json!({});
        c.post("/p", body).await.unwrap();
        // 1 initial + 3 retries = 4 hits.
        let hits = mock.hits_async().await;
        assert_eq!(hits, 4, "should attempt 1 + 3 retries on persistent 503");
    }

    #[tokio::test]
    async fn cavectl_post_does_not_retry_on_non_503() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/p");
                then.status(500).body("boom");
            })
            .await;
        let c = ApiClient::new(server.base_url(), None, Format::Json);
        c.post("/p", serde_json::json!({})).await.unwrap();
        // No retry on 500.
        assert_eq!(mock.hits_async().await, 1);
    }

    #[tokio::test]
    async fn cavectl_post_does_not_retry_on_503_without_location() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST).path("/p");
                then.status(503).body("not leader; leader_url=unknown");
            })
            .await;
        let c = ApiClient::new(server.base_url(), None, Format::Json);
        c.post("/p", serde_json::json!({})).await.unwrap();
        // Without a Location header there's nowhere to retry to.
        assert_eq!(mock.hits_async().await, 1);
    }
}
