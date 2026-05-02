use anyhow::{Context, Result};
use colored::Colorize;
use reqwest::{Client, Method, StatusCode};
use serde_json::Value;

use crate::Format;

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
                    eprintln!("{} HTTP {}: {}", "✗".red().bold(), status.as_u16(), body.trim());
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
            .build(Method::POST, path)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}{}", self.base_url, path))?;
        self.finish(resp).await
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let resp = self
            .build(Method::DELETE, path)
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}{}", self.base_url, path))?;
        self.finish(resp).await
    }

    /// PUT raw bytes (used for Nexus raw asset upload).
    pub async fn put_bytes(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let resp = self
            .build(Method::PUT, path)
            .body(body)
            .send()
            .await
            .with_context(|| format!("Failed to connect to {}{}", self.base_url, path))?;
        self.finish(resp).await
    }

    fn print(&self, value: &Value) {
        match &self.format {
            Format::Json => println!("{}", serde_json::to_string_pretty(value).unwrap_or_default()),
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
