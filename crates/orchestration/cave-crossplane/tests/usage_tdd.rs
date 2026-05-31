// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Strict-TDD coverage for the Usage protection subsystem (src/usage.rs).
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   - apis/protection/v1beta1/usage_types.go
//!   - internal/controller/protection/usage/reconciler.go
//!
//! The Usage resource declares that one resource (`by`) uses another (`of`),
//! gating deletion of the `of` resource while any Usage references it. The
//! deletion-admission decision, the in-use label policy, and the
//! `replayDeletion` planning are all pure in-crate policy — the only
//! apiserver-coupled residual is the finalizer write-back + webhook
//! registration (cave-apiserver).

use cave_crossplane::usage::{
    DeletionDecision, ResourceTarget, Usage, UsageStore, FINALIZER, IN_USE_LABEL,
};

fn target(api: &str, kind: &str, name: &str) -> ResourceTarget {
    ResourceTarget::new(api, kind, name)
}

#[test]
fn constants_match_upstream() {
    assert_eq!(IN_USE_LABEL, "crossplane.io/in-use");
    assert_eq!(FINALIZER, "usage.apiextensions.crossplane.io");
    // The in-use label map helper carries the literal "true" value.
    let m = UsageStore::in_use_label_map();
    assert_eq!(m.get(IN_USE_LABEL).map(String::as_str), Some("true"));
}

#[test]
fn register_and_find_usages_of_target() {
    let store = UsageStore::new();
    let u = Usage::new("u1", target("nop.example/v1", "Bucket", "my-bucket"))
        .with_by(target("apps/v1", "Deployment", "consumer"));
    store.register(u);

    let found = store.usages_of(&target("nop.example/v1", "Bucket", "my-bucket"));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "u1");

    // A different resource is not referenced.
    assert!(store
        .usages_of(&target("nop.example/v1", "Bucket", "other"))
        .is_empty());
}

#[test]
fn namespace_disambiguates_targets() {
    let store = UsageStore::new();
    let of = ResourceTarget::new("v1", "ConfigMap", "cm").in_namespace("ns-a");
    store.register(Usage::new("u-ns", of));

    // Same name/kind but a different namespace must NOT match.
    let other_ns = ResourceTarget::new("v1", "ConfigMap", "cm").in_namespace("ns-b");
    assert!(store.usages_of(&other_ns).is_empty());

    let same_ns = ResourceTarget::new("v1", "ConfigMap", "cm").in_namespace("ns-a");
    assert_eq!(store.usages_of(&same_ns).len(), 1);
}

#[test]
fn deletion_denied_while_referenced() {
    let store = UsageStore::new();
    let of = target("nop.example/v1", "Bucket", "b");
    store.register(
        Usage::new("u1", of.clone())
            .with_reason("bucket is in use by the website deployment"),
    );

    match store.admit_deletion(&of) {
        DeletionDecision::Denied { by_usages, message } => {
            assert_eq!(by_usages, vec!["u1".to_string()]);
            assert!(message.contains("in use"), "message: {message}");
        }
        DeletionDecision::Allowed => panic!("expected deletion to be denied"),
    }
    assert!(store.is_protected(&of));
}

#[test]
fn deletion_allowed_when_not_referenced() {
    let store = UsageStore::new();
    store.register(Usage::new("u1", target("nop.example/v1", "Bucket", "b")));
    let free = target("nop.example/v1", "Bucket", "free");
    assert_eq!(store.admit_deletion(&free), DeletionDecision::Allowed);
    assert!(!store.is_protected(&free));
}

#[test]
fn multiple_usages_all_listed_and_removal_unblocks() {
    let store = UsageStore::new();
    let of = target("nop.example/v1", "Bucket", "b");
    store.register(Usage::new("u1", of.clone()));
    store.register(Usage::new("u2", of.clone()));

    match store.admit_deletion(&of) {
        DeletionDecision::Denied { by_usages, .. } => {
            assert_eq!(by_usages, vec!["u1".to_string(), "u2".to_string()]);
        }
        DeletionDecision::Allowed => panic!("expected denied"),
    }

    // Removing one still leaves it protected; removing both frees it.
    store.remove("u1");
    assert!(matches!(
        store.admit_deletion(&of),
        DeletionDecision::Denied { .. }
    ));
    store.remove("u2");
    assert_eq!(store.admit_deletion(&of), DeletionDecision::Allowed);
}

#[test]
fn replay_planning_requires_flag_and_recorded_attempt() {
    let store = UsageStore::new();
    let of = target("nop.example/v1", "Bucket", "b");
    // Usage with replayDeletion = true.
    store.register(Usage::new("u1", of.clone()).with_replay_deletion(true));

    // No deletion was attempted yet → no replay even though the flag is set.
    assert!(store.plan_replay("u1").is_none());

    // A blocked delete records the attempt (mirrors the webhook stamping the
    // crossplane.io/deletion-attempt annotation on the `of` resource).
    let decision = store.admit_deletion(&of);
    assert!(matches!(decision, DeletionDecision::Denied { .. }));
    assert!(store.had_deletion_attempt(&of));

    // Now removing the Usage replays the delete on `of`.
    let replay = store.plan_replay("u1").expect("replay should be planned");
    assert_eq!(replay.name, "b");
    assert_eq!(replay.kind, "Bucket");
}

#[test]
fn replay_not_planned_when_flag_unset() {
    let store = UsageStore::new();
    let of = target("nop.example/v1", "Bucket", "b");
    store.register(Usage::new("u1", of.clone())); // replayDeletion defaults to false
    let _ = store.admit_deletion(&of); // records the attempt
    assert!(store.had_deletion_attempt(&of));
    assert!(
        store.plan_replay("u1").is_none(),
        "replayDeletion unset → no replay even after a recorded attempt"
    );
}

#[test]
fn unconditional_protection_when_by_absent() {
    // A Usage with no `by` protects the `of` resource unconditionally
    // (a "protection" usage, not a dependency usage).
    let store = UsageStore::new();
    let of = target("nop.example/v1", "Bucket", "b");
    store.register(Usage::new("protect", of.clone()));
    assert!(store.usages_of(&of)[0].by.is_none());
    assert!(matches!(
        store.admit_deletion(&of),
        DeletionDecision::Denied { .. }
    ));
}
