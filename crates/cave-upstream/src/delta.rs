//! Surface-delta computation for upstream releases.
//!
//! ## Phase 1 (this file): tag-level delta — production
//!
//! [`detect_release_delta`] queries the GitHub Releases API for a repo,
//! compares the latest tag against the last known tag in [`crate::state`],
//! and returns a [`ReleaseDelta`] describing the version transition. The
//! poll uses ETag / If-Modified-Since for conditional requests so that
//! unchanged repos do not consume rate-limit budget (304 responses are
//! free on the authenticated GitHub API).
//!
//! Tag-level detection is the *minimum useful* delta: it tells the pump
//! "version X.Y.Z just shipped, you should re-port". The Charter's
//! self-improving loop only requires that signal — the actual port work
//! is the Qwen pump's responsibility.
//!
//! ## Phase 2 (deferred — see ADR-RUNTIME-UPSTREAM-WATCH-001 §"Phase 2"):
//! source-level public-API surface diff
//!
//! Honest scope: implementing real public-API diffs across Go (`go doc`),
//! Java (`javap`), TypeScript (`.d.ts`), and Rust (`cargo public-api`)
//! requires (a) cloning each upstream source, (b) running language-
//! specific toolchains, and (c) caching by tag+SHA. That is multiple
//! weeks of work per ecosystem and is explicitly out of scope for the
//! daemon's first delivery.
//!
//! What is in this file is the [`SurfaceDiffer`] trait that Phase 2
//! plugs into without requiring any change to [`detect_release_delta`].
//! The default differ is [`TagOnlyDiffer`], which is the production
//! Phase-1 behavior.

use crate::state::ProjectState;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// One surface item — added/removed/changed by a release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceItem {
    /// Language-specific symbol path, e.g. `"clientv3.New"` or
    /// `"io.etcd.jetcd.Watch.watch"`. Free-form; the pump treats it as a
    /// label.
    pub symbol: String,
    /// `"function"`, `"type"`, `"const"`, `"interface"`, `"endpoint"`, etc.
    pub kind: String,
    /// Optional one-line note from the differ.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Surface diff between two tags. All vectors empty == no change at the
/// public-API level (or, for [`TagOnlyDiffer`], no source-level diff was
/// computed at all).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SurfaceDiff {
    pub added: Vec<SurfaceItem>,
    pub removed: Vec<SurfaceItem>,
    pub changed: Vec<SurfaceItem>,
}

impl SurfaceDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
    pub fn total(&self) -> usize {
        self.added.len() + self.removed.len() + self.changed.len()
    }
}

/// A full release-level delta, ready to be turned into a pump payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseDelta {
    pub github_repo: String,
    pub old_tag: Option<String>,
    pub new_tag: String,
    pub release_url: String,
    pub published_at: Option<DateTime<Utc>>,
    pub release_name: Option<String>,
    pub release_body: Option<String>,
    pub surface_diff: SurfaceDiff,
}

/// Outcome of a single release poll.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    /// New release detected — caller should write a pump payload.
    NewRelease(ReleaseDelta),
    /// No change since last poll (HTTP 304 or tag matches `last_known_tag`).
    Unchanged,
    /// Repo has no releases at all yet (empty array from API).
    NoReleases,
    /// We hit the rate limit. Caller should back off.
    RateLimited {
        /// Unix epoch second the limit resets, if known.
        reset_at: Option<i64>,
    },
}

#[derive(Debug, Error)]
pub enum DeltaError {
    #[error("github http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("github returned status {0}: {1}")]
    BadStatus(StatusCode, String),
    #[error("response parse error: {0}")]
    Parse(String),
}

/// Subset of the GitHub `/repos/{owner}/{repo}/releases/latest` schema we use.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<DateTime<Utc>>,
}

