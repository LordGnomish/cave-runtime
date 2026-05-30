// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Port of pkg/scheduling/taints.go from kubernetes-sigs/karpenter v1.12.1
// (sha ed490e8): the `Taints` decorated slice with its k8s toleration-matching
// semantics (ToleratesTaint / MatchTaint), `Tolerates`, `Merge`, and the
// `KnownEphemeralTaints` table. The toleration matcher reproduces upstream
// k8s `(*Toleration).ToleratesTaint` (k8s.io/api/core/v1) and `(*Taint).
// MatchTaint`, which karpenter relies on verbatim.

use cave_karpenter::scheduling::taints::{
    Effect, Operator, Taint, Taints, Toleration, KNOWN_EPHEMERAL_TAINTS,
};

fn taint(key: &str, value: Option<&str>, effect: Effect) -> Taint {
    Taint {
        key: key.to_string(),
        value: value.map(str::to_string),
        effect,
    }
}

// ---- Toleration::tolerates_taint (k8s semantics) -----------------------------

#[test]
fn equal_operator_tolerates_matching_key_value_effect() {
    let t = Toleration {
        key: Some("dedicated".into()),
        operator: Operator::Equal,
        value: Some("gpu".into()),
        effect: Some(Effect::NoSchedule),
    };
    assert!(t.tolerates_taint(&taint("dedicated", Some("gpu"), Effect::NoSchedule)));
}

#[test]
fn equal_operator_rejects_value_mismatch() {
    let t = Toleration {
        key: Some("dedicated".into()),
        operator: Operator::Equal,
        value: Some("gpu".into()),
        effect: Some(Effect::NoSchedule),
    };
    assert!(!t.tolerates_taint(&taint("dedicated", Some("cpu"), Effect::NoSchedule)));
}

#[test]
fn exists_operator_tolerates_any_value_for_key() {
    let t = Toleration {
        key: Some("dedicated".into()),
        operator: Operator::Exists,
        value: None,
        effect: Some(Effect::NoSchedule),
    };
    assert!(t.tolerates_taint(&taint("dedicated", Some("anything"), Effect::NoSchedule)));
}

#[test]
fn empty_effect_tolerates_any_effect() {
    // toleration.Effect == "" matches every effect for that key
    let t = Toleration {
        key: Some("dedicated".into()),
        operator: Operator::Exists,
        value: None,
        effect: None,
    };
    assert!(t.tolerates_taint(&taint("dedicated", None, Effect::NoSchedule)));
    assert!(t.tolerates_taint(&taint("dedicated", None, Effect::NoExecute)));
}

#[test]
fn empty_key_with_exists_is_universal_toleration() {
    // key == "" && operator Exists tolerates everything
    let t = Toleration {
        key: None,
        operator: Operator::Exists,
        value: None,
        effect: None,
    };
    assert!(t.tolerates_taint(&taint("any-key", Some("v"), Effect::NoExecute)));
    assert!(t.tolerates_taint(&taint("other", None, Effect::NoSchedule)));
}

#[test]
fn effect_mismatch_is_not_tolerated() {
    let t = Toleration {
        key: Some("dedicated".into()),
        operator: Operator::Exists,
        value: None,
        effect: Some(Effect::NoSchedule),
    };
    assert!(!t.tolerates_taint(&taint("dedicated", None, Effect::NoExecute)));
}

#[test]
fn key_mismatch_is_not_tolerated() {
    let t = Toleration {
        key: Some("dedicated".into()),
        operator: Operator::Exists,
        value: None,
        effect: None,
    };
    assert!(!t.tolerates_taint(&taint("other", None, Effect::NoSchedule)));
}

// ---- Taints::tolerates -------------------------------------------------------

#[test]
fn tolerates_ok_when_every_taint_tolerated() {
    let ts = Taints::from(vec![
        taint("a", Some("1"), Effect::NoSchedule),
        taint("b", None, Effect::NoExecute),
    ]);
    let tols = vec![
        Toleration {
            key: None,
            operator: Operator::Exists,
            value: None,
            effect: None,
        }, // universal
    ];
    assert!(ts.tolerates(&tols).is_ok());
}

#[test]
fn tolerates_err_lists_untolerated_taint() {
    let ts = Taints::from(vec![
        taint("a", Some("1"), Effect::NoSchedule),
        taint("b", None, Effect::NoExecute),
    ]);
    // only tolerates "a"
    let tols = vec![Toleration {
        key: Some("a".into()),
        operator: Operator::Exists,
        value: None,
        effect: None,
    }];
    let err = ts.tolerates(&tols).expect_err("b is not tolerated");
    assert!(err.untolerated.iter().any(|t| t.key == "b"));
    assert!(!err.untolerated.iter().any(|t| t.key == "a"));
}

#[test]
fn tolerates_pod_delegates_to_pod_tolerations() {
    let ts = Taints::from(vec![taint("a", Some("1"), Effect::NoSchedule)]);
    let pod_tols = vec![Toleration {
        key: Some("a".into()),
        operator: Operator::Equal,
        value: Some("1".into()),
        effect: Some(Effect::NoSchedule),
    }];
    assert!(ts.tolerates_pod(&pod_tols).is_ok());
}

// ---- Taint::matches_taint + Merge --------------------------------------------

#[test]
fn match_taint_compares_key_and_effect_only() {
    let a = taint("k", Some("v1"), Effect::NoSchedule);
    let b = taint("k", Some("v2"), Effect::NoSchedule); // different value
    let c = taint("k", Some("v1"), Effect::NoExecute); // different effect
    assert!(a.matches_taint(&b), "value is ignored by MatchTaint");
    assert!(!a.matches_taint(&c), "effect difference breaks match");
}

#[test]
fn merge_appends_only_unmatched_taints() {
    let base = Taints::from(vec![taint("a", Some("1"), Effect::NoSchedule)]);
    let with = Taints::from(vec![
        taint("a", Some("999"), Effect::NoSchedule), // matches base (key+effect) → skipped
        taint("b", None, Effect::NoExecute),         // new → appended
    ]);
    let merged = base.merge(&with);
    let keys: Vec<&str> = merged.iter().map(|t| t.key.as_str()).collect();
    assert_eq!(keys, vec!["a", "b"]);
    // base's original "a" value preserved (not overwritten by with's 999)
    let a = merged.iter().find(|t| t.key == "a").unwrap();
    assert_eq!(a.value.as_deref(), Some("1"));
}

// ---- KnownEphemeralTaints ----------------------------------------------------

#[test]
fn known_ephemeral_taints_table() {
    let by_key = |k: &str, e: Effect| {
        KNOWN_EPHEMERAL_TAINTS
            .iter()
            .any(|t| t.key == k && t.effect == e)
    };
    assert!(by_key("node.kubernetes.io/not-ready", Effect::NoSchedule));
    assert!(by_key("node.kubernetes.io/not-ready", Effect::NoExecute));
    assert!(by_key("node.kubernetes.io/unreachable", Effect::NoSchedule));
    assert!(by_key(
        "node.cloudprovider.kubernetes.io/uninitialized",
        Effect::NoSchedule
    ));
    assert!(by_key("karpenter.sh/unregistered", Effect::NoExecute));
    assert_eq!(KNOWN_EPHEMERAL_TAINTS.len(), 5);
}

#[test]
fn external_cloud_provider_taint_carries_true_value() {
    let t = KNOWN_EPHEMERAL_TAINTS
        .iter()
        .find(|t| t.key == "node.cloudprovider.kubernetes.io/uninitialized")
        .unwrap();
    assert_eq!(t.value.as_deref(), Some("true"));
}
