// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `legacyserviceaccounttokencleaner` — removes the
//! auto-generated `ServiceAccount` token Secrets that were
//! deprecated by KEP-2799 (v1.24+). Kubernetes installs that
//! were upgraded from <v1.24 still carry these secrets even
//! though the apiserver no longer creates new ones.
//!
//! Mirrors `pkg/controller/legacyserviceaccounttokencleaner/`
//! from upstream. The reconciler:
//!
//! 1. Lists every `Secret` of type
//!    `kubernetes.io/service-account-token`.
//! 2. Skips secrets touched within the configured grace
//!    window (default 365 d) — operators may still need them.
//! 3. Skips secrets that are referenced by a still-existing
//!    `ServiceAccount.spec.secrets[]` slot (manual binding).
//! 4. Deletes the rest.

use std::collections::BTreeSet;

/// One observed secret + its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedSecret {
    pub namespace: String,
    pub name: String,
    /// Service-account this secret was minted for. May or may
    /// not still exist.
    pub service_account: String,
    /// Unix seconds the secret was last accessed/used. Drives
    /// the grace check.
    pub last_used_unix: i64,
    /// `true` if some ServiceAccount.spec.secrets[] entry
    /// references this secret by name (manual binding —
    /// upstream still respects these).
    pub referenced_by_sa: bool,
}

/// What the cleaner decides for one secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Within grace window — leave alone.
    Skip,
    /// SA spec still references this secret — preserved.
    PreservedByReference,
    /// Eligible for delete.
    Delete,
}

#[derive(Debug, Clone, Copy)]
pub struct CleanerConfig {
    /// Seconds a secret must be untouched before deletion is
    /// eligible. Upstream default: 365 days.
    pub grace_seconds: i64,
    /// Maximum number of secrets to delete per pass — bounded
    /// so the GC doesn't blow up the apiserver write rate.
    pub max_per_pass: usize,
}

impl Default for CleanerConfig {
    fn default() -> Self {
        Self {
            grace_seconds: 365 * 86_400,
            max_per_pass: 100,
        }
    }
}

/// Plan a single reconciliation pass. Returns the (namespace,
/// name) pairs to delete in stable alphabetical order, capped
/// at `cfg.max_per_pass`.
pub fn plan_pass(
    now: i64,
    cfg: &CleanerConfig,
    secrets: &[ObservedSecret],
    live_service_accounts: &BTreeSet<(String, String)>,
) -> Vec<(String, String)> {
    let mut decisions: Vec<(String, String, Action)> = secrets
        .iter()
        .map(|s| {
            let action = evaluate(now, cfg, s, live_service_accounts);
            (s.namespace.clone(), s.name.clone(), action)
        })
        .collect();
    decisions.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    decisions
        .into_iter()
        .filter_map(|(ns, name, a)| if a == Action::Delete { Some((ns, name)) } else { None })
        .take(cfg.max_per_pass)
        .collect()
}

