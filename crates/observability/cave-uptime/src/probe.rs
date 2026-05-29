// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Probe execution: HTTP, TCP, DNS, Push monitoring implementations.
//!
//! Maps to Uptime Kuma's monitor execution backends.

use crate::models::{ProbeResult, ProbeType, UptimeProbe};
use chrono::Utc;
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};
use thiserror::Error;
use uuid::Uuid;

// ─── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur during probe execution.
#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("timeout after {0}")]
    Timeout(String),
    #[error("connection failed: {0}")]
    ConnectionFailed(String),
    #[error("DNS resolution failed: {0}")]
    DnsResolutionFailed(String),
    #[error("HTTP error: {0}")]
    HttpError(String),
    #[error("keyword not found: {0}")]
    KeywordNotFound(String),
    #[error("unexpected status {actual}, expected {expected}")]
    StatusMismatch { expected: u16, actual: u16 },
}

// ─── HTTP probe ───────────────────────────────────────────────────────────────

/// Configuration for an HTTP/HTTPS probe.
#[derive(Debug, Clone)]
pub struct HttpProbeConfig {
    pub url: String,
    pub timeout_ms: u32,
    pub method: String,
    pub keyword: Option<String>,
    pub expected_status: Option<u16>,
    pub follow_redirects: bool,
}

impl HttpProbeConfig {
    /// Create a GET probe with sensible defaults (matches Uptime Kuma defaults).
    pub fn new(url: String) -> Self {
        HttpProbeConfig {
            url,
            timeout_ms: 5000,
            method: "GET".to_string(),
            keyword: None,
            expected_status: None,
            follow_redirects: true,
        }
    }
}

/// Execute an HTTP/HTTPS probe and return a `ProbeResult`.
///
/// Uses `reqwest` with a strict timeout. If `config.keyword` is set the
/// response body is checked for the substring. If `config.expected_status`
/// is set the HTTP status code must match exactly.
pub async fn execute_http_probe(probe_id: Uuid, config: &HttpProbeConfig) -> ProbeResult {
    let start = Instant::now();
    let timeout = Duration::from_millis(config.timeout_ms as u64);

    let client = match reqwest::Client::builder()
        .timeout(timeout)
        .redirect(if config.follow_redirects {
            reqwest::redirect::Policy::limited(10)
        } else {
            reqwest::redirect::Policy::none()
        })
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return build_probe_result(
                probe_id,
                false,
                0,
                None,
                Some(format!("client build error: {e}")),
            );
        }
    };

    let method = match reqwest::Method::from_bytes(config.method.as_bytes()) {
        Ok(m) => m,
        Err(e) => {
            return build_probe_result(
                probe_id,
                false,
                0,
                None,
                Some(format!("invalid HTTP method: {e}")),
            );
        }
    };

    let request = match client.request(method, &config.url).build() {
        Ok(r) => r,
        Err(e) => {
            return build_probe_result(
                probe_id,
                false,
                0,
                None,
                Some(format!("request build error: {e}")),
            );
        }
    };

    match client.execute(request).await {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            let status = resp.status().as_u16();

            // Status-code check (if configured)
            if let Some(expected) = config.expected_status {
                if status != expected {
                    return build_probe_result(
                        probe_id,
                        false,
                        latency,
                        Some(status),
                        Some(format!(
                            "status mismatch: expected {expected}, got {status}"
                        )),
                    );
                }
            }

            // Keyword check (if configured)
            if let Some(kw) = &config.keyword {
                match resp.text().await {
                    Ok(body) if body.contains(kw.as_str()) => {
                        build_probe_result(probe_id, true, latency, Some(status), None)
                    }
                    Ok(_) => build_probe_result(
                        probe_id,
                        false,
                        latency,
                        Some(status),
                        Some(format!("keyword '{kw}' not found in response body")),
                    ),
                    Err(e) => build_probe_result(
                        probe_id,
                        false,
                        latency,
                        Some(status),
                        Some(format!("body read error: {e}")),
                    ),
                }
            } else {
                // Treat 2xx/3xx as success (matches Uptime Kuma behaviour)
                let success = (200..400).contains(&(status as u32));
                build_probe_result(
                    probe_id,
                    success,
                    latency,
                    Some(status),
                    if success {
                        None
                    } else {
                        Some(format!("HTTP {status}"))
                    },
                )
            }
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            let msg = if e.is_timeout() {
                format!("timeout after {}ms", config.timeout_ms)
            } else if e.is_connect() {
                format!("connection failed: {e}")
            } else {
                format!("request error: {e}")
            };
            build_probe_result(probe_id, false, latency, None, Some(msg))
        }
    }
}