/// Pluggable source-level differ. Phase 2 plugs in language-specific
/// implementations behind this trait without touching the daemon loop.
///
/// The default implementation in this crate is [`TagOnlyDiffer`], which
/// returns an empty [`SurfaceDiff`] — the *tag itself* is the signal in
/// Phase 1.
#[async_trait]
pub trait SurfaceDiffer: Send + Sync {
    async fn diff(
        &self,
        github_repo: &str,
        old_tag: Option<&str>,
        new_tag: &str,
    ) -> SurfaceDiff;
}

/// Phase-1 default: emits an empty diff. The release tag transition is the
/// signal; the pump decides what to do with it.
pub struct TagOnlyDiffer;

#[async_trait]
impl SurfaceDiffer for TagOnlyDiffer {
    async fn diff(
        &self,
        _repo: &str,
        _old_tag: Option<&str>,
        _new_tag: &str,
    ) -> SurfaceDiff {
        SurfaceDiff::default()
    }
}

/// Configuration handed to [`detect_release_delta`].
pub struct PollConfig<'a> {
    pub github_api_base: &'a str,
    pub github_token: Option<&'a str>,
    pub user_agent: &'a str,
    pub request_timeout: Duration,
}

impl<'a> PollConfig<'a> {
    pub fn production(token: Option<&'a str>) -> Self {
        Self {
            github_api_base: "https://api.github.com",
            github_token: token,
            user_agent: "cave-upstream-watchd/1.0",
            request_timeout: Duration::from_secs(15),
        }
    }
}

fn build_headers(cfg: &PollConfig<'_>, st: &ProjectState) -> anyhow::Result<HeaderMap> {
    let mut h = HeaderMap::new();
    h.insert(USER_AGENT, HeaderValue::from_str(cfg.user_agent)?);
    h.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    h.insert(
        reqwest::header::HeaderName::from_static("x-github-api-version"),
        HeaderValue::from_static("2022-11-28"),
    );
    if let Some(t) = cfg.github_token {
        let val = format!("Bearer {t}");
        h.insert(reqwest::header::AUTHORIZATION, HeaderValue::from_str(&val)?);
    }
    if let Some(etag) = &st.etag {
        if let Ok(v) = HeaderValue::from_str(etag) {
            h.insert(IF_NONE_MATCH, v);
        }
    } else if let Some(lm) = &st.last_modified {
        if let Ok(v) = HeaderValue::from_str(lm) {
            h.insert(IF_MODIFIED_SINCE, v);
        }
    }
    Ok(h)
}