/// Per-secret decision. Pure.
pub fn evaluate(
    now: i64,
    cfg: &CleanerConfig,
    s: &ObservedSecret,
    live_service_accounts: &BTreeSet<(String, String)>,
) -> Action {
    if s.referenced_by_sa {
        return Action::PreservedByReference;
    }
    if now.saturating_sub(s.last_used_unix) < cfg.grace_seconds {
        return Action::Skip;
    }
    // SA gone? Definitely deletable. SA present? Still
    // deletable — KEP-2799 deprecation; the SA's spec.secrets
    // would have set the reference flag if it cared.
    let _sa_present = live_service_accounts
        .contains(&(s.namespace.clone(), s.service_account.clone()));
    Action::Delete
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(ns: &str, name: &str, last: i64, referenced: bool) -> ObservedSecret {
        ObservedSecret {
            namespace: ns.into(),
            name: name.into(),
            service_account: format!("sa-{name}"),
            last_used_unix: last,
            referenced_by_sa: referenced,
        }
    }

    #[test]
    fn skip_when_within_grace() {
        let cfg = CleanerConfig::default();
        let s = secret("ns", "a", 1_000, false);
        // now only 10 days after last_used.
        let now = 1_000 + 10 * 86_400;
        assert_eq!(evaluate(now, &cfg, &s, &BTreeSet::new()), Action::Skip);
    }

    #[test]
    fn delete_when_past_grace() {
        let cfg = CleanerConfig::default();
        let s = secret("ns", "a", 0, false);
        let now = cfg.grace_seconds + 1;
        assert_eq!(evaluate(now, &cfg, &s, &BTreeSet::new()), Action::Delete);
    }

    #[test]
    fn preserved_when_referenced() {
        let cfg = CleanerConfig::default();
        let s = secret("ns", "a", 0, true);
        let now = cfg.grace_seconds + 1;
        assert_eq!(
            evaluate(now, &cfg, &s, &BTreeSet::new()),
            Action::PreservedByReference
        );
    }

    #[test]
    fn plan_returns_alphabetical_pairs() {
        let cfg = CleanerConfig::default();
        let now = cfg.grace_seconds + 1;
        let secs = vec![
            secret("ns", "z", 0, false),
            secret("ns", "a", 0, false),
            secret("ns", "m", 0, false),
        ];
        let plan = plan_pass(now, &cfg, &secs, &BTreeSet::new());
        let names: Vec<&str> = plan.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, vec!["a", "m", "z"]);
    }

    #[test]
    fn plan_respects_max_per_pass() {
        let mut cfg = CleanerConfig::default();
        cfg.max_per_pass = 2;
        let now = cfg.grace_seconds + 1;
        let secs: Vec<ObservedSecret> =
            (0..5).map(|i| secret("ns", &format!("s{i}"), 0, false)).collect();
        let plan = plan_pass(now, &cfg, &secs, &BTreeSet::new());
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn referenced_excluded_from_plan() {
        let cfg = CleanerConfig::default();
        let now = cfg.grace_seconds + 1;
        let secs = vec![
            secret("ns", "a", 0, false),
            secret("ns", "b", 0, true), // referenced
            secret("ns", "c", 0, false),
        ];
        let plan = plan_pass(now, &cfg, &secs, &BTreeSet::new());
        let names: Vec<&str> = plan.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
    }

    #[test]
    fn within_grace_excluded_from_plan() {
        let cfg = CleanerConfig::default();
        // Set `now` beyond grace from epoch so the "ancient"
        // secret is eligible; "a" was used 1 second ago and
        // is within grace.
        let now = cfg.grace_seconds + 10;
        let secs = vec![
            secret("ns", "a", now - 1, false), // fresh
            secret("ns", "b", 0, false),       // ancient
        ];
        let plan = plan_pass(now, &cfg, &secs, &BTreeSet::new());
        let names: Vec<&str> = plan.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, vec!["b"]);
    }

    #[test]
    fn plan_ordering_is_stable_across_namespaces() {
        let cfg = CleanerConfig::default();
        let now = cfg.grace_seconds + 1;
        let secs = vec![
            secret("ns2", "a", 0, false),
            secret("ns1", "z", 0, false),
            secret("ns1", "b", 0, false),
        ];
        let plan = plan_pass(now, &cfg, &secs, &BTreeSet::new());
        let pairs: Vec<(String, String)> = plan;
        assert_eq!(
            pairs,
            vec![
                ("ns1".into(), "b".into()),
                ("ns1".into(), "z".into()),
                ("ns2".into(), "a".into()),
            ]
        );
    }

    #[test]
    fn empty_input_returns_empty_plan() {
        let cfg = CleanerConfig::default();
        let plan = plan_pass(1_000_000, &cfg, &[], &BTreeSet::new());
        assert!(plan.is_empty());
    }
}
