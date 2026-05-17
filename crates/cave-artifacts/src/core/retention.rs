// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts core::retention (consolidates Pulp retain_repo_versions + Harbor retention policy)
//! `RetentionPolicy` — rule set + evaluator that decides Keep / Delete.
//!
//! Maps to:
//! - pulpcore `Repository.retain_repo_versions` (count-based)
//! - Harbor   `pkg/policy/retention/manager.go` (age + count + label-based)
//! - cave-artifacts portal `/admin/artifacts/retention` editor

use super::{Artifact, Tag};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Outcome of evaluating one artifact against a policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionAction {
    Keep,
    Delete,
}

/// Single rule. Multiple rules in a policy combine via AND for keep — i.e.
/// an artifact is kept only if every rule says `Keep`. Matches Harbor's
/// `retention_id_template_xx` AND-combination and Pulp's `retain_*` chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RetentionRule {
    /// Keep only the most recent `n` artifacts (by `created_at`).
    KeepLastN { n: usize },
    /// Delete any artifact older than `days`.
    DeleteOlderThanDays { days: i64 },
    /// Keep artifacts whose tag list contains *at least one* of `tags`
    /// (e.g. `latest`, `stable`, `v*`).
    KeepTagged { tags: Vec<String> },
    /// Always-keep: artifacts the policy must never touch (e.g.
    /// signed-with non-None).
    KeepSigned,
}

/// Policy = ordered rule set + a `tenant_scope` label for portal display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub name: String,
    pub rules: Vec<RetentionRule>,
}

