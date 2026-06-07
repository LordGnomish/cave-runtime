// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! The poll pass — fetch the latest upstream tag for every distinct repo
//! once, then fan the result back out across each cave module that tracks
//! it and classify drift.
//!
//! `poll_all` is generic over [`ReleaseFetcher`] so the whole pass runs
//! offline and deterministically under test with a fake fetcher; the
//! binary wires in the live [`GithubFetcher`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::config::TrackerConfig;
use crate::registry::{drift, DriftStatus, ReleaseFetcher, Upstream};

/// One upstream after polling: the registry entry, the fetched latest
/// tag (if any), and the drift verdict against its pinned baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResult {
    pub upstream: Upstream,
    pub latest: Option<String>,
    pub status: DriftStatus,
}

/// The full result of one daily poll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollSummary {
    pub polled_at_utc: String,
    pub results: Vec<PollResult>,
    /// repos whose latest tag could not be fetched (offline/rate-limited).
    pub unresolved: Vec<String>,
}

impl PollSummary {
    pub fn total(&self) -> usize {
        self.results.len()
    }

    pub fn count(&self, status: DriftStatus) -> usize {
        self.results.iter().filter(|r| r.status == status).count()
    }

    /// Registry-only summary with no fetched tags — the graceful-offline
    /// fallback that still emits every row (all `Unknown`).
    pub fn from_registry_only(cfg: &TrackerConfig) -> Self {
        let results = cfg
            .upstreams
            .iter()
            .map(|u| PollResult {
                status: drift(u.pinned.as_deref(), None),
                upstream: u.clone(),
                latest: None,
            })
            .collect();
        Self {
            polled_at_utc: chrono::Utc::now().to_rfc3339(),
            results,
            unresolved: cfg.distinct_repos(),
        }
    }
}

/// Poll every distinct repo exactly once through `fetcher`, then build a
/// [`PollResult`] for each upstream (fanning a shared repo's tag out to
/// all cave modules tracking it).
pub async fn poll_all<F: ReleaseFetcher>(cfg: &TrackerConfig, fetcher: &F) -> PollSummary {
    let mut tags: BTreeMap<String, Option<String>> = BTreeMap::new();
    for repo in cfg.distinct_repos() {
        let latest = fetcher.latest_release(&repo).await;
        tags.insert(repo, latest);
    }

    let mut results = Vec::with_capacity(cfg.upstreams.len());
    for u in &cfg.upstreams {
        let latest = tags.get(&u.repo).cloned().flatten();
        let status = drift(u.pinned.as_deref(), latest.as_deref());
        results.push(PollResult {
            upstream: u.clone(),
            latest,
            status,
        });
    }

    let unresolved: Vec<String> = tags
        .into_iter()
        .filter_map(|(repo, tag)| tag.is_none().then_some(repo))
        .collect();

    PollSummary {
        polled_at_utc: chrono::Utc::now().to_rfc3339(),
        results,
        unresolved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Deterministic offline fetcher driven by a fixed map.
    struct MapFetcher(HashMap<String, String>);

    #[async_trait::async_trait]
    impl ReleaseFetcher for MapFetcher {
        async fn latest_release(&self, repo: &str) -> Option<String> {
            self.0.get(repo).cloned()
        }
    }

    fn small_cfg() -> TrackerConfig {
        let mut c = TrackerConfig::default_config();
        c.upstreams.truncate(3);
        // Start from a clean slate (the default registry now ships
        // curated pins) so this fixture isolates the pin→drift logic:
        // only upstream[0] is pinned, the rest stay unpinned/Unknown.
        for u in &mut c.upstreams {
            u.pinned = None;
        }
        c.upstreams[0].pinned = Some("v1.0.0".to_string());
        c
    }

    #[tokio::test]
    async fn poll_resolves_known_repos_and_classifies_drift() {
        let cfg = small_cfg();
        let repo0 = cfg.upstreams[0].repo.clone();
        let mut map = HashMap::new();
        // Same version → in-sync (with v-normalisation).
        map.insert(repo0.clone(), "1.0.0".to_string());
        let f = MapFetcher(map);
        let s = poll_all(&cfg, &f).await;
        assert_eq!(s.total(), 3);
        assert_eq!(s.results[0].status, DriftStatus::InSync);
        // Unpinned entries are Unknown even when a tag is fetched.
        assert_eq!(s.results[1].status, DriftStatus::Unknown);
    }

    #[tokio::test]
    async fn poll_marks_behind_when_pin_differs() {
        let cfg = small_cfg();
        let repo0 = cfg.upstreams[0].repo.clone();
        let mut map = HashMap::new();
        map.insert(repo0, "v2.0.0".to_string());
        let f = MapFetcher(map);
        let s = poll_all(&cfg, &f).await;
        assert_eq!(s.results[0].status, DriftStatus::Behind);
        assert_eq!(s.results[0].latest.as_deref(), Some("v2.0.0"));
    }

    #[tokio::test]
    async fn unresolved_repos_are_recorded() {
        let cfg = small_cfg();
        let f = MapFetcher(HashMap::new()); // nothing resolves
        let s = poll_all(&cfg, &f).await;
        assert_eq!(s.unresolved.len(), cfg.distinct_repos().len());
        assert!(s.results.iter().all(|r| r.latest.is_none()));
    }

    #[tokio::test]
    async fn shared_repo_fetched_once_fans_to_all_modules() {
        // Two upstreams pointing at the same repo must both receive the
        // tag from a single fetch.
        let mut cfg = TrackerConfig::default_config();
        cfg.upstreams.retain(|u| u.repo == "kubernetes/kubernetes");
        assert!(cfg.upstreams.len() >= 2, "fixture needs shared repo");
        let mut map = HashMap::new();
        map.insert("kubernetes/kubernetes".to_string(), "v1.33.0".to_string());
        let f = MapFetcher(map);
        let s = poll_all(&cfg, &f).await;
        assert!(s.results.iter().all(|r| r.latest.as_deref() == Some("v1.33.0")));
        assert!(s.unresolved.is_empty());
    }

    #[test]
    fn registry_only_has_every_row_unknown() {
        let cfg = TrackerConfig::default_config();
        let s = PollSummary::from_registry_only(&cfg);
        assert_eq!(s.total(), cfg.upstreams.len());
        assert_eq!(s.count(DriftStatus::Unknown), cfg.upstreams.len());
    }
}
