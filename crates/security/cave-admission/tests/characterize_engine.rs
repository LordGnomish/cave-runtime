// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Characterization tests for cave_admission::engine (policy evaluation functions).

use cave_admission::engine::{
    builtin_policies, evaluate_all_policies, evaluate_mutation, evaluate_policy,
    evaluate_validation_rule, evaluate_verify_images, matches_policy,
};
use cave_admission::models::{
    MutationPatch, Operation, PatchOp, Policy, PolicyMatch, PolicyRule, PolicySpec,
    VerifyImagesRule, Resource, ResourceMeta,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn bare_resource(kind: &str, name: &str) -> Resource {
    Resource {
        api_version: "v1".into(),
        kind: kind.into(),
        metadata: ResourceMeta {
            name: name.into(),
            namespace: Some("default".into()),
            labels: HashMap::new(),
            annotations: HashMap::new(),
        },
        spec: serde_json::json!({}),
    }
}

fn minimal_policy(spec: PolicySpec) -> Policy {
    Policy {
        id: Uuid::new_v4(),
        name: "test".into(),
        description: "".into(),
        match_criteria: PolicyMatch {
            kinds: vec!["*".into()],
            namespaces: vec![],
            operations: vec![],
            label_selector: None,
        },
        spec,
        audit_mode: false,
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

// --- matches_policy -------------------------------------------------------

#[test]
fn matches_wildcard_kind() {
    let p = minimal_policy(PolicySpec::Validate { rules: vec![] });
    let r = bare_resource("Pod", "p");
    assert!(matches_policy(&p, &r, Operation::Create));
}

#[test]
fn does_not_match_disabled() {
    let mut p = minimal_policy(PolicySpec::Validate { rules: vec![] });
    p.enabled = false;
    let r = bare_resource("Pod", "p");
    assert!(!matches_policy(&p, &r, Operation::Create));
}

#[test]
fn label_selector_filters_correctly() {
    let mut p = minimal_policy(PolicySpec::Validate { rules: vec![] });
    let mut sel = HashMap::new();
    sel.insert("team".to_string(), "platform".to_string());
    p.match_criteria.label_selector = Some(sel);

    let mut r = bare_resource("Pod", "p");
    assert!(!matches_policy(&p, &r, Operation::Create), "Missing label should not match");
    r.metadata.labels.insert("team".into(), "platform".into());
    assert!(matches_policy(&p, &r, Operation::Create), "Matching label should match");
}

// --- evaluate_validation_rule -------------------------------------------

#[test]
fn required_annotation_passes_when_present() {
    let rule = PolicyRule::RequiredAnnotation { key: "owner".into() };
    let mut r = bare_resource("Pod", "p");
    assert!(evaluate_validation_rule(&rule, &r).is_some(), "Missing annotation → violation");
    r.metadata.annotations.insert("owner".into(), "platform".into());
    assert!(evaluate_validation_rule(&rule, &r).is_none(), "Present annotation → ok");
}

#[test]
fn max_replicas_rule() {
    let rule = PolicyRule::MaxReplicas { max: 3 };
    let mut r = bare_resource("Deployment", "d");
    r.spec = serde_json::json!({"replicas": 5});
    assert!(evaluate_validation_rule(&rule, &r).is_some(), "5 > 3 → violation");
    r.spec = serde_json::json!({"replicas": 3});
    assert!(evaluate_validation_rule(&rule, &r).is_none(), "3 == max → ok");
}

#[test]
fn required_namespace_rule() {
    let rule = PolicyRule::RequiredNamespace { namespaces: vec!["prod".into()] };
    let r_prod = {
        let mut r = bare_resource("Pod", "p");
        r.metadata.namespace = Some("prod".into());
        r
    };
    let r_dev = {
        let mut r = bare_resource("Pod", "p");
        r.metadata.namespace = Some("dev".into());
        r
    };
    assert!(evaluate_validation_rule(&rule, &r_prod).is_none());
    assert!(evaluate_validation_rule(&rule, &r_dev).is_some());
}

// --- evaluate_mutation ---------------------------------------------------

#[test]
fn mutation_returns_all_patches() {
    let patches = vec![
        MutationPatch { op: PatchOp::Add, path: "/metadata/labels/env".into(), value: Some(serde_json::json!("prod")) },
        MutationPatch { op: PatchOp::Remove, path: "/metadata/annotations/debug".into(), value: None },
    ];
    let r = bare_resource("Pod", "p");
    let result = evaluate_mutation(&patches, &r);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].path, "/metadata/labels/env");
}

// --- evaluate_verify_images ----------------------------------------------

#[test]
fn verify_images_allows_digest_ref() {
    let rule = VerifyImagesRule {
        allowed_registries: vec!["gcr.io/".into()],
        require_signature: true,
        key_ref: None,
    };
    let mut r = bare_resource("Pod", "p");
    r.spec = serde_json::json!({"image": "gcr.io/project/app@sha256:abc123"});
    assert!(evaluate_verify_images(&rule, &r).is_none(), "Digest ref should be treated as signed");
}

#[test]
fn verify_images_blocks_wrong_registry() {
    let rule = VerifyImagesRule {
        allowed_registries: vec!["gcr.io/".into()],
        require_signature: false,
        key_ref: None,
    };
    let mut r = bare_resource("Pod", "p");
    r.spec = serde_json::json!({"image": "docker.io/nginx:1.25"});
    assert!(evaluate_verify_images(&rule, &r).is_some());
}

// --- evaluate_policy -------------------------------------------------------

#[test]
fn audit_mode_allows_but_records_violation() {
    let mut policy = minimal_policy(PolicySpec::Validate {
        rules: vec![PolicyRule::RequiredLabel { key: "env".into(), allowed_values: vec![] }],
    });
    policy.audit_mode = true;
    let r = bare_resource("Pod", "p");
    let result = evaluate_policy(&policy, &r, Operation::Create);
    assert!(result.allowed, "Audit mode must allow");
    assert!(!result.violations.is_empty(), "Violation should still be recorded");
}

#[test]
fn generate_policy_builds_companion_resource() {
    use cave_admission::models::GenerateRule;
    let policy = minimal_policy(PolicySpec::Generate {
        generate: GenerateRule {
            kind: "NetworkPolicy".into(),
            api_version: "networking.k8s.io/v1".into(),
            name_template: "{{name}}-netpol".into(),
            spec: serde_json::json!({}),
        },
    });
    let r = bare_resource("Pod", "my-pod");
    let result = evaluate_policy(&policy, &r, Operation::Create);
    assert_eq!(result.generated_resources.len(), 1);
    assert_eq!(
        result.generated_resources[0]["metadata"]["name"],
        serde_json::json!("my-pod-netpol")
    );
}

// --- evaluate_all_policies -----------------------------------------------

#[test]
fn evaluate_all_policies_merges_mutations() {
    let mut p1 = minimal_policy(PolicySpec::Mutate {
        patches: vec![MutationPatch { op: PatchOp::Add, path: "/metadata/labels/a".into(), value: Some(serde_json::json!("1")) }],
    });
    p1.match_criteria.kinds = vec!["Pod".into()];
    let mut p2 = minimal_policy(PolicySpec::Mutate {
        patches: vec![MutationPatch { op: PatchOp::Add, path: "/metadata/labels/b".into(), value: Some(serde_json::json!("2")) }],
    });
    p2.name = "p2".into();
    p2.match_criteria.kinds = vec!["Pod".into()];

    let r = bare_resource("Pod", "p");
    let result = evaluate_all_policies(&[p1, p2], &r, Operation::Create);
    assert!(result.allowed);
    assert_eq!(result.mutations.len(), 2);
}

// --- builtin_policies ---------------------------------------------------

#[test]
fn builtin_policies_all_enabled_and_named() {
    let policies = builtin_policies();
    assert!(policies.len() >= 3);
    for p in &policies {
        assert!(p.enabled);
        assert!(!p.name.is_empty());
    }
}