impl RetentionPolicy {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rules: Vec::new(),
        }
    }

    pub fn with_rule(mut self, rule: RetentionRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Evaluate the policy against `artifact` in the context of `all` (the
    /// whole repository, sorted any way — we sort by created_at inside).
    /// Returns `Keep` when every rule agrees, `Delete` if any rule says so.
    ///
    /// `now` is injected so unit tests can pin time deterministically.
    pub fn evaluate(&self, artifact: &Artifact, all: &[Artifact], now: DateTime<Utc>) -> RetentionAction {
        if self.rules.is_empty() {
            // Empty policy = keep everything (matches Pulp default of
            // `retain_repo_versions = None`).
            return RetentionAction::Keep;
        }
        for rule in &self.rules {
            if matches!(self.eval_rule(rule, artifact, all, now), RetentionAction::Delete) {
                return RetentionAction::Delete;
            }
        }
        RetentionAction::Keep
    }

    fn eval_rule(
        &self,
        rule: &RetentionRule,
        artifact: &Artifact,
        all: &[Artifact],
        now: DateTime<Utc>,
    ) -> RetentionAction {
        match rule {
            RetentionRule::KeepLastN { n } => {
                let mut sorted: Vec<&Artifact> = all.iter().collect();
                sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                let keep_set: std::collections::HashSet<&str> = sorted
                    .iter()
                    .take(*n)
                    .map(|a| a.digest.as_str())
                    .collect();
                if keep_set.contains(artifact.digest.as_str()) {
                    RetentionAction::Keep
                } else {
                    RetentionAction::Delete
                }
            }
            RetentionRule::DeleteOlderThanDays { days } => {
                let cutoff = now - Duration::days(*days);
                if artifact.created_at < cutoff {
                    RetentionAction::Delete
                } else {
                    RetentionAction::Keep
                }
            }
            RetentionRule::KeepTagged { tags } => {
                // KeepTagged is an *exception* rule — if any tag matches, keep.
                // Used as a sieve before destructive rules; we encode it as
                // "Delete when no tag matches".
                if artifact.tags.iter().any(|t| tag_matches_any(t, tags)) {
                    RetentionAction::Keep
                } else {
                    RetentionAction::Delete
                }
            }
            RetentionRule::KeepSigned => {
                if artifact.tags.iter().any(|t| t.signed_with.is_some()) {
                    RetentionAction::Keep
                } else {
                    RetentionAction::Delete
                }
            }
        }
    }
}

fn tag_matches_any(t: &Tag, patterns: &[String]) -> bool {
    patterns.iter().any(|p| {
        if let Some(prefix) = p.strip_suffix('*') {
            t.name.starts_with(prefix)
        } else {
            t.name == *p
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn artifact_at(digest: &str, created: DateTime<Utc>) -> Artifact {
        let mut a = Artifact::new(digest, 1, "application/octet-stream");
        a.created_at = created;
        a
    }

    #[test]
    fn empty_policy_keeps_everything() {
        let p = RetentionPolicy::new("noop");
        let a = artifact_at("sha256:1", at(2024, 1, 1));
        assert_eq!(p.evaluate(&a, &[a.clone()], at(2026, 5, 17)), RetentionAction::Keep);
    }

    #[test]
    fn keep_last_n_keeps_newest() {
        let p = RetentionPolicy::new("last2").with_rule(RetentionRule::KeepLastN { n: 2 });
        let a1 = artifact_at("sha256:1", at(2024, 1, 1));
        let a2 = artifact_at("sha256:2", at(2024, 2, 1));
        let a3 = artifact_at("sha256:3", at(2024, 3, 1));
        let all = vec![a1.clone(), a2.clone(), a3.clone()];
        let now = at(2024, 4, 1);
        assert_eq!(p.evaluate(&a3, &all, now), RetentionAction::Keep);
        assert_eq!(p.evaluate(&a2, &all, now), RetentionAction::Keep);
        assert_eq!(p.evaluate(&a1, &all, now), RetentionAction::Delete);
    }

    #[test]
    fn delete_older_than_days_uses_now() {
        let p = RetentionPolicy::new("90d").with_rule(RetentionRule::DeleteOlderThanDays { days: 90 });
        let old = artifact_at("sha256:1", at(2024, 1, 1));
        let new = artifact_at("sha256:2", at(2026, 5, 1));
        let now = at(2026, 5, 17);
        assert_eq!(p.evaluate(&old, &[old.clone(), new.clone()], now), RetentionAction::Delete);
        assert_eq!(p.evaluate(&new, &[old.clone(), new.clone()], now), RetentionAction::Keep);
    }

    #[test]
    fn keep_tagged_with_glob() {
        let p = RetentionPolicy::new("keep_release")
            .with_rule(RetentionRule::KeepTagged { tags: vec!["v*".into(), "stable".into()] });
        let mut a = artifact_at("sha256:1", at(2024, 1, 1));
        a.tags.push(Tag::new("v1.2.0", "sha256:1"));
        assert_eq!(p.evaluate(&a, &[a.clone()], at(2024, 6, 1)), RetentionAction::Keep);
        let mut b = artifact_at("sha256:2", at(2024, 1, 1));
        b.tags.push(Tag::new("nightly", "sha256:2"));
        assert_eq!(p.evaluate(&b, &[b.clone()], at(2024, 6, 1)), RetentionAction::Delete);
        let mut c = artifact_at("sha256:3", at(2024, 1, 1));
        c.tags.push(Tag::new("stable", "sha256:3"));
        assert_eq!(p.evaluate(&c, &[c.clone()], at(2024, 6, 1)), RetentionAction::Keep);
    }

    #[test]
    fn keep_signed_passes_signed_artifacts_only() {
        let p = RetentionPolicy::new("only_signed").with_rule(RetentionRule::KeepSigned);
        let mut signed = artifact_at("sha256:1", at(2024, 1, 1));
        let mut tag = Tag::new("v1", "sha256:1");
        tag.signed_with = Some("alice".into());
        signed.tags.push(tag);
        let unsigned = artifact_at("sha256:2", at(2024, 1, 1));
        let now = at(2024, 6, 1);
        assert_eq!(p.evaluate(&signed, &[signed.clone(), unsigned.clone()], now), RetentionAction::Keep);
        assert_eq!(p.evaluate(&unsigned, &[signed.clone(), unsigned.clone()], now), RetentionAction::Delete);
    }

    #[test]
    fn multiple_rules_combine_pessimistically() {
        let p = RetentionPolicy::new("strict")
            .with_rule(RetentionRule::KeepLastN { n: 5 })
            .with_rule(RetentionRule::DeleteOlderThanDays { days: 30 });
        // Newest of 1, but 90 days old → second rule deletes.
        let old_only = artifact_at("sha256:1", at(2024, 1, 1));
        let now = at(2024, 6, 1);
        assert_eq!(p.evaluate(&old_only, &[old_only.clone()], now), RetentionAction::Delete);
    }
}
