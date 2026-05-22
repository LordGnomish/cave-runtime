// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub release poller.
//!
//! One method: `fetch_latest(client, repo)` → `PollOutcome`. The
//! client handles:
//!
//! * `Authorization: Bearer $token` (optional — falls back to
//!   anonymous 60 req/h limit).
//! * `User-Agent: cave-upstream-watchd/<version>` — required by
//!   GitHub's API ToS.
//! * `If-None-Match: <etag>` for 304 caching.
//! * `If-Modified-Since: <last_modified>` as secondary cache.
//! * Rate-limit header parsing — surfaces `X-RateLimit-Remaining`
//!   and `X-RateLimit-Reset` so callers can back off.
//! * 404 surfaced as `NoRelease` (the repo exists but has no
//!   releases yet — common for cave's tracked tools).
//!
//! Returns `PollOutcome::NewRelease { tag, body, .. }` on success
//! with a non-cached response. The caller pipes `body` into
//! `changelog::parse_release_body` to produce structured entries.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PollError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("missing required header: {0}")]
    MissingHeader(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestRelease {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub html_url: Option<String>,
    /// `true` for prereleases — caller decides whether to track or
    /// skip these (we don't filter here).
    #[serde(default)]
    pub prerelease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PollOutcome {
    /// Server returned 200 + fresh data. `etag` / `last_modified`
    /// are propagated so the caller can save them for next time.
    NewRelease {
        release: LatestRelease,
        etag: Option<String>,
        last_modified: Option<String>,
        rate_limit_remaining: Option<u32>,
    },
    /// Server returned 304 — our cached etag/last-modified still
    /// reflects reality. No body to update.
    NotModified { rate_limit_remaining: Option<u32> },
    /// Repo has no releases yet (404 on `/releases/latest`).
    NoRelease,
    /// Hit the rate-limit ceiling. `reset_at_epoch_seconds` is the
    /// `X-RateLimit-Reset` header.
    RateLimited { reset_at_epoch_seconds: Option<u64> },
}

#[derive(Debug, Clone)]
pub struct GitHubClient {
    inner: reqwest::Client,
    token: Option<String>,
    /// Override base URL — used by tests to point at a httpmock server.
    base_url: String,
}

impl GitHubClient {
    pub fn new(token: Option<String>) -> Self {
        Self::with_base_url(token, "https://api.github.com".to_string())
    }

    pub fn with_base_url(token: Option<String>, base_url: String) -> Self {
        let inner = reqwest::Client::builder()
            .user_agent(concat!("cave-upstream-watchd/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client");
        Self {
            inner,
            token,
            base_url,
        }
    }

    /// Fetch `/repos/{repo}/releases/latest` with conditional cache
    /// headers. Returns the structured outcome.
    pub async fn fetch_latest(
        &self,
        repo: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<PollOutcome, PollError> {
        let url = format!("{}/repos/{repo}/releases/latest", self.base_url);
        let mut req = self.inner.get(&url);
        req = req.header("Accept", "application/vnd.github+json");
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        if let Some(et) = etag {
            req = req.header("If-None-Match", et);
        }
        if let Some(lm) = last_modified {
            req = req.header("If-Modified-Since", lm);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let remaining = header_u32(&headers, "x-ratelimit-remaining");
        let reset = header_u32(&headers, "x-ratelimit-reset").map(|v| v as u64);

        // 403 with `X-RateLimit-Remaining: 0` is the rate-limited
        // path. 403 without that header is some other ACL failure and
        // we surface it as an http error.
        if status.as_u16() == 403 && remaining == Some(0) {
            return Ok(PollOutcome::RateLimited {
                reset_at_epoch_seconds: reset,
            });
        }
        if status.as_u16() == 304 {
            return Ok(PollOutcome::NotModified {
                rate_limit_remaining: remaining,
            });
        }
        if status.as_u16() == 404 {
            return Ok(PollOutcome::NoRelease);
        }
        // Surface other HTTP errors via error_for_status().
        let body = resp.error_for_status()?.text().await?;
        let release: LatestRelease = serde_json::from_str(&body)?;
        let etag_out = header_str(&headers, "etag");
        let lm_out = header_str(&headers, "last-modified");
        Ok(PollOutcome::NewRelease {
            release,
            etag: etag_out,
            last_modified: lm_out,
            rate_limit_remaining: remaining,
        })
    }
}

/// Top-level convenience: build a client + fetch in one call. Used
/// by `main.rs` to keep the binary small.
pub async fn fetch_latest(
    token: Option<String>,
    repo: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<PollOutcome, PollError> {
    GitHubClient::new(token)
        .fetch_latest(repo, etag, last_modified)
        .await
}

fn header_str(h: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}
fn header_u32(h: &reqwest::header::HeaderMap, name: &str) -> Option<u32> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::GET, MockServer};

    fn release_json(tag: &str, body: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "tag_name": tag,
            "name": tag,
            "body": body,
            "published_at": "2026-05-13T12:00:00Z",
            "html_url": format!("https://github.com/x/y/releases/{tag}"),
            "prerelease": false,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn fetch_latest_returns_new_release_with_etag_and_rate_limit() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/repos/etcd-io/etcd/releases/latest");
            then.status(200)
                .header("etag", "\"abc\"")
                .header("last-modified", "Wed, 13 May 2026 12:00:00 GMT")
                .header("x-ratelimit-remaining", "4990")
                .header("content-type", "application/json")
                .body(release_json("v3.5.13", "## Added\n- foo\n"));
        });
        let cli = GitHubClient::with_base_url(None, server.base_url());
        let outcome = cli.fetch_latest("etcd-io/etcd", None, None).await.unwrap();
        m.assert();
        match outcome {
            PollOutcome::NewRelease {
                release,
                etag,
                last_modified,
                rate_limit_remaining,
            } => {
                assert_eq!(release.tag_name, "v3.5.13");
                assert!(release.body.unwrap().contains("## Added"));
                assert_eq!(etag.as_deref(), Some("\"abc\""));
                assert!(last_modified.is_some());
                assert_eq!(rate_limit_remaining, Some(4990));
            }
            other => panic!("expected NewRelease, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_latest_returns_not_modified_on_304() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/etcd-io/etcd/releases/latest")
                .header("If-None-Match", "\"abc\"");
            then.status(304).header("x-ratelimit-remaining", "4989");
        });
        let cli = GitHubClient::with_base_url(None, server.base_url());
        let outcome = cli
            .fetch_latest("etcd-io/etcd", Some("\"abc\""), None)
            .await
            .unwrap();
        m.assert();
        assert!(matches!(outcome, PollOutcome::NotModified { .. }));
    }

    #[tokio::test]
    async fn fetch_latest_returns_no_release_on_404() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/repos/x/y/releases/latest");
            then.status(404);
        });
        let cli = GitHubClient::with_base_url(None, server.base_url());
        let outcome = cli.fetch_latest("x/y", None, None).await.unwrap();
        m.assert();
        assert!(matches!(outcome, PollOutcome::NoRelease));
    }

    #[tokio::test]
    async fn fetch_latest_returns_rate_limited_on_403_with_remaining_zero() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET).path("/repos/x/y/releases/latest");
            then.status(403)
                .header("x-ratelimit-remaining", "0")
                .header("x-ratelimit-reset", "1714665600");
        });
        let cli = GitHubClient::with_base_url(None, server.base_url());
        let outcome = cli.fetch_latest("x/y", None, None).await.unwrap();
        m.assert();
        match outcome {
            PollOutcome::RateLimited {
                reset_at_epoch_seconds,
            } => {
                assert_eq!(reset_at_epoch_seconds, Some(1714665600));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_latest_sends_user_agent_and_auth_when_token_present() {
        let server = MockServer::start();
        let m = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/x/y/releases/latest")
                .header("Authorization", "Bearer test-token");
            then.status(200)
                .header("content-type", "application/json")
                .body(release_json("v1.0.0", ""));
        });
        let cli = GitHubClient::with_base_url(Some("test-token".into()), server.base_url());
        let _ = cli.fetch_latest("x/y", None, None).await.unwrap();
        m.assert();
    }
}
