// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Lines-of-code measurement — a coarse *port-depth* signal layered on
//! top of the version-drift signal.
//!
//! For a handful of headline upstreams we shallow-clone the repo at its
//! latest tip, run `tokei` over it, and compare the result against the
//! `tokei` count of the cave-* crate that ports it. The ratio
//! `cave_code / upstream_code` is a rough "how much of the surface have
//! we actually re-implemented" number — never a parity score (the cave
//! ports are deliberately a focused Rust re-implementation, not a 1:1
//! line translation), but a useful day-over-day trend.
//!
//! The whole pass is generic over [`LocSource`] so it runs offline and
//! deterministically under test; the binary wires in [`TokeiLoc`], which
//! shells out to `git` and `tokei` and cleans up its clone immediately.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Headline upstreams we measure by default — the big, well-known ports
/// where a LOC trend is most informative. Operators can widen this with
/// `cave-runtime-tracker measure --repo org/name`.
pub const DEFAULT_MEASURE_REPOS: &[&str] = &[
    "kubernetes/kubernetes",
    "clastix/kamaji",
    "cilium/cilium",
    "openbao/openbao",
    "kedacore/keda",
    "kubernetes-sigs/karpenter",
    "twentyhq/twenty",
    "FerretDB/FerretDB",
    "apache/kafka",
    "apache/pulsar",
];

/// A `tokei` line tally for one tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LocStats {
    pub code: usize,
    pub comments: usize,
    pub blanks: usize,
}

impl LocStats {
    pub fn total_lines(&self) -> usize {
        self.code + self.comments + self.blanks
    }
}

/// Parse the `--output json` document `tokei` writes. We only keep the
/// roll-up under the synthetic `"Total"` language key (`tokei` always
/// emits it), which sums code/comments/blanks across every language.
pub fn parse_tokei_json(json: &str) -> Option<LocStats> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let total = v.get("Total")?;
    let field = |k: &str| total.get(k).and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    Some(LocStats {
        code: field("code"),
        comments: field("comments"),
        blanks: field("blanks"),
    })
}

/// Port-depth ratio `cave_code / upstream_code`. `None` when either side
/// is missing or the upstream has zero code (no division by zero, no
/// fabricated 0% for an unmeasured upstream).
pub fn port_ratio(cave: Option<LocStats>, upstream: Option<LocStats>) -> Option<f64> {
    let (c, u) = (cave?, upstream?);
    if u.code == 0 {
        return None;
    }
    Some(c.code as f64 / u.code as f64)
}

/// One measured upstream↔cave pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    pub upstream_repo: String,
    pub cave_module: String,
    pub upstream: Option<LocStats>,
    pub cave: Option<LocStats>,
    /// `cave.code / upstream.code`, when both sides measured.
    pub ratio: Option<f64>,
}

/// Source of LOC tallies. The live implementation clones + runs `tokei`;
/// the test implementation serves a fixed map.
pub trait LocSource {
    /// LOC for an upstream `org/repo` (live: shallow-clone then `tokei`).
    fn upstream_loc(&self, repo: &str) -> Option<LocStats>;
    /// LOC for a local `cave-<x>` crate, by module name.
    fn cave_loc(&self, module: &str) -> Option<LocStats>;
}

/// Measure the registry rows whose `repo` is in `repos`, fetching each
/// distinct upstream's LOC exactly once and fanning it out to every cave
/// module that tracks it (mirrors the version poll's distinct-repo fan).
pub fn measure_subset<S: LocSource>(
    upstreams: &[crate::registry::Upstream],
    source: &S,
    repos: &[&str],
) -> Vec<Measurement> {
    use std::collections::BTreeMap;
    let mut upstream_cache: BTreeMap<String, Option<LocStats>> = BTreeMap::new();
    let mut out = Vec::new();
    for u in upstreams {
        if !repos.contains(&u.repo.as_str()) {
            continue;
        }
        let up = upstream_cache
            .entry(u.repo.clone())
            .or_insert_with(|| source.upstream_loc(&u.repo))
            .to_owned();
        let cave = source.cave_loc(&u.cave_module);
        let ratio = port_ratio(cave, up);
        out.push(Measurement {
            upstream_repo: u.repo.clone(),
            cave_module: u.cave_module.clone(),
            upstream: up,
            cave,
            ratio,
        });
    }
    out
}

/// Live LOC source: `git clone --depth 1` into a temp dir + `tokei`, and
/// `tokei` directly over `crates/**/<module>` for the cave side.
pub struct TokeiLoc {
    /// cave-runtime workspace root (holds `crates/`).
    pub workspace_root: PathBuf,
    /// Where shallow clones are made (each removed right after measuring).
    pub clone_root: PathBuf,
}

impl TokeiLoc {
    pub fn new(workspace_root: impl Into<PathBuf>, clone_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            clone_root: clone_root.into(),
        }
    }

    /// Run `tokei --output json <path>` and parse the roll-up. Returns
    /// `None` if `tokei` is absent, fails, or the path is empty.
    fn tokei(path: &Path) -> Option<LocStats> {
        let out = std::process::Command::new("tokei")
            .arg("--output")
            .arg("json")
            .arg(path)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_tokei_json(&String::from_utf8_lossy(&out.stdout))
    }

    /// Locate a cave crate dir by module name across the nested layout
    /// (`crates/<group>/<module>`) and the flat layout (`crates/<module>`).
    fn crate_dir(&self, module: &str) -> Option<PathBuf> {
        let flat = self.workspace_root.join("crates").join(module);
        if flat.join("Cargo.toml").is_file() {
            return Some(flat);
        }
        let crates = self.workspace_root.join("crates");
        let groups = std::fs::read_dir(&crates).ok()?;
        for g in groups.flatten() {
            let cand = g.path().join(module);
            if cand.join("Cargo.toml").is_file() {
                return Some(cand);
            }
        }
        None
    }
}