/// Hit `GET /repos/{repo}/releases/latest` and decide if it's new.
///
/// Mutates `state` to record the new ETag/Last-Modified/tag if changed.
/// Returns a [`PollOutcome`] the caller can act on.
pub async fn detect_release_delta(
    client: &Client,
    cfg: &PollConfig<'_>,
    differ: &dyn SurfaceDiffer,
    state: &mut ProjectState,
) -> Result<PollOutcome, DeltaError> {
    let url = format!(
        "{}/repos/{}/releases/latest",
        cfg.github_api_base.trim_end_matches('/'),
        state.github_repo
    );
    let headers = build_headers(cfg, state).map_err(|e| DeltaError::Parse(e.to_string()))?;
    let resp = client
        .get(&url)
        .headers(headers)
        .timeout(cfg.request_timeout)
        .send()
        .await?;

    let status = resp.status();
    if status == StatusCode::NOT_MODIFIED {
        // 304: nothing changed. Round-trip succeeded, so reset error counter.
        state.last_checked = Some(Utc::now());
        state.consecutive_errors = 0;
        return Ok(PollOutcome::Unchanged);
    }

    if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
        let reset_at = resp
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok());
        return Ok(PollOutcome::RateLimited { reset_at });
    }

    if status == StatusCode::NOT_FOUND {
        // Repo exists but has no published releases (or repo missing).
        // Caller will see this as "no signal".
        state.last_checked = Some(Utc::now());
        return Ok(PollOutcome::NoReleases);
    }

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let truncated = body.chars().take(200).collect::<String>();
        return Err(DeltaError::BadStatus(status, truncated));
    }

    // Capture caching headers BEFORE consuming body.
    let new_etag = resp
        .headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let new_last_modified = resp
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let release: GithubRelease = resp
        .json()
        .await
        .map_err(|e| DeltaError::Parse(format!("decode latest release: {e}")))?;

    state.last_checked = Some(Utc::now());
    state.etag = new_etag;
    state.last_modified = new_last_modified;
    state.consecutive_errors = 0;

    let new_tag = release.tag_name.clone();
    let old_tag = state.last_known_tag.clone();
    if old_tag.as_deref() == Some(new_tag.as_str()) {
        // Server returned 200 (no ETag echo) but tag is identical.
        return Ok(PollOutcome::Unchanged);
    }

    let surface_diff = differ
        .diff(&state.github_repo, old_tag.as_deref(), &new_tag)
        .await;

    state.last_known_tag = Some(new_tag.clone());
    state.last_delta_summary = Some(format!(
        "{} -> {} ({} surface changes)",
        old_tag.as_deref().unwrap_or("∅"),
        new_tag,
        surface_diff.total()
    ));

    Ok(PollOutcome::NewRelease(ReleaseDelta {
        github_repo: state.github_repo.clone(),
        old_tag,
        new_tag,
        release_url: release.html_url,
        published_at: release.published_at,
        release_name: release.name,
        release_body: release.body,
        surface_diff,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    fn ua() -> &'static str {
        "cave-upstream-watchd/test"
    }

    #[tokio::test]
    async fn first_poll_with_release_returns_new_release() {
        let mut server = Server::new_async().await;
        let body = r#"{
            "tag_name": "v3.5.10",
            "name": "v3.5.10",
            "body": "fixes",
            "html_url": "https://github.com/etcd-io/etcd/releases/tag/v3.5.10",
            "published_at": "2026-04-01T00:00:00Z"
        }"#;
        let _m = server
            .mock("GET", "/repos/etcd-io/etcd/releases/latest")
            .with_status(200)
            .with_header("etag", "\"abc\"")
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let cfg = PollConfig {
            github_api_base: &server.url(),
            github_token: None,
            user_agent: ua(),
            request_timeout: Duration::from_secs(5),
        };
        let client = Client::new();
        let mut st = ProjectState::new("etcd-io/etcd");

        let outcome =
            detect_release_delta(&client, &cfg, &TagOnlyDiffer, &mut st).await.unwrap();
        match outcome {
            PollOutcome::NewRelease(d) => {
                assert_eq!(d.new_tag, "v3.5.10");
                assert_eq!(d.old_tag, None);
            }
            other => panic!("expected NewRelease, got {other:?}"),
        }
        assert_eq!(st.last_known_tag.as_deref(), Some("v3.5.10"));
        assert_eq!(st.etag.as_deref(), Some("\"abc\""));
        assert_eq!(st.consecutive_errors, 0);
    }

    #[tokio::test]
    async fn second_poll_with_same_tag_returns_unchanged() {
        let mut server = Server::new_async().await;
        let body = r#"{
            "tag_name": "v3.5.10",
            "html_url": "https://github.com/etcd-io/etcd/releases/tag/v3.5.10"
        }"#;
        let _m = server
            .mock("GET", "/repos/etcd-io/etcd/releases/latest")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        let cfg = PollConfig {
            github_api_base: &server.url(),
            github_token: None,
            user_agent: ua(),
            request_timeout: Duration::from_secs(5),
        };
        let mut st = ProjectState::new("etcd-io/etcd");
        st.last_known_tag = Some("v3.5.10".to_string());

        let outcome =
            detect_release_delta(&Client::new(), &cfg, &TagOnlyDiffer, &mut st).await.unwrap();
        assert!(matches!(outcome, PollOutcome::Unchanged));
    }

    #[tokio::test]
    async fn three_oh_four_returns_unchanged_and_does_not_clear_state() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/repos/etcd-io/etcd/releases/latest")
            .match_header("if-none-match", "\"abc\"")
            .with_status(304)
            .create_async()
            .await;

        let cfg = PollConfig {
            github_api_base: &server.url(),
            github_token: None,
            user_agent: ua(),
            request_timeout: Duration::from_secs(5),
        };
        let mut st = ProjectState::new("etcd-io/etcd");
        st.etag = Some("\"abc\"".to_string());
        st.last_known_tag = Some("v3.5.9".to_string());

        let outcome =
            detect_release_delta(&Client::new(), &cfg, &TagOnlyDiffer, &mut st).await.unwrap();
        assert!(matches!(outcome, PollOutcome::Unchanged));
        // ETag and last_known_tag preserved on 304.
        assert_eq!(st.etag.as_deref(), Some("\"abc\""));
        assert_eq!(st.last_known_tag.as_deref(), Some("v3.5.9"));
    }

    #[tokio::test]
    async fn rate_limit_returns_rate_limited_with_reset() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/repos/foo/bar/releases/latest")
            .with_status(403)
            .with_header("x-ratelimit-reset", "1735689600")
            .with_body("rate limited")
            .create_async()
            .await;

        let cfg = PollConfig {
            github_api_base: &server.url(),
            github_token: None,
            user_agent: ua(),
            request_timeout: Duration::from_secs(5),
        };
        let mut st = ProjectState::new("foo/bar");
        let outcome =
            detect_release_delta(&Client::new(), &cfg, &TagOnlyDiffer, &mut st).await.unwrap();
        match outcome {
            PollOutcome::RateLimited { reset_at } => {
                assert_eq!(reset_at, Some(1735689600));
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_releases_yet_returns_no_releases() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("GET", "/repos/foo/empty/releases/latest")
            .with_status(404)
            .with_body(r#"{"message":"Not Found"}"#)
            .create_async()
            .await;

        let cfg = PollConfig {
            github_api_base: &server.url(),
            github_token: None,
            user_agent: ua(),
            request_timeout: Duration::from_secs(5),
        };
        let mut st = ProjectState::new("foo/empty");
        let outcome =
            detect_release_delta(&Client::new(), &cfg, &TagOnlyDiffer, &mut st).await.unwrap();
        assert!(matches!(outcome, PollOutcome::NoReleases));
    }

    #[tokio::test]
    async fn tag_transition_preserves_old_tag_in_delta() {
        let mut server = Server::new_async().await;
        let body = r#"{
            "tag_name": "v3.6.0",
            "html_url": "https://github.com/etcd-io/etcd/releases/tag/v3.6.0"
        }"#;
        let _m = server
            .mock("GET", "/repos/etcd-io/etcd/releases/latest")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        let cfg = PollConfig {
            github_api_base: &server.url(),
            github_token: None,
            user_agent: ua(),
            request_timeout: Duration::from_secs(5),
        };
        let mut st = ProjectState::new("etcd-io/etcd");
        st.last_known_tag = Some("v3.5.10".to_string());

        let outcome =
            detect_release_delta(&Client::new(), &cfg, &TagOnlyDiffer, &mut st).await.unwrap();
        match outcome {
            PollOutcome::NewRelease(d) => {
                assert_eq!(d.old_tag.as_deref(), Some("v3.5.10"));
                assert_eq!(d.new_tag, "v3.6.0");
            }
            other => panic!("expected NewRelease, got {other:?}"),
        }
        assert_eq!(st.last_known_tag.as_deref(), Some("v3.6.0"));
    }
}

// PollOutcome needs Debug for panic-formatting in test failures, but we
// don't want it derivable on the public type because it gets noisy in
// tracing output. Provide a manual impl gated to tests.
#[cfg(test)]
impl std::fmt::Debug for PollOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollOutcome::NewRelease(d) => write!(f, "NewRelease({} -> {})", d.old_tag.as_deref().unwrap_or("∅"), d.new_tag),
            PollOutcome::Unchanged => write!(f, "Unchanged"),
            PollOutcome::NoReleases => write!(f, "NoReleases"),
            PollOutcome::RateLimited { reset_at } => write!(f, "RateLimited(reset_at={reset_at:?})"),
        }
    }
}
