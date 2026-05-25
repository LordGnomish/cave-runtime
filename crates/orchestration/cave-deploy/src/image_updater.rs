// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Argo CD Image Updater parity — registry tag watcher + Application image
//! mutator. Mirrors `argoproj-labs/argocd-image-updater` (Apache-2.0).
//!
//! Five primitives:
//! * [`ImageRef`]         — registry + repository + current tag tuple.
//! * [`TagSelector`]      — semver / digest / latest / regex / newest-build
//!                          strategies (Apache-2.0 upstream parity).
//! * [`UpdateStrategy`]   — git-write vs. annotation-write delivery channel.
//! * [`RegistryEndpoint`] — credential-bearing registry handle (keychain only).
//! * [`ImageUpdater`]     — orchestrator that scans Applications, evaluates
//!                          tag candidates, and emits [`ImageUpdate`] deltas.
//!
//! Upstream cross-reference (argoproj-labs/argocd-image-updater@v0.16.0):
//!   * `pkg/image/version_constraint.go` — semver tag matcher
//!   * `pkg/image/version_metadata.go`   — newest-build sort
//!   * `pkg/argocd/argocd.go`            — Application annotation write path
//!   * `pkg/git/git.go`                  — git-write commit path
//!   * `pkg/registry/client.go`          — Manifest v2 + OCI digest fetch

use crate::error::DeployError;
use crate::models::Application;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Reference to a container image in a registry — `registry/repo:tag`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    pub alias: String,
    pub registry: String,
    pub repository: String,
    pub tag: String,
    /// Optional digest pin (`sha256:…`). When set, `tag` is informational.
    #[serde(default)]
    pub digest: Option<String>,
}

impl ImageRef {
    pub fn parse(alias: &str, raw: &str) -> Result<Self, DeployError> {
        let (registry, rest) = match raw.split_once('/') {
            Some((reg, r)) if reg.contains('.') || reg.contains(':') => (reg.to_string(), r),
            _ => ("docker.io".to_string(), raw),
        };
        let (repo_tag, digest) = match rest.split_once('@') {
            Some((rt, d)) => (rt, Some(d.to_string())),
            None => (rest, None),
        };
        let (repository, tag) = match repo_tag.rsplit_once(':') {
            Some((r, t)) if !r.is_empty() && !t.contains('/') => (r.to_string(), t.to_string()),
            _ => (repo_tag.to_string(), "latest".to_string()),
        };
        if repository.is_empty() {
            return Err(DeployError::ManifestParse(format!(
                "image_updater: empty repository in {raw}"
            )));
        }
        Ok(Self {
            alias: alias.to_string(),
            registry,
            repository,
            tag,
            digest,
        })
    }

    pub fn full(&self) -> String {
        match &self.digest {
            Some(d) => format!("{}/{}@{}", self.registry, self.repository, d),
            None => format!("{}/{}:{}", self.registry, self.repository, self.tag),
        }
    }
}

/// How to choose the next tag from a candidate set.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "spec")]
pub enum TagSelector {
    /// Highest SemVer release that satisfies `constraint` (e.g. `~1.4`).
    Semver { constraint: String },
    /// Latest by registry push timestamp (newest-build in upstream).
    NewestBuild,
    /// Always re-pin to a fresh digest of `tag` (idempotent latest follower).
    Digest { tag: String },
    /// Sticky tag — never advance.
    Latest,
    /// Tags matching `pattern`, sorted alphabetically descending (regex
    /// parity with `argocd-image-updater.argoproj.io/<alias>.update-strategy=regex`).
    Regex { pattern: String },
}

