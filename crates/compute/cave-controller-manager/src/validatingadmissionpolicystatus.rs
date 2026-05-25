// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `validatingadmissionpolicystatus` — reconciler that
//! aggregates per-CRD-version type-check outcomes onto a
//! `ValidatingAdmissionPolicy.status` block.
//!
//! Mirrors `pkg/controller/validatingadmissionpolicystatus/`
//! from upstream. The apiserver type-checks each `VAP` at write
//! time (cave-apiserver/src/vap_advanced.rs); this controller
//! aggregates those per-version outcomes into a single status
//! report on the policy object so the kubectl status column can
//! summarise "compiles cleanly against the matched API group".

use std::collections::BTreeMap;

/// One per-(GroupVersionKind) check outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCheck {
    pub group: String,
    pub version: String,
    pub kind: String,
    /// `Ok` = compiles; `Err(message)` = type error.
    pub result: Result<(), String>,
}

/// Aggregated status the reconciler writes back on the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyStatus {
    /// `True` when every type-check passed, `False` when any
    /// failed, `Unknown` when no checks have run yet.
    pub type_checking: ConditionState,
    /// Generation the status reflects — Kubernetes uses this
    /// to avoid stale status overwrites.
    pub observed_generation: i64,
    /// Per-version warnings — only `Err` results land here.
    pub warnings: Vec<TypeCheckWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionState {
    True,
    False,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCheckWarning {
    pub gvk: String,
    pub message: String,
}

/// Pure reducer — given a generation + per-GVK check results,
/// produces the aggregated status block. Deterministic /
/// order-independent.
pub fn aggregate(generation: i64, checks: &[TypeCheck]) -> PolicyStatus {
    if checks.is_empty() {
        return PolicyStatus {
            type_checking: ConditionState::Unknown,
            observed_generation: generation,
            warnings: Vec::new(),
        };
    }
    let mut warnings: Vec<TypeCheckWarning> = checks
        .iter()
        .filter_map(|c| {
            c.result.as_ref().err().map(|m| TypeCheckWarning {
                gvk: format!("{}/{}/{}", c.group, c.version, c.kind),
                message: m.clone(),
            })
        })
        .collect();
    warnings.sort_by(|a, b| a.gvk.cmp(&b.gvk));
    let type_checking = if warnings.is_empty() {
        ConditionState::True
    } else {
        ConditionState::False
    };
    PolicyStatus {
        type_checking,
        observed_generation: generation,
        warnings,
    }
}

/// Per-policy state the controller carries between
/// reconciliation cycles. The actual `kube` informer / queue
/// lives one layer up; this module is the deterministic core.
#[derive(Debug, Default)]
pub struct StatusReconciler {
    by_policy: BTreeMap<String, PolicyStatus>,
}

impl StatusReconciler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a fresh aggregated status for `policy_name`. Stale
    /// generations are rejected so an out-of-order reconcile
    /// can't roll back the status.
    pub fn upsert(&mut self, policy_name: &str, fresh: PolicyStatus) -> bool {
        let mut applied = true;
        self.by_policy
            .entry(policy_name.to_string())
            .and_modify(|prev| {
                if fresh.observed_generation < prev.observed_generation {
                    applied = false;
                } else {
                    *prev = fresh.clone();
                }
            })
            .or_insert(fresh);
        applied
    }

    pub fn get(&self, policy_name: &str) -> Option<&PolicyStatus> {
        self.by_policy.get(policy_name)
    }

    pub fn len(&self) -> usize {
        self.by_policy.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_policy.is_empty()
    }

    /// Forget a policy — used when the controller observes a
    /// deletion event.
    pub fn forget(&mut self, policy_name: &str) {
        self.by_policy.remove(policy_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(kind: &str, ok: bool) -> TypeCheck {
        TypeCheck {
            group: "apps".into(),
            version: "v1".into(),
            kind: kind.into(),
            result: if ok {
                Ok(())
            } else {
                Err(format!("compile error in {kind}"))
            },
        }
    }

    #[test]
    fn aggregate_empty_returns_unknown() {
        let s = aggregate(1, &[]);
        assert_eq!(s.type_checking, ConditionState::Unknown);
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn aggregate_all_pass_returns_true() {
        let s = aggregate(1, &[check("Deployment", true), check("StatefulSet", true)]);
        assert_eq!(s.type_checking, ConditionState::True);
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn aggregate_any_fail_returns_false() {
        let s = aggregate(1, &[check("Deployment", true), check("StatefulSet", false)]);
        assert_eq!(s.type_checking, ConditionState::False);
        assert_eq!(s.warnings.len(), 1);
        assert!(s.warnings[0].gvk.ends_with("StatefulSet"));
    }

    #[test]
    fn aggregate_warnings_sorted_lex_by_gvk() {
        let s = aggregate(
            1,
            &[check("Z", false), check("A", false), check("M", false)],
        );
        assert_eq!(s.warnings.len(), 3);
        for w in s.warnings.windows(2) {
            assert!(w[0].gvk <= w[1].gvk);
        }
    }

    #[test]
    fn aggregate_observed_generation_threads_through() {
        let s = aggregate(42, &[check("D", true)]);
        assert_eq!(s.observed_generation, 42);
    }

    #[test]
    fn upsert_first_write_succeeds() {
        let mut r = StatusReconciler::new();
        assert!(r.upsert("p", aggregate(1, &[check("D", true)])));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn upsert_rejects_stale_generation() {
        let mut r = StatusReconciler::new();
        r.upsert("p", aggregate(5, &[check("D", true)]));
        let stale = r.upsert("p", aggregate(3, &[check("D", false)]));
        assert!(!stale);
        // Previous status preserved.
        assert_eq!(r.get("p").unwrap().observed_generation, 5);
    }

    #[test]
    fn upsert_accepts_equal_generation() {
        let mut r = StatusReconciler::new();
        r.upsert("p", aggregate(5, &[check("D", true)]));
        let ok = r.upsert("p", aggregate(5, &[check("D", false)]));
        assert!(ok);
        assert_eq!(r.get("p").unwrap().type_checking, ConditionState::False);
    }

    #[test]
    fn upsert_accepts_higher_generation() {
        let mut r = StatusReconciler::new();
        r.upsert("p", aggregate(1, &[check("D", true)]));
        r.upsert("p", aggregate(2, &[check("D", false)]));
        assert_eq!(r.get("p").unwrap().observed_generation, 2);
    }

    #[test]
    fn forget_removes_entry() {
        let mut r = StatusReconciler::new();
        r.upsert("p", aggregate(1, &[]));
        r.forget("p");
        assert!(r.is_empty());
    }
}
