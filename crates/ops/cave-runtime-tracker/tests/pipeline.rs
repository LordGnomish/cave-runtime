// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! End-to-end pipeline test: registry → poll (offline fake fetcher) →
//! daily report → on-disk md/json. Proves the acceptance contract
//! ("`poll` works, daily markdown report is produced") without a network.

use std::collections::HashMap;

use cave_runtime_tracker::{
    poll_all, DailyReport, DriftStatus, ReleaseFetcher, TrackerConfig,
};

/// Deterministic offline fetcher: returns a fixed "latest" for every
/// repo it is asked about, so the whole registry resolves.
struct AllSameFetcher(String);

#[async_trait::async_trait]
impl ReleaseFetcher for AllSameFetcher {
    async fn latest_release(&self, _repo: &str) -> Option<String> {
        Some(self.0.clone())
    }
}

#[tokio::test]
async fn full_pipeline_emits_markdown_with_every_subsystem() {
    let cfg = TrackerConfig::default_config();
    let fetcher = AllSameFetcher("v9.9.9".to_string());

    let summary = poll_all(&cfg, &fetcher).await;
    // Every upstream resolved, so nothing is unresolved and every row
    // carries the fetched tag.
    assert!(summary.unresolved.is_empty(), "all repos should resolve");
    assert_eq!(summary.total(), cfg.upstreams.len());
    assert!(summary.results.iter().all(|r| r.latest.as_deref() == Some("v9.9.9")));
    // The registry now ships curated pins (cont2): pinned rows whose
    // baseline differs from the fetched v9.9.9 are Behind; the remaining
    // unpinned rows are honestly Unknown. Together they cover every row,
    // and the Behind count equals the number of pinned rows.
    let pinned = cfg.upstreams.iter().filter(|u| u.pinned.is_some()).count();
    assert!(pinned >= 50, "expected curated pins in the default registry");
    assert_eq!(summary.count(DriftStatus::Behind), pinned);
    assert_eq!(
        summary.count(DriftStatus::Behind) + summary.count(DriftStatus::Unknown),
        cfg.upstreams.len()
    );

    let report = DailyReport::assemble(summary);
    let md = report.to_markdown();
    // The markdown digest names a representative subsystem from each end
    // of the registry.
    for needle in ["Cilium", "kube-apiserver", "Twenty", "FerretDB", "KEDA"] {
        assert!(md.contains(needle), "report missing {needle}");
    }

    // Persist and re-read, mimicking the LaunchAgent's `report` run.
    let dir = tempfile::tempdir().unwrap();
    let (json_path, md_path) = report
        .write_to_dir(dir.path(), "2026-06-07", true)
        .unwrap();
    assert!(json_path.exists() && md_path.exists());
    let on_disk = std::fs::read_to_string(&md_path).unwrap();
    assert!(on_disk.contains("# cave-runtime-tracker"));

    let json = std::fs::read_to_string(&json_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["totals"]["tracked"], cfg.upstreams.len());
}

#[tokio::test]
async fn pinned_entry_reports_in_sync_or_behind() {
    let mut cfg = TrackerConfig::default_config();
    // Two upstreams with *distinct* repos (many cave modules share
    // kubernetes/kubernetes, which would collapse in the fetch map).
    cfg.upstreams
        .retain(|u| u.repo == "cilium/cilium" || u.repo == "coredns/coredns");
    assert_eq!(cfg.upstreams.len(), 2, "fixture expects two distinct repos");
    cfg.upstreams[0].pinned = Some("v1.0.0".to_string());
    cfg.upstreams[1].pinned = Some("v1.0.0".to_string());

    let mut map = HashMap::new();
    map.insert(cfg.upstreams[0].repo.clone(), "v1.0.0".to_string()); // in-sync
    map.insert(cfg.upstreams[1].repo.clone(), "v2.0.0".to_string()); // behind

    struct M(HashMap<String, String>);
    #[async_trait::async_trait]
    impl ReleaseFetcher for M {
        async fn latest_release(&self, repo: &str) -> Option<String> {
            self.0.get(repo).cloned()
        }
    }

    let summary = poll_all(&cfg, &M(map)).await;
    assert_eq!(summary.count(DriftStatus::InSync), 1);
    assert_eq!(summary.count(DriftStatus::Behind), 1);
}
