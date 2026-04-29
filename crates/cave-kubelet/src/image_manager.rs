//! Image manager — image pulls, secret resolution, parallel-pull throttling.
//!
//! Mirrors `pkg/kubelet/images/image_manager.go`: pull-policy gating
//! (Always / IfNotPresent / Never), secret-based credential resolution,
//! parallel-pull throttling via the `--max-parallel-image-pulls` setting,
//! pull-progress tracking, and dedup of concurrent pulls of the same
//! image (multiple pods waiting on a single in-flight pull).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImagePullPolicy {
    /// Always pull the image (default for `:latest`).
    Always,
    /// Pull only if the image is not present locally.
    IfNotPresent,
    /// Never pull; fail if not present.
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    pub repository: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

impl ImageRef {
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        // digest reference: repo@sha256:...
        if let Some(idx) = s.find('@') {
            let repo = &s[..idx];
            let dig = &s[idx + 1..];
            if repo.is_empty() || dig.is_empty() {
                return None;
            }
            return Some(Self {
                repository: repo.into(),
                tag: None,
                digest: Some(dig.into()),
            });
        }
        // tag reference: repo:tag
        if let Some(idx) = s.rfind(':') {
            // Distinguish ":port" in registry hostname from ":tag" — if there's
            // a "/" after the colon, it's port.
            let after = &s[idx + 1..];
            if !after.contains('/') {
                return Some(Self {
                    repository: s[..idx].into(),
                    tag: Some(after.into()),
                    digest: None,
                });
            }
        }
        Some(Self {
            repository: s.into(),
            tag: None,
            digest: None,
        })
    }

    pub fn is_latest(&self) -> bool {
        self.tag.as_deref() == Some("latest")
    }

    pub fn key(&self) -> String {
        match (&self.tag, &self.digest) {
            (_, Some(d)) => format!("{}@{}", self.repository, d),
            (Some(t), None) => format!("{}:{}", self.repository, t),
            (None, None) => self.repository.clone(),
        }
    }
}