// ─── TCP probe ────────────────────────────────────────────────────────────────

/// Execute a TCP port-open probe.
///
/// Attempts `tokio::net::TcpStream::connect` within the given timeout.
/// If the connection succeeds (even if immediately closed by the remote)
/// the probe is considered UP.
pub async fn execute_tcp_probe(
    probe_id: Uuid,
    host: &str,
    port: u16,
    timeout_ms: u32,
) -> ProbeResult {
    let addr = format!("{host}:{port}");
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms as u64);

    // Resolve to socket addresses (blocking, run on thread pool)
    let addr_owned = addr.clone();
    let addrs: Vec<std::net::SocketAddr> = match tokio::task::spawn_blocking(move || {
        addr_owned.to_socket_addrs().map(|i| i.collect::<Vec<_>>())
    })
    .await
    {
        Ok(Ok(a)) if !a.is_empty() => a,
        Ok(Ok(_)) => {
            return build_probe_result(
                probe_id,
                false,
                start.elapsed().as_millis() as u64,
                None,
                Some(format!("no addresses resolved for {host}")),
            );
        }
        Ok(Err(e)) => {
            return build_probe_result(
                probe_id,
                false,
                start.elapsed().as_millis() as u64,
                None,
                Some(format!("DNS resolution failed: {e}")),
            );
        }
        Err(e) => {
            return build_probe_result(
                probe_id,
                false,
                start.elapsed().as_millis() as u64,
                None,
                Some(format!("resolve task failed: {e}")),
            );
        }
    };

    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addrs[0])).await {
        Ok(Ok(_stream)) => {
            build_probe_result(probe_id, true, start.elapsed().as_millis() as u64, None, None)
        }
        Ok(Err(e)) => build_probe_result(
            probe_id,
            false,
            start.elapsed().as_millis() as u64,
            None,
            Some(format!("TCP connect failed: {e}")),
        ),
        Err(_) => build_probe_result(
            probe_id,
            false,
            timeout_ms as u64,
            None,
            Some(format!("TCP timeout after {timeout_ms}ms")),
        ),
    }
}

// ─── DNS probe ────────────────────────────────────────────────────────────────

/// Execute a DNS resolution probe.
///
/// Uses the system resolver (via `ToSocketAddrs` on a blocking thread).
/// At least one resolved address is considered UP.
pub async fn execute_dns_probe(probe_id: Uuid, hostname: &str, timeout_ms: u32) -> ProbeResult {
    let start = Instant::now();
    let hostname_owned = hostname.to_string();
    let timeout = Duration::from_millis(timeout_ms as u64);

    let resolve_future = tokio::task::spawn_blocking(move || {
        format!("{hostname_owned}:0").to_socket_addrs()
    });

    match tokio::time::timeout(timeout, resolve_future).await {
        Ok(Ok(Ok(mut addrs))) => {
            let latency = start.elapsed().as_millis() as u64;
            if addrs.next().is_some() {
                build_probe_result(probe_id, true, latency, None, None)
            } else {
                build_probe_result(
                    probe_id,
                    false,
                    latency,
                    None,
                    Some(format!("DNS returned 0 records for {hostname}")),
                )
            }
        }
        Ok(Ok(Err(e))) => build_probe_result(
            probe_id,
            false,
            start.elapsed().as_millis() as u64,
            None,
            Some(format!("DNS resolution error: {e}")),
        ),
        Ok(Err(e)) => build_probe_result(
            probe_id,
            false,
            start.elapsed().as_millis() as u64,
            None,
            Some(format!("task join error: {e}")),
        ),
        Err(_) => build_probe_result(
            probe_id,
            false,
            timeout_ms as u64,
            None,
            Some(format!("DNS timeout after {timeout_ms}ms")),
        ),
    }
}

// ─── Push probe (passive heartbeat) ──────────────────────────────────────────