impl TagSelector {
    /// Pick the next tag from `candidates`. Returns `None` if no candidate beats
    /// `current`. Candidate ordering follows registry response order; this
    /// function imposes its own sort.
    pub fn select<'a>(
        &self,
        current: &str,
        candidates: &'a [TagCandidate],
    ) -> Option<&'a TagCandidate> {
        match self {
            TagSelector::Semver { constraint } => {
                let mut best: Option<&TagCandidate> = None;
                for c in candidates {
                    if !semver_satisfies(&c.tag, constraint) {
                        continue;
                    }
                    if !semver_gt(&c.tag, current) {
                        continue;
                    }
                    if best.is_none_or(|b| semver_gt(&c.tag, &b.tag)) {
                        best = Some(c);
                    }
                }
                best
            }
            TagSelector::NewestBuild => candidates
                .iter()
                .filter(|c| c.pushed_at.is_some() && c.tag != current)
                .max_by_key(|c| c.pushed_at),
            TagSelector::Digest { tag } => candidates
                .iter()
                .find(|c| &c.tag == tag && c.digest.as_deref() != Some(current)),
            TagSelector::Latest => None,
            TagSelector::Regex { pattern } => {
                let mut hits: Vec<&TagCandidate> = candidates
                    .iter()
                    .filter(|c| regex_lite_match(pattern, &c.tag) && c.tag != current)
                    .collect();
                hits.sort_by(|a, b| b.tag.cmp(&a.tag));
                hits.first().copied()
            }
        }
    }
}

/// A tag observed from a registry — `tag` plus optional digest and push time.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TagCandidate {
    pub tag: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub pushed_at: Option<i64>, // unix seconds; None ⇒ unknown
}

/// Where the new tag is written to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum UpdateStrategy {
    /// Patch `Application.metadata.annotations.image-updater/<alias>.allow-tags`
    /// via the cave-deploy API — no git write.
    AnnotationWrite,
    /// Commit a Kustomize/Helm patch back to the source repo. `path` is the
    /// values.yaml or kustomization.yaml to mutate. `branch` is the write
    /// target.
    GitWrite { path: String, branch: String },
}

/// Registry handle. Credentials reference keychain entries — they are never
/// inlined in the manifest (consistent with cave-vault charter).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegistryEndpoint {
    pub name: String,
    pub url: String,
    /// Optional cave-vault keychain id for basic-auth or token credentials.
    #[serde(default)]
    pub credential_ref: Option<String>,
    /// If true, scan order is randomised to spread cache cost.
    #[serde(default)]
    pub jittered: bool,
}

/// Single update emission — what was selected, why, and how it should land.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImageUpdate {
    pub application: String,
    pub alias: String,
    pub from_tag: String,
    pub to_tag: String,
    pub from_digest: Option<String>,
    pub to_digest: Option<String>,
    pub strategy: UpdateStrategy,
    pub selector: TagSelector,
}

/// In-memory orchestrator. Mirrors the upstream `pkg/argocd/argocd.go::
/// updateApplication` loop without the goroutines.
#[derive(Default)]
pub struct ImageUpdater {
    registries: BTreeMap<String, RegistryEndpoint>,
    selectors: BTreeMap<String, (TagSelector, UpdateStrategy)>,
}

impl ImageUpdater {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_registry(&mut self, reg: RegistryEndpoint) {
        self.registries.insert(reg.name.clone(), reg);
    }

    /// Register a per-alias `(selector, strategy)` policy. Aliases are matched
    /// against `Application.metadata.annotations.image-updater/<alias>.image`.
    pub fn set_policy(&mut self, alias: impl Into<String>, sel: TagSelector, str_: UpdateStrategy) {
        self.selectors.insert(alias.into(), (sel, str_));
    }

    pub fn registries(&self) -> impl Iterator<Item = &RegistryEndpoint> {
        self.registries.values()
    }

    pub fn policy(&self, alias: &str) -> Option<&(TagSelector, UpdateStrategy)> {
        self.selectors.get(alias)
    }

