// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Standalone `runonce` mode — boot static-pod manifests
//! without an apiserver. Used by boot-time control-plane
//! components (e.g. starting the apiserver itself from a static
//! manifest before any apiserver exists).
//!
//! Mirrors `pkg/kubelet/runonce/` from upstream. The state
//! machine loads manifests from disk, validates them, and
//! reports a per-pod outcome — Running / Completed / Failed.
//! The actual container start lives in the CRI layer; this
//! module owns the *orchestration* logic.

use std::collections::BTreeMap;

/// One static pod manifest as the kubelet sees it on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticPodManifest {
    /// Path the manifest was loaded from (used in failure
    /// reporting).
    pub source: String,
    /// `metadata.name`.
    pub name: String,
    /// `spec.containers[*].image`.
    pub images: Vec<String>,
    /// `restartPolicy` — `Always` / `OnFailure` / `Never`.
    pub restart_policy: RestartPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

/// Outcome the runonce mode reports per pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOnceResult {
    /// Pod containers exited with status 0.
    Completed,
    /// Containers ran successfully and are still running (e.g.
    /// `Always` restart pods).
    Running,
    /// A container exited non-zero and policy was `Never` or
    /// retries exceeded.
    Failed { reason: String },
    /// Manifest failed validation — never started.
    Rejected { reason: String },
}

/// State machine that loads manifests, validates them, and
/// records outcomes.
#[derive(Debug, Default)]
pub struct RunOnce {
    manifests: BTreeMap<String, StaticPodManifest>,
    outcomes: BTreeMap<String, RunOnceResult>,
}

impl RunOnce {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate a manifest before loading. Mirrors upstream's
    /// shape: non-empty name, ≥1 container image, no whitespace
    /// in name.
    pub fn validate(m: &StaticPodManifest) -> Result<(), String> {
        if m.name.is_empty() {
            return Err("static pod name must not be empty".into());
        }
        if m.name.chars().any(|c| c.is_whitespace()) {
            return Err(format!(
                "static pod name must not contain whitespace: '{}'",
                m.name
            ));
        }
        if m.images.is_empty() {
            return Err("static pod must declare at least one container image".into());
        }
        Ok(())
    }

    /// Load a manifest. Rejected manifests are still recorded
    /// (with `Rejected` outcome) so the operator can see why.
    pub fn load(&mut self, manifest: StaticPodManifest) {
        match Self::validate(&manifest) {
            Ok(()) => {
                self.manifests.insert(manifest.name.clone(), manifest);
            }
            Err(reason) => {
                let name = manifest.name.clone();
                self.outcomes
                    .insert(name, RunOnceResult::Rejected { reason });
            }
        }
    }

    /// Record an outcome after the CRI layer has reported one.
    pub fn record(&mut self, name: &str, outcome: RunOnceResult) {
        self.outcomes.insert(name.to_string(), outcome);
    }

    pub fn outcomes(&self) -> &BTreeMap<String, RunOnceResult> {
        &self.outcomes
    }

    pub fn manifests(&self) -> Vec<&StaticPodManifest> {
        self.manifests.values().collect()
    }

    /// `true` if every loaded manifest has finished
    /// (Completed / Failed / Rejected). Used by the runonce
    /// driver to decide when to exit.
    pub fn all_settled(&self) -> bool {
        let pending = self.manifests.keys().filter(|n| {
            !matches!(
                self.outcomes.get(*n),
                Some(RunOnceResult::Completed | RunOnceResult::Failed { .. })
            )
        });
        pending.count() == 0
    }

    pub fn manifest_count(&self) -> usize {
        self.manifests.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str) -> StaticPodManifest {
        StaticPodManifest {
            source: format!("/etc/kubernetes/manifests/{name}.yaml"),
            name: name.into(),
            images: vec!["registry/img:tag".into()],
            restart_policy: RestartPolicy::Never,
        }
    }

    #[test]
    fn validate_accepts_normal_manifest() {
        let m = manifest("apiserver");
        assert!(RunOnce::validate(&m).is_ok());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut m = manifest("ok");
        m.name = String::new();
        assert!(RunOnce::validate(&m).is_err());
    }

    #[test]
    fn validate_rejects_whitespace_in_name() {
        let mut m = manifest("ok");
        m.name = "two words".into();
        assert!(RunOnce::validate(&m).is_err());
    }

    #[test]
    fn validate_rejects_no_images() {
        let mut m = manifest("ok");
        m.images.clear();
        assert!(RunOnce::validate(&m).is_err());
    }

    #[test]
    fn load_valid_manifest_stores_it() {
        let mut r = RunOnce::new();
        r.load(manifest("apiserver"));
        assert_eq!(r.manifest_count(), 1);
        assert!(r.outcomes().is_empty());
    }

    #[test]
    fn load_invalid_manifest_records_rejected_outcome() {
        let mut r = RunOnce::new();
        let mut m = manifest("ok");
        m.name = "bad name".into();
        r.load(m);
        assert_eq!(r.manifest_count(), 0);
        let outcome = r.outcomes().get("bad name").unwrap();
        assert!(matches!(outcome, RunOnceResult::Rejected { .. }));
    }

    #[test]
    fn record_overwrites_previous_outcome() {
        let mut r = RunOnce::new();
        r.load(manifest("a"));
        r.record("a", RunOnceResult::Running);
        assert_eq!(r.outcomes().get("a"), Some(&RunOnceResult::Running));
        r.record("a", RunOnceResult::Completed);
        assert_eq!(r.outcomes().get("a"), Some(&RunOnceResult::Completed));
    }

    #[test]
    fn all_settled_false_when_any_running() {
        let mut r = RunOnce::new();
        r.load(manifest("a"));
        r.load(manifest("b"));
        r.record("a", RunOnceResult::Completed);
        r.record("b", RunOnceResult::Running);
        assert!(!r.all_settled());
    }

    #[test]
    fn all_settled_true_when_every_pod_finished() {
        let mut r = RunOnce::new();
        r.load(manifest("a"));
        r.load(manifest("b"));
        r.record("a", RunOnceResult::Completed);
        r.record(
            "b",
            RunOnceResult::Failed {
                reason: "crash".into(),
            },
        );
        assert!(r.all_settled());
    }

    #[test]
    fn all_settled_true_when_no_manifests() {
        let r = RunOnce::new();
        assert!(r.all_settled());
    }

    #[test]
    fn restart_policy_round_trips() {
        for p in [
            RestartPolicy::Always,
            RestartPolicy::OnFailure,
            RestartPolicy::Never,
        ] {
            let mut m = manifest("a");
            m.restart_policy = p;
            assert_eq!(m.restart_policy, p);
        }
    }
}