/// Evaluate a push (passive heartbeat) probe.
///
/// The remote service calls back to `/api/uptime/push/<slug>` instead of
/// cave-uptime polling it. This function checks whether the last heartbeat
/// arrived within `interval_seconds + 30s` grace period.
pub fn evaluate_push_probe(
    probe_id: Uuid,
    last_push_at: Option<i64>,
    interval_seconds: u32,
) -> ProbeResult {
    let now = Utc::now().timestamp();
    match last_push_at {
        None => build_probe_result(
            probe_id,
            false,
            0,
            None,
            Some("no push received yet".to_string()),
        ),
        Some(ts) => {
            let age = now.saturating_sub(ts);
            let grace = (interval_seconds as i64) + 30;
            let ok = age <= grace;
            build_probe_result(
                probe_id,
                ok,
                0,
                None,
                if ok {
                    None
                } else {
                    Some(format!(
                        "push overdue: last push {age}s ago, interval {interval_seconds}s"
                    ))
                },
            )
        }
    }
}

// ─── Dispatch: run any probe ──────────────────────────────────────────────────

/// Dispatch the correct probe executor for a given `UptimeProbe`.
///
/// This is the entry point called by the scheduler.
pub async fn run_probe(probe: &UptimeProbe) -> ProbeResult {
    match probe.probe_type {
        ProbeType::Http => {
            let cfg = HttpProbeConfig {
                url: probe.target_url.clone(),
                timeout_ms: probe.timeout_ms,
                method: "GET".to_string(),
                keyword: None,
                expected_status: None,
                follow_redirects: true,
            };
            execute_http_probe(probe.id, &cfg).await
        }
        ProbeType::Tcp => {
            let (host, port) = parse_host_port(&probe.target_url).unwrap_or(("localhost", 80));
            // `host` borrows from `probe.target_url` — need owned copy
            let host_owned = host.to_string();
            execute_tcp_probe(probe.id, &host_owned, port, probe.timeout_ms).await
        }
        ProbeType::Dns => {
            let host = probe
                .target_url
                .trim_start_matches("dns://")
                .split(':')
                .next()
                .unwrap_or(&probe.target_url);
            let host_owned = host.to_string();
            execute_dns_probe(probe.id, &host_owned, probe.timeout_ms).await
        }
        ProbeType::Ping => {
            // ICMP ping requires raw sockets and elevated privileges —
            // scope-cut to TCP:80 connectivity check as proxy.
            let host = probe.target_url.trim_start_matches("ping://").to_string();
            execute_tcp_probe(probe.id, &host, 80, probe.timeout_ms).await
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Construct a `ProbeResult` from raw components.
pub fn build_probe_result(
    probe_id: Uuid,
    success: bool,
    latency_ms: u64,
    status_code: Option<u16>,
    error: Option<String>,
) -> ProbeResult {
    ProbeResult {
        probe_id,
        success,
        latency_ms,
        status_code,
        error,
        checked_at: Utc::now(),
    }
}

/// Parse `host:port` from a URL-like string, stripping common schemes.
pub fn parse_host_port(target: &str) -> Option<(&str, u16)> {
    let bare = target
        .trim_start_matches("tcp://")
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let (host, port_str) = bare.rsplit_once(':')?;
    let port: u16 = port_str.trim_end_matches('/').parse().ok()?;
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_port_plain() {
        let (h, p) = parse_host_port("example.com:8080").unwrap();
        assert_eq!(h, "example.com");
        assert_eq!(p, 8080);
    }

    #[test]
    fn parse_host_port_with_scheme() {
        let (h, p) = parse_host_port("tcp://db.internal:5432").unwrap();
        assert_eq!(h, "db.internal");
        assert_eq!(p, 5432);
    }

    #[test]
    fn parse_host_port_none_for_no_colon() {
        assert!(parse_host_port("example.com").is_none());
    }

    #[test]
    fn build_result_success_fields() {
        let id = Uuid::new_v4();
        let r = build_probe_result(id, true, 42, Some(200), None);
        assert!(r.success);
        assert_eq!(r.latency_ms, 42);
        assert_eq!(r.status_code, Some(200));
        assert!(r.error.is_none());
    }

    #[test]
    fn push_probe_overdue_msg() {
        let id = Uuid::new_v4();
        let now = Utc::now().timestamp();
        let r = evaluate_push_probe(id, Some(now - 200), 60);
        assert!(!r.success);
        let msg = r.error.unwrap();
        assert!(msg.contains("overdue"), "expected 'overdue' in '{msg}'");
    }
}