impl LocSource for TokeiLoc {
    fn upstream_loc(&self, repo: &str) -> Option<LocStats> {
        // A filesystem-safe, collision-resistant dir name for the clone.
        let slug = repo.replace('/', "__");
        let dest = self.clone_root.join(format!("cave-rt-tracker-{slug}"));
        let _ = std::fs::remove_dir_all(&dest); // clear any stale clone
        let url = format!("https://github.com/{repo}.git");
        let status = std::process::Command::new("git")
            .args(["clone", "--depth", "1", "--quiet", &url])
            .arg(&dest)
            .status()
            .ok()?;
        let stats = if status.success() {
            Self::tokei(&dest)
        } else {
            None
        };
        // Always reclaim disk — a shallow k8s clone is ~150 MB.
        let _ = std::fs::remove_dir_all(&dest);
        stats
    }

    fn cave_loc(&self, module: &str) -> Option<LocStats> {
        let dir = self.crate_dir(module)?;
        Self::tokei(&dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::default_registry;
    use std::collections::HashMap;

    const SAMPLE: &str = r#"{
        "Rust": {"blanks": 10, "code": 100, "comments": 20, "reports": [], "children": {}},
        "Total": {"blanks": 30, "code": 250, "comments": 70, "reports": [], "children": {}}
    }"#;

    #[test]
    fn parse_tokei_takes_the_total_rollup() {
        let s = parse_tokei_json(SAMPLE).unwrap();
        assert_eq!(s.code, 250);
        assert_eq!(s.comments, 70);
        assert_eq!(s.blanks, 30);
        assert_eq!(s.total_lines(), 350);
    }

    #[test]
    fn parse_tokei_rejects_garbage() {
        assert!(parse_tokei_json("not json").is_none());
        assert!(parse_tokei_json("{}").is_none()); // no Total key
    }

    #[test]
    fn port_ratio_is_cave_over_upstream() {
        let cave = LocStats { code: 800, ..Default::default() };
        let up = LocStats { code: 4000, ..Default::default() };
        assert_eq!(port_ratio(Some(cave), Some(up)), Some(0.2));
    }

    #[test]
    fn port_ratio_guards_missing_and_zero() {
        let some = LocStats { code: 10, ..Default::default() };
        assert_eq!(port_ratio(None, Some(some)), None);
        assert_eq!(port_ratio(Some(some), None), None);
        assert_eq!(
            port_ratio(Some(some), Some(LocStats::default())),
            None,
            "zero upstream must not divide"
        );
    }

    /// Offline source: fixed upstream + cave tallies, counts the clones.
    struct FakeSource {
        upstream: HashMap<String, LocStats>,
        cave: HashMap<String, LocStats>,
        hits: std::cell::RefCell<HashMap<String, usize>>,
    }

    impl LocSource for FakeSource {
        fn upstream_loc(&self, repo: &str) -> Option<LocStats> {
            *self.hits.borrow_mut().entry(repo.to_string()).or_insert(0) += 1;
            self.upstream.get(repo).copied()
        }
        fn cave_loc(&self, module: &str) -> Option<LocStats> {
            self.cave.get(module).copied()
        }
    }

    #[test]
    fn measure_subset_fans_distinct_repo_and_computes_ratio() {
        let reg = default_registry();
        let mut upstream = HashMap::new();
        upstream.insert(
            "kubernetes/kubernetes".to_string(),
            LocStats { code: 5_000_000, ..Default::default() },
        );
        let mut cave = HashMap::new();
        // Two of the five k8s rows have a cave measurement.
        cave.insert("cave-apiserver".to_string(), LocStats { code: 50_000, ..Default::default() });
        cave.insert("cave-scheduler".to_string(), LocStats { code: 25_000, ..Default::default() });
        let src = FakeSource {
            upstream,
            cave,
            hits: Default::default(),
        };

        let ms = measure_subset(&reg, &src, &["kubernetes/kubernetes"]);
        // Five registry rows track kubernetes/kubernetes.
        assert!(ms.len() >= 5, "got {}", ms.len());
        // The upstream was cloned exactly once despite the fan-out.
        assert_eq!(*src.hits.borrow().get("kubernetes/kubernetes").unwrap(), 1);
        // apiserver: 50_000 / 5_000_000 = 0.01.
        let apiserver = ms.iter().find(|m| m.cave_module == "cave-apiserver").unwrap();
        assert_eq!(apiserver.ratio, Some(0.01));
        // A row with no cave measurement reports ratio None, not 0.
        let unmeasured = ms.iter().find(|m| m.cave.is_none());
        assert!(unmeasured.is_some_and(|m| m.ratio.is_none()));
    }

    #[test]
    fn measure_subset_skips_repos_not_requested() {
        let reg = default_registry();
        let src = FakeSource {
            upstream: HashMap::new(),
            cave: HashMap::new(),
            hits: Default::default(),
        };
        let ms = measure_subset(&reg, &src, &["cilium/cilium"]);
        assert!(ms.iter().all(|m| m.upstream_repo == "cilium/cilium"));
        assert!(src.hits.borrow().keys().all(|r| r == "cilium/cilium"));
    }

    #[test]
    fn default_measure_repos_are_all_in_the_registry() {
        let repos: std::collections::BTreeSet<String> =
            default_registry().into_iter().map(|u| u.repo).collect();
        for r in DEFAULT_MEASURE_REPOS {
            assert!(repos.contains(*r), "measure repo {r} missing from registry");
        }
    }
}