/// Default pull-policy resolution per upstream `pkg/api/v1/pod/util.go`:
/// `latest` tag → Always; explicit tag/digest → IfNotPresent.
pub fn default_pull_policy(image: &ImageRef) -> ImagePullPolicy {
    if image.is_latest() {
        ImagePullPolicy::Always
    } else {
        ImagePullPolicy::IfNotPresent
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePullSecret {
    pub name: String,
    pub server: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullState {
    Queued,
    InFlight,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullStatus {
    pub image_key: String,
    pub state: PullState,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub bytes_pulled: u64,
    pub bytes_total: u64,
    pub error_message: Option<String>,
    /// pods waiting on this pull.
    pub waiters: BTreeSet<String>,
    pub attempts: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("policy=Never and image not present: {0}")]
    NotPresentAndNeverPull(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("backoff: pull failed recently for {0}")]
    PullBackoff(String),
}

pub type ImageResult<T> = Result<T, ImageError>;

#[derive(Debug, Clone, Copy)]
pub struct ImageManagerConfig {
    pub max_parallel_pulls: usize,
    pub pull_failure_backoff: Duration,
}

impl Default for ImageManagerConfig {
    fn default() -> Self {
        Self {
            max_parallel_pulls: 1,
            pull_failure_backoff: Duration::seconds(30),
        }
    }
}

#[derive(Debug)]
pub struct ImageManager {
    pub cfg: ImageManagerConfig,
    /// Locally-present image keys.
    present: BTreeSet<String>,
    /// Per-image pull status.
    pulls: BTreeMap<String, PullStatus>,
    /// Pods that have been recorded as referring to images (for GC anchoring).
    image_refs_by_pod: BTreeMap<String, BTreeSet<String>>,
    /// Pull queue (FIFO).
    queue: VecDeque<String>,
    /// In-flight count.
    in_flight: usize,
}

impl ImageManager {
    pub fn new(cfg: ImageManagerConfig) -> Self {
        Self {
            cfg,
            present: BTreeSet::new(),
            pulls: BTreeMap::new(),
            image_refs_by_pod: BTreeMap::new(),
            queue: VecDeque::new(),
            in_flight: 0,
        }
    }

    pub fn record_present(&mut self, image_key: &str) {
        self.present.insert(image_key.to_string());
    }

    pub fn forget(&mut self, image_key: &str) {
        self.present.remove(image_key);
        self.pulls.remove(image_key);
    }

    pub fn is_present(&self, image_key: &str) -> bool {
        self.present.contains(image_key)
    }

    pub fn pull_status(&self, image_key: &str) -> Option<&PullStatus> {
        self.pulls.get(image_key)
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.len()
    }

    pub fn refs_by_pod(&self, pod_uid: &str) -> Vec<String> {
        self.image_refs_by_pod
            .get(pod_uid)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Determine whether a pull should be initiated and if so, queue it.
    /// Returns whether the pod should be admitted to wait for the image.
    pub fn ensure_image(
        &mut self,
        image: &ImageRef,
        policy: ImagePullPolicy,
        pod_uid: &str,
        now: DateTime<Utc>,
    ) -> ImageResult<EnsureOutcome> {
        let key = image.key();
        let present = self.is_present(&key);
        // Track pod → image binding so GC can compute refcounts.
        self.image_refs_by_pod
            .entry(pod_uid.to_string())
            .or_default()
            .insert(key.clone());
        match policy {
            ImagePullPolicy::Never => {
                if present {
                    Ok(EnsureOutcome::AlreadyPresent)
                } else {
                    Err(ImageError::NotPresentAndNeverPull(key))
                }
            }
            ImagePullPolicy::IfNotPresent if present => Ok(EnsureOutcome::AlreadyPresent),
            _ => self.start_or_join_pull(&key, pod_uid, now),
        }
    }

    fn start_or_join_pull(
        &mut self,
        image_key: &str,
        pod_uid: &str,
        now: DateTime<Utc>,
    ) -> ImageResult<EnsureOutcome> {
        // If there's already an in-flight or queued pull, attach as waiter.
        if let Some(status) = self.pulls.get_mut(image_key) {
            if matches!(status.state, PullState::InFlight | PullState::Queued) {
                status.waiters.insert(pod_uid.to_string());
                return Ok(EnsureOutcome::Pulling);
            }
            // Failed pull within backoff window?
            if status.state == PullState::Failed {
                let backoff_until = status
                    .completed_at
                    .map(|t| t + self.cfg.pull_failure_backoff);
                if let Some(t) = backoff_until {
                    if now < t {
                        return Err(ImageError::PullBackoff(image_key.into()));
                    }
                }
                status.state = PullState::Queued;
                status.error_message = None;
                status.bytes_pulled = 0;
                status.attempts += 1;
                status.waiters.insert(pod_uid.to_string());
                self.queue.push_back(image_key.to_string());
                self.maybe_admit_next(now);
                return Ok(EnsureOutcome::Pulling);
            }
        }
        // Fresh pull.
        let mut waiters = BTreeSet::new();
        waiters.insert(pod_uid.to_string());
        let mut attempts = 1;
        if let Some(prev) = self.pulls.get(image_key) {
            attempts = prev.attempts + 1;
        }
        self.pulls.insert(
            image_key.to_string(),
            PullStatus {
                image_key: image_key.to_string(),
                state: PullState::Queued,
                started_at: None,
                completed_at: None,
                bytes_pulled: 0,
                bytes_total: 0,
                error_message: None,
                waiters,
                attempts,
            },
        );
        self.queue.push_back(image_key.to_string());
        self.maybe_admit_next(now);
        Ok(EnsureOutcome::Pulling)
    }

    fn maybe_admit_next(&mut self, now: DateTime<Utc>) {
        while self.in_flight < self.cfg.max_parallel_pulls {
            let key = match self.queue.pop_front() {
                None => break,
                Some(k) => k,
            };
            if let Some(status) = self.pulls.get_mut(&key) {
                if status.state == PullState::Queued {
                    status.state = PullState::InFlight;
                    status.started_at = Some(now);
                    self.in_flight += 1;
                }
            }
        }
    }

    /// Driver-side: report a successful pull.
    pub fn record_pull_success(
        &mut self,
        image_key: &str,
        bytes: u64,
        now: DateTime<Utc>,
    ) -> ImageResult<()> {
        let status = self
            .pulls
            .get_mut(image_key)
            .ok_or_else(|| ImageError::NotFound(image_key.into()))?;
        status.state = PullState::Succeeded;
        status.bytes_pulled = bytes;
        status.bytes_total = bytes;
        status.completed_at = Some(now);
        if self.in_flight > 0 {
            self.in_flight -= 1;
        }
        self.present.insert(image_key.to_string());
        self.maybe_admit_next(now);
        Ok(())
    }

    pub fn record_pull_failure(
        &mut self,
        image_key: &str,
        err: &str,
        now: DateTime<Utc>,
    ) -> ImageResult<()> {
        let status = self
            .pulls
            .get_mut(image_key)
            .ok_or_else(|| ImageError::NotFound(image_key.into()))?;
        status.state = PullState::Failed;
        status.error_message = Some(err.into());
        status.completed_at = Some(now);
        if self.in_flight > 0 {
            self.in_flight -= 1;
        }
        self.maybe_admit_next(now);
        Ok(())
    }

    /// Update progress for an in-flight pull.
    pub fn update_pull_progress(
        &mut self,
        image_key: &str,
        pulled: u64,
        total: u64,
    ) -> ImageResult<()> {
        let status = self
            .pulls
            .get_mut(image_key)
            .ok_or_else(|| ImageError::NotFound(image_key.into()))?;
        status.bytes_pulled = pulled;
        status.bytes_total = total;
        Ok(())
    }

    pub fn drop_pod(&mut self, pod_uid: &str) {
        self.image_refs_by_pod.remove(pod_uid);
        for status in self.pulls.values_mut() {
            status.waiters.remove(pod_uid);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureOutcome {
    AlreadyPresent,
    Pulling,
}

/// Resolve the credential for a pull: first match by exact server, then by
/// hostname suffix. Returns None if no secret matches.
pub fn resolve_credential<'a>(
    image_repository: &str,
    secrets: &'a [ImagePullSecret],
) -> Option<&'a ImagePullSecret> {
    let host = image_repository.split('/').next().unwrap_or(image_repository);
    // Exact match first.
    if let Some(s) = secrets.iter().find(|s| s.server == host) {
        return Some(s);
    }
    // Suffix match (registry-1.docker.io for docker.io/...).
    secrets
        .iter()
        .find(|s| host.ends_with(s.server.as_str()) && !s.server.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(s, 0).unwrap()
    }

    fn img(s: &str) -> ImageRef {
        ImageRef::parse(s).unwrap()
    }

    #[test]
    fn parse_image_with_tag() {
        let r = ImageRef::parse("nginx:1.25").unwrap();
        assert_eq!(r.repository, "nginx");
        assert_eq!(r.tag.as_deref(), Some("1.25"));
        assert!(r.digest.is_none());
    }

    #[test]
    fn parse_image_with_digest() {
        let r = ImageRef::parse("nginx@sha256:abcd").unwrap();
        assert_eq!(r.repository, "nginx");
        assert!(r.tag.is_none());
        assert_eq!(r.digest.as_deref(), Some("sha256:abcd"));
    }

    #[test]
    fn parse_image_no_tag_no_digest() {
        let r = ImageRef::parse("nginx").unwrap();
        assert_eq!(r.repository, "nginx");
        assert!(r.tag.is_none());
        assert!(r.digest.is_none());
    }

    #[test]
    fn parse_image_with_registry_port_no_tag() {
        // registry.example:5000/repo
        let r = ImageRef::parse("registry.example:5000/repo").unwrap();
        assert_eq!(r.repository, "registry.example:5000/repo");
        assert!(r.tag.is_none());
    }

    #[test]
    fn parse_image_with_registry_port_and_tag() {
        let r = ImageRef::parse("registry.example:5000/repo:v1").unwrap();
        assert_eq!(r.repository, "registry.example:5000/repo");
        assert_eq!(r.tag.as_deref(), Some("v1"));
    }

    #[test]
    fn parse_image_empty_returns_none() {
        assert!(ImageRef::parse("").is_none());
    }

    #[test]
    fn parse_image_digest_with_empty_digest_returns_none() {
        assert!(ImageRef::parse("repo@").is_none());
    }

    #[test]
    fn is_latest_recognised() {
        assert!(img("nginx:latest").is_latest());
        assert!(!img("nginx:1.25").is_latest());
        assert!(!img("nginx").is_latest());
    }

    #[test]
    fn key_uses_digest_when_present() {
        let r = img("nginx@sha256:abcd");
        assert_eq!(r.key(), "nginx@sha256:abcd");
    }

    #[test]
    fn key_uses_tag_when_no_digest() {
        let r = img("nginx:1.25");
        assert_eq!(r.key(), "nginx:1.25");
    }

    #[test]
    fn key_repo_only_when_neither_set() {
        let r = img("nginx");
        assert_eq!(r.key(), "nginx");
    }

    #[test]
    fn default_pull_policy_latest_is_always() {
        assert_eq!(default_pull_policy(&img("nginx:latest")), ImagePullPolicy::Always);
    }

    #[test]
    fn default_pull_policy_pinned_tag_is_if_not_present() {
        assert_eq!(default_pull_policy(&img("nginx:1.25")), ImagePullPolicy::IfNotPresent);
    }

    #[test]
    fn default_pull_policy_digest_is_if_not_present() {
        assert_eq!(default_pull_policy(&img("nginx@sha256:abc")), ImagePullPolicy::IfNotPresent);
    }

    #[test]
    fn ensure_never_when_present_returns_already_present() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.record_present("nginx:1.25");
        let outcome = m
            .ensure_image(&img("nginx:1.25"), ImagePullPolicy::Never, "p", t(0))
            .unwrap();
        assert_eq!(outcome, EnsureOutcome::AlreadyPresent);
    }

    #[test]
    fn ensure_never_when_absent_errors() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        let err = m
            .ensure_image(&img("nginx:1.25"), ImagePullPolicy::Never, "p", t(0))
            .unwrap_err();
        assert!(matches!(err, ImageError::NotPresentAndNeverPull(_)));
    }

    #[test]
    fn ensure_if_not_present_when_present_returns_already_present() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.record_present("nginx:1.25");
        let outcome = m
            .ensure_image(&img("nginx:1.25"), ImagePullPolicy::IfNotPresent, "p", t(0))
            .unwrap();
        assert_eq!(outcome, EnsureOutcome::AlreadyPresent);
    }

    #[test]
    fn ensure_if_not_present_when_absent_queues_pull() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        let outcome = m
            .ensure_image(&img("nginx:1.25"), ImagePullPolicy::IfNotPresent, "p", t(0))
            .unwrap();
        assert_eq!(outcome, EnsureOutcome::Pulling);
        let s = m.pull_status("nginx:1.25").unwrap();
        assert_eq!(s.state, PullState::InFlight);
    }

    #[test]
    fn ensure_always_pulls_even_if_present() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.record_present("nginx:latest");
        let outcome = m
            .ensure_image(&img("nginx:latest"), ImagePullPolicy::Always, "p", t(0))
            .unwrap();
        assert_eq!(outcome, EnsureOutcome::Pulling);
    }

    #[test]
    fn parallel_pulls_throttled_to_max() {
        let mut m = ImageManager::new(ImageManagerConfig {
            max_parallel_pulls: 1,
            pull_failure_backoff: Duration::seconds(30),
        });
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.ensure_image(&img("b:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        assert_eq!(m.in_flight(), 1);
        assert_eq!(m.queue_depth(), 1);
    }

    #[test]
    fn next_pull_admitted_after_completion() {
        let mut m = ImageManager::new(ImageManagerConfig {
            max_parallel_pulls: 1,
            pull_failure_backoff: Duration::seconds(30),
        });
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.ensure_image(&img("b:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_success("a:1", 1024, t(1)).unwrap();
        assert_eq!(m.in_flight(), 1);
        let s = m.pull_status("b:1").unwrap();
        assert_eq!(s.state, PullState::InFlight);
    }

    #[test]
    fn duplicate_ensure_attaches_as_waiter() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "pod-1", t(0)).unwrap();
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "pod-2", t(0)).unwrap();
        let s = m.pull_status("a:1").unwrap();
        assert_eq!(s.waiters.len(), 2);
    }

    #[test]
    fn record_success_marks_present_and_releases_slot() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_success("a:1", 2048, t(1)).unwrap();
        assert!(m.is_present("a:1"));
        assert_eq!(m.in_flight(), 0);
    }

    #[test]
    fn record_failure_does_not_mark_present() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_failure("a:1", "registry timeout", t(1)).unwrap();
        assert!(!m.is_present("a:1"));
    }

    #[test]
    fn failure_backoff_blocks_immediate_retry() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_failure("a:1", "fail", t(1)).unwrap();
        let err = m
            .ensure_image(&img("a:1"), ImagePullPolicy::Always, "p2", t(2))
            .unwrap_err();
        assert!(matches!(err, ImageError::PullBackoff(_)));
    }

    #[test]
    fn failure_backoff_clears_after_period() {
        let mut m = ImageManager::new(ImageManagerConfig {
            max_parallel_pulls: 1,
            pull_failure_backoff: Duration::seconds(5),
        });
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_failure("a:1", "fail", t(1)).unwrap();
        let outcome = m
            .ensure_image(&img("a:1"), ImagePullPolicy::Always, "p2", t(7))
            .unwrap();
        assert_eq!(outcome, EnsureOutcome::Pulling);
    }

    #[test]
    fn attempts_counter_grows_on_retry() {
        let mut m = ImageManager::new(ImageManagerConfig {
            max_parallel_pulls: 1,
            pull_failure_backoff: Duration::seconds(0),
        });
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_failure("a:1", "fail", t(1)).unwrap();
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(2)).unwrap();
        assert_eq!(m.pull_status("a:1").unwrap().attempts, 2);
    }

    #[test]
    fn pull_progress_updates_bytes() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.update_pull_progress("a:1", 100, 1000).unwrap();
        let s = m.pull_status("a:1").unwrap();
        assert_eq!(s.bytes_pulled, 100);
        assert_eq!(s.bytes_total, 1000);
    }

    #[test]
    fn update_progress_unknown_image_errors() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        assert!(m.update_pull_progress("ghost", 0, 0).is_err());
    }

    #[test]
    fn forget_clears_present_and_pulls() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.record_pull_success("a:1", 1, t(1)).unwrap();
        m.forget("a:1");
        assert!(!m.is_present("a:1"));
        assert!(m.pull_status("a:1").is_none());
    }

    #[test]
    fn drop_pod_clears_pod_refs_and_waiter_set() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "pod-1", t(0)).unwrap();
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "pod-2", t(0)).unwrap();
        m.drop_pod("pod-1");
        assert!(m.refs_by_pod("pod-1").is_empty());
        let s = m.pull_status("a:1").unwrap();
        assert_eq!(s.waiters.len(), 1);
    }

    #[test]
    fn refs_by_pod_returns_image_keys() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "pod-1", t(0)).unwrap();
        m.ensure_image(&img("b:2"), ImagePullPolicy::Always, "pod-1", t(0)).unwrap();
        let refs = m.refs_by_pod("pod-1");
        assert!(refs.contains(&"a:1".to_string()));
        assert!(refs.contains(&"b:2".to_string()));
    }

    #[test]
    fn resolve_credential_exact_server_match() {
        let secrets = vec![ImagePullSecret {
            name: "s".into(),
            server: "registry.io".into(),
            username: "u".into(),
            password: "p".into(),
        }];
        let s = resolve_credential("registry.io/foo/bar", &secrets).unwrap();
        assert_eq!(s.server, "registry.io");
    }

    #[test]
    fn resolve_credential_suffix_match() {
        let secrets = vec![ImagePullSecret {
            name: "s".into(),
            server: "io".into(),
            username: "u".into(),
            password: "p".into(),
        }];
        let s = resolve_credential("registry.io/foo", &secrets);
        assert!(s.is_some());
    }

    #[test]
    fn resolve_credential_no_match_returns_none() {
        let secrets = vec![ImagePullSecret {
            name: "s".into(),
            server: "other.com".into(),
            username: "u".into(),
            password: "p".into(),
        }];
        assert!(resolve_credential("registry.io/foo", &secrets).is_none());
    }

    #[test]
    fn resolve_credential_empty_server_skipped() {
        let secrets = vec![ImagePullSecret {
            name: "s".into(),
            server: "".into(),
            username: "u".into(),
            password: "p".into(),
        }];
        assert!(resolve_credential("registry.io/foo", &secrets).is_none());
    }

    #[test]
    fn record_success_unknown_image_errors() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        assert!(m.record_pull_success("ghost", 0, t(0)).is_err());
    }

    #[test]
    fn record_failure_unknown_image_errors() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        assert!(m.record_pull_failure("ghost", "fail", t(0)).is_err());
    }

    #[test]
    fn parallel_three_concurrent_pulls() {
        let mut m = ImageManager::new(ImageManagerConfig {
            max_parallel_pulls: 3,
            pull_failure_backoff: Duration::seconds(30),
        });
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.ensure_image(&img("b:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.ensure_image(&img("c:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        m.ensure_image(&img("d:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        assert_eq!(m.in_flight(), 3);
        assert_eq!(m.queue_depth(), 1);
    }

    #[test]
    fn always_with_present_image_still_attaches_waiter_during_pull() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.record_present("nginx:latest");
        m.ensure_image(&img("nginx:latest"), ImagePullPolicy::Always, "pod-1", t(0)).unwrap();
        m.ensure_image(&img("nginx:latest"), ImagePullPolicy::Always, "pod-2", t(0)).unwrap();
        assert_eq!(m.pull_status("nginx:latest").unwrap().waiters.len(), 2);
    }

    #[test]
    fn pull_state_transitions_through_lifecycle() {
        let mut m = ImageManager::new(ImageManagerConfig::default());
        m.ensure_image(&img("a:1"), ImagePullPolicy::Always, "p", t(0)).unwrap();
        assert_eq!(m.pull_status("a:1").unwrap().state, PullState::InFlight);
        m.record_pull_success("a:1", 100, t(1)).unwrap();
        assert_eq!(m.pull_status("a:1").unwrap().state, PullState::Succeeded);
    }

    #[test]
    fn key_for_repo_with_path() {
        assert_eq!(img("ghcr.io/foo/bar:v1").key(), "ghcr.io/foo/bar:v1");
    }

    #[test]
    fn parse_image_with_path_no_tag() {
        let r = img("ghcr.io/foo/bar");
        assert_eq!(r.repository, "ghcr.io/foo/bar");
        assert!(r.tag.is_none());
    }

    #[test]
    fn config_default_values() {
        let c = ImageManagerConfig::default();
        assert_eq!(c.max_parallel_pulls, 1);
        assert_eq!(c.pull_failure_backoff, Duration::seconds(30));
    }
}