    /// Evaluate one Application against `candidates_by_alias`. Returns the
    /// concrete updates that would be emitted (no I/O — caller writes them
    /// through `apply_*`).
    pub fn plan(
        &self,
        app: &Application,
        observations: &BTreeMap<String, Vec<TagCandidate>>,
    ) -> Vec<ImageUpdate> {
        let mut out = Vec::new();
        for (alias, (sel, str_)) in &self.selectors {
            let Some(images) = images_for_alias(app, alias) else {
                continue;
            };
            for img in images {
                let Some(cands) = observations.get(alias) else {
                    continue;
                };
                if let Some(c) = sel.select(&img.tag, cands) {
                    if c.tag == img.tag && c.digest == img.digest {
                        continue;
                    }
                    out.push(ImageUpdate {
                        application: app.name.clone(),
                        alias: alias.clone(),
                        from_tag: img.tag.clone(),
                        to_tag: c.tag.clone(),
                        from_digest: img.digest.clone(),
                        to_digest: c.digest.clone(),
                        strategy: str_.clone(),
                        selector: sel.clone(),
                    });
                }
            }
        }
        out
    }
}

fn images_for_alias(app: &Application, alias: &str) -> Option<Vec<ImageRef>> {
    // upstream annotation: `image-updater.argoproj.io/<alias>.image-name=…`
    // We mirror the convention but accept the raw repo string straight off the
    // annotation value. Multiple images per alias are supported by `,`-split.
    let key = format!("image-updater.argoproj.io/{alias}.image-name");
    let raw = app.annotations.get(&key)?;
    let mut out = Vec::new();
    for chunk in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(img) = ImageRef::parse(alias, chunk) {
            out.push(img);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

// ─────────────────────────── SemVer helpers (range-only subset) ─────────────
// Only the comparators argocd-image-updater actually exercises:
//   `^X.Y[.Z]`, `~X.Y[.Z]`, `>=X.Y.Z`, `=X.Y.Z`, bare `X.Y.Z`.

fn semver_parse(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.strip_prefix('v').unwrap_or(v);
    // strip pre-release / build for ordering — upstream behaves the same.
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let maj = parts.next()?.parse().ok()?;
    let min = parts.next().unwrap_or("0").parse().ok()?;
    let pat = parts.next().unwrap_or("0").parse().ok()?;
    Some((maj, min, pat))
}

fn semver_gt(a: &str, b: &str) -> bool {
    match (semver_parse(a), semver_parse(b)) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

fn semver_satisfies(v: &str, constraint: &str) -> bool {
    let Some((vmaj, vmin, vpat)) = semver_parse(v) else {
        return false;
    };
    let c = constraint.trim();
    if let Some(rest) = c.strip_prefix("^") {
        let Some((cmaj, cmin, cpat)) = semver_parse(rest) else {
            return false;
        };
        return vmaj == cmaj && (vmin, vpat) >= (cmin, cpat);
    }
    if let Some(rest) = c.strip_prefix("~") {
        let Some((cmaj, cmin, cpat)) = semver_parse(rest) else {
            return false;
        };
        return vmaj == cmaj && vmin == cmin && vpat >= cpat;
    }
    if let Some(rest) = c.strip_prefix(">=") {
        let Some(cv) = semver_parse(rest) else {
            return false;
        };
        return (vmaj, vmin, vpat) >= cv;
    }
    if let Some(rest) = c.strip_prefix("=") {
        let Some(cv) = semver_parse(rest) else {
            return false;
        };
        return (vmaj, vmin, vpat) == cv;
    }
    match semver_parse(c) {
        Some(cv) => (vmaj, vmin, vpat) == cv,
        None => false,
    }
}

// Tiny regex-lite: `*` (any-run) + `?` (one). No anchors — full match implied.
// Mirrors upstream behaviour for the `regex` strategy on shell-glob inputs.
fn regex_lite_match(pattern: &str, text: &str) -> bool {
    fn rec(p: &[u8], t: &[u8]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        match p[0] {
            b'*' => (0..=t.len()).any(|i| rec(&p[1..], &t[i..])),
            b'?' => !t.is_empty() && rec(&p[1..], &t[1..]),
            c => !t.is_empty() && t[0] == c && rec(&p[1..], &t[1..]),
        }
    }
    rec(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Application, ApplicationSource, ApplicationSpec, Destination, ResourceTracking,
    };
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn cand(tag: &str, digest: Option<&str>, pushed: Option<i64>) -> TagCandidate {
        TagCandidate {
            tag: tag.to_string(),
            digest: digest.map(str::to_string),
            pushed_at: pushed,
        }
    }

    fn fixture_app(name: &str, annotations: HashMap<String, String>) -> Application {
        Application {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: "argocd".into(),
            spec: ApplicationSpec {
                source: ApplicationSource {
                    repo_url: "https://github.com/example/repo.git".into(),
                    target_revision: Some("main".into()),
                    path: Some("manifests/".into()),
                    helm: None,
                    kustomize: None,
                    directory: None,
                },
                sources: vec![],
                destination: Destination {
                    server: "https://kubernetes.default.svc".into(),
                    name: None,
                    namespace: "default".into(),
                },
                project: "default".into(),
                sync_policy: None,
                ignored_differences: None,
                info: None,
                revision_history_limit: None,
            },
            status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: HashMap::new(),
            annotations,
            tracking: ResourceTracking::default(),
        }
    }

    fn app_with_annot(alias: &str, raw: &str) -> Application {
        let mut annot = HashMap::new();
        annot.insert(
            format!("image-updater.argoproj.io/{alias}.image-name"),
            raw.to_string(),
        );
        fixture_app("guestbook", annot)
    }

    #[test]
    fn imageref_parses_docker_hub_default() {
        let img = ImageRef::parse("api", "nginx:1.25").unwrap();
        assert_eq!(img.registry, "docker.io");
        assert_eq!(img.repository, "nginx");
        assert_eq!(img.tag, "1.25");
    }

    #[test]
    fn imageref_parses_private_registry() {
        let img = ImageRef::parse("api", "ghcr.io/cave/runtime:v0.1.0").unwrap();
        assert_eq!(img.registry, "ghcr.io");
        assert_eq!(img.repository, "cave/runtime");
        assert_eq!(img.tag, "v0.1.0");
    }

    #[test]
    fn imageref_parses_digest_pin() {
        let img =
            ImageRef::parse("api", "ghcr.io/cave/runtime@sha256:deadbeefdeadbeefdeadbeef").unwrap();
        assert_eq!(img.digest.as_deref(), Some("sha256:deadbeefdeadbeefdeadbeef"));
        assert_eq!(img.tag, "latest");
    }

    #[test]
    fn imageref_rejects_empty_repo() {
        assert!(ImageRef::parse("api", "").is_err());
    }

    #[test]
    fn semver_selector_picks_highest_satisfying() {
        let sel = TagSelector::Semver {
            constraint: "^1.4".into(),
        };
        let cands = [cand("1.3.9", None, None), cand("1.4.7", None, None), cand("1.5.0", None, None), cand("2.0.0", None, None)];
        let picked = sel.select("1.4.0", &cands).unwrap();
        assert_eq!(picked.tag, "1.5.0");
    }

    #[test]
    fn semver_selector_respects_tilde_constraint() {
        let sel = TagSelector::Semver {
            constraint: "~1.4.0".into(),
        };
        let cands = [cand("1.4.7", None, None), cand("1.5.0", None, None)];
        let picked = sel.select("1.4.0", &cands).unwrap();
        assert_eq!(picked.tag, "1.4.7");
    }

    #[test]
    fn semver_selector_yields_none_when_current_is_highest() {
        let sel = TagSelector::Semver {
            constraint: "^1.4".into(),
        };
        let cands = [cand("1.4.0", None, None)];
        assert!(sel.select("1.4.0", &cands).is_none());
    }

    #[test]
    fn newest_build_selector_uses_push_time() {
        let sel = TagSelector::NewestBuild;
        let cands = [
            cand("a", None, Some(100)),
            cand("b", None, Some(300)),
            cand("c", None, Some(200)),
        ];
        let picked = sel.select("a", &cands).unwrap();
        assert_eq!(picked.tag, "b");
    }

    #[test]
    fn digest_selector_refreshes_pin_only_on_change() {
        let sel = TagSelector::Digest {
            tag: "stable".into(),
        };
        let cands = [cand("stable", Some("sha256:new"), None)];
        let picked = sel.select("sha256:old", &cands).unwrap();
        assert_eq!(picked.digest.as_deref(), Some("sha256:new"));
    }

    #[test]
    fn latest_selector_never_advances() {
        let sel = TagSelector::Latest;
        let cands = [cand("latest", None, None), cand("v9.9.9", None, None)];
        assert!(sel.select("latest", &cands).is_none());
    }

    #[test]
    fn regex_selector_picks_highest_alpha_descending() {
        let sel = TagSelector::Regex {
            pattern: "release-*".into(),
        };
        let cands = [
            cand("release-2026-01-01", None, None),
            cand("release-2026-05-23", None, None),
            cand("dev", None, None),
        ];
        let picked = sel.select("release-2025-12-01", &cands).unwrap();
        assert_eq!(picked.tag, "release-2026-05-23");
    }

    #[test]
    fn plan_emits_update_for_annotated_image() {
        let app = app_with_annot("api", "ghcr.io/cave/runtime:1.4.0");
        let mut updater = ImageUpdater::new();
        updater.set_policy(
            "api",
            TagSelector::Semver {
                constraint: "^1.4".into(),
            },
            UpdateStrategy::AnnotationWrite,
        );
        let mut obs = BTreeMap::new();
        obs.insert(
            "api".to_string(),
            vec![cand("1.4.1", None, None), cand("1.5.0", None, None)],
        );
        let plan = updater.plan(&app, &obs);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].from_tag, "1.4.0");
        assert_eq!(plan[0].to_tag, "1.5.0");
        assert!(matches!(plan[0].strategy, UpdateStrategy::AnnotationWrite));
    }

    #[test]
    fn plan_skips_alias_without_observation() {
        let app = app_with_annot("api", "ghcr.io/cave/runtime:1.4.0");
        let mut updater = ImageUpdater::new();
        updater.set_policy(
            "api",
            TagSelector::Semver {
                constraint: "^1.4".into(),
            },
            UpdateStrategy::AnnotationWrite,
        );
        assert!(updater.plan(&app, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn plan_skips_alias_without_annotation() {
        let app = fixture_app("guestbook", HashMap::new());
        let mut updater = ImageUpdater::new();
        updater.set_policy(
            "api",
            TagSelector::Latest,
            UpdateStrategy::AnnotationWrite,
        );
        let obs: BTreeMap<String, Vec<TagCandidate>> = BTreeMap::new();
        assert!(updater.plan(&app, &obs).is_empty());
    }

    #[test]
    fn registry_round_trip_holds_credential_ref() {
        let mut up = ImageUpdater::new();
        up.add_registry(RegistryEndpoint {
            name: "ghcr".into(),
            url: "https://ghcr.io".into(),
            credential_ref: Some("kv/cave/ghcr-token".into()),
            jittered: true,
        });
        let regs: Vec<&RegistryEndpoint> = up.registries().collect();
        assert_eq!(regs.len(), 1);
        assert_eq!(regs[0].credential_ref.as_deref(), Some("kv/cave/ghcr-token"));
    }

    #[test]
    fn git_write_strategy_round_trips() {
        let s = UpdateStrategy::GitWrite {
            path: "values.yaml".into(),
            branch: "main".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: UpdateStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
