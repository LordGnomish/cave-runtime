// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Gatekeeper expansion engine (gatekeeper pkg/expansion/*).
//!
//! Faithful line-port of:
//!   - gatekeeper/pkg/expansion/system.go  (Expand / expandResource /
//!     ValidateTemplate / mockNameForResource / ensureOwnerReference /
//!     expandRecursive maxRecursionDepth)
//!   - gatekeeper/pkg/expansion/db.go      (matchers/generators adjacency +
//!     templatesForGVK + cycle detection)
//!   - gatekeeper/pkg/mutation/match/apply_to.go (ApplyTo::matches / flatten)
//!
//! Source SHA eda110bdaf2510288dccd73a1be4dd0c6442a4aa (v3.22.2).

use cave_policy::expansion::{
    ApplyTo, ExpansionSystem, ExpansionTemplate, ExpansionTemplateSpec, GeneratedGvk,
    GroupVersionKind,
};
use serde_json::json;

fn gvk(g: &str, v: &str, k: &str) -> GroupVersionKind {
    GroupVersionKind {
        group: g.into(),
        version: v.into(),
        kind: k.into(),
    }
}

fn deployment_template() -> ExpansionTemplate {
    // Mirrors the canonical "expand-deployments" fixture: a Deployment's
    // spec.template (a Pod template) is expanded into a Pod resource.
    ExpansionTemplate {
        name: "expand-deployments".into(),
        spec: ExpansionTemplateSpec {
            apply_to: vec![ApplyTo {
                groups: vec!["apps".into()],
                versions: vec!["v1".into()],
                kinds: vec!["Deployment".into()],
            }],
            template_source: "spec.template".into(),
            generated_gvk: GeneratedGvk {
                group: "".into(),
                version: "v1".into(),
                kind: "Pod".into(),
            },
            enforcement_action: "".into(),
        },
    }
}

fn deployment_obj() -> serde_json::Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": "nginx-deployment", "namespace": "prod" },
        "spec": {
            "template": {
                "metadata": { "labels": { "app": "nginx" } },
                "spec": { "containers": [{ "name": "nginx", "image": "nginx:1.2.3" }] }
            }
        }
    })
}

#[test]
fn test_apply_to_matches() {
    let a = ApplyTo {
        groups: vec!["apps".into()],
        versions: vec!["v1".into()],
        kinds: vec!["Deployment".into()],
    };
    assert!(a.matches(&gvk("apps", "v1", "Deployment")));
    assert!(!a.matches(&gvk("apps", "v1", "StatefulSet")));
    assert!(!a.matches(&gvk("", "v1", "Deployment")));
    assert!(!a.matches(&gvk("apps", "v1beta1", "Deployment")));
}

#[test]
fn test_apply_to_flatten_cartesian() {
    let a = ApplyTo {
        groups: vec!["apps".into()],
        versions: vec!["v1".into(), "v1beta1".into()],
        kinds: vec!["Deployment".into(), "StatefulSet".into()],
    };
    // 1 group × 2 versions × 2 kinds = 4
    assert_eq!(a.flatten().len(), 4);
}

#[test]
fn test_validate_template_rejects_empty_name() {
    let mut t = deployment_template();
    t.name = "".into();
    let mut sys = ExpansionSystem::new();
    assert!(sys.upsert_template(t).is_err());
}

#[test]
fn test_validate_template_rejects_long_name() {
    let mut t = deployment_template();
    t.name = "a".repeat(64);
    let mut sys = ExpansionSystem::new();
    assert!(sys.upsert_template(t).is_err());
}

#[test]
fn test_validate_template_rejects_empty_source() {
    let mut t = deployment_template();
    t.spec.template_source = "".into();
    let mut sys = ExpansionSystem::new();
    assert!(sys.upsert_template(t).is_err());
}

#[test]
fn test_validate_template_rejects_empty_apply_to() {
    let mut t = deployment_template();
    t.spec.apply_to = vec![];
    let mut sys = ExpansionSystem::new();
    assert!(sys.upsert_template(t).is_err());
}

#[test]
fn test_validate_template_rejects_self_edge() {
    // A template that applies to the same GVK it generates is a self-edge.
    let mut t = deployment_template();
    t.spec.generated_gvk = GeneratedGvk {
        group: "apps".into(),
        version: "v1".into(),
        kind: "Deployment".into(),
    };
    let mut sys = ExpansionSystem::new();
    assert!(sys.upsert_template(t).is_err());
}

#[test]
fn test_expand_deployment_to_pod() {
    let mut sys = ExpansionSystem::new();
    sys.upsert_template(deployment_template()).unwrap();

    let resultants = sys.expand(&deployment_obj()).unwrap();
    assert_eq!(resultants.len(), 1);
    let r = &resultants[0];
    assert_eq!(r.template_name, "expand-deployments");

    // GVK rewritten to the generatedGVK (Pod).
    assert_eq!(r.obj["apiVersion"], "v1");
    assert_eq!(r.obj["kind"], "Pod");

    // The source field (spec.template) becomes the new object's content.
    assert_eq!(r.obj["spec"]["containers"][0]["image"], "nginx:1.2.3");

    // mockNameForResource: "<generator name>-<lowercased kind>".
    assert_eq!(r.obj["metadata"]["name"], "nginx-deployment-pod");
    // Namespace inherited from parent (no Namespace override passed).
    assert_eq!(r.obj["metadata"]["namespace"], "prod");

    // ownerReference appended.
    let owners = &r.obj["metadata"]["ownerReferences"];
    assert_eq!(owners[0]["apiVersion"], "apps/v1");
    assert_eq!(owners[0]["kind"], "Deployment");
    assert_eq!(owners[0]["name"], "nginx-deployment");
}

#[test]
fn test_expand_no_match_returns_empty() {
    let mut sys = ExpansionSystem::new();
    sys.upsert_template(deployment_template()).unwrap();
    // A StatefulSet does not match the apply_to.
    let obj = json!({
        "apiVersion": "apps/v1",
        "kind": "StatefulSet",
        "metadata": { "name": "db" },
        "spec": { "template": {} }
    });
    let resultants = sys.expand(&obj).unwrap();
    assert!(resultants.is_empty());
}

#[test]
fn test_expand_recursive_two_hops() {
    // Template A: Deployment -> Pod.  Template B: Pod -> ConfigMap (contrived).
    // expandRecursive should produce BOTH resultants from a single Deployment.
    let mut sys = ExpansionSystem::new();
    sys.upsert_template(deployment_template()).unwrap();

    let pod_to_cm = ExpansionTemplate {
        name: "expand-pods".into(),
        spec: ExpansionTemplateSpec {
            apply_to: vec![ApplyTo {
                groups: vec!["".into()],
                versions: vec!["v1".into()],
                kinds: vec!["Pod".into()],
            }],
            template_source: "spec".into(),
            generated_gvk: GeneratedGvk {
                group: "".into(),
                version: "v1".into(),
                kind: "ConfigMap".into(),
            },
            enforcement_action: "".into(),
        },
    };
    sys.upsert_template(pod_to_cm).unwrap();

    let resultants = sys.expand(&deployment_obj()).unwrap();
    // One Pod (from Deployment) + one ConfigMap (from that Pod) = 2.
    assert_eq!(resultants.len(), 2);
    let kinds: Vec<String> = resultants
        .iter()
        .map(|r| r.obj["kind"].as_str().unwrap().to_string())
        .collect();
    assert!(kinds.contains(&"Pod".to_string()));
    assert!(kinds.contains(&"ConfigMap".to_string()));
}

#[test]
fn test_expand_cycle_is_rejected_on_upsert() {
    // A: Deployment(apply) -> Pod(generate);  B: Pod(apply) -> Deployment(generate).
    // Adding B closes a cycle Pod->Deployment->Pod and must error.
    let mut sys = ExpansionSystem::new();
    sys.upsert_template(deployment_template()).unwrap();

    let pod_to_deploy = ExpansionTemplate {
        name: "expand-pods-cycle".into(),
        spec: ExpansionTemplateSpec {
            apply_to: vec![ApplyTo {
                groups: vec!["".into()],
                versions: vec!["v1".into()],
                kinds: vec!["Pod".into()],
            }],
            template_source: "spec".into(),
            generated_gvk: GeneratedGvk {
                group: "apps".into(),
                version: "v1".into(),
                kind: "Deployment".into(),
            },
            enforcement_action: "".into(),
        },
    };
    assert!(sys.upsert_template(pod_to_deploy).is_err());
}

#[test]
fn test_mock_name_lowercased() {
    // Directly exercise the documented naming contract via expand.
    let mut sys = ExpansionSystem::new();
    let mut t = deployment_template();
    t.spec.generated_gvk.kind = "MyResource".into();
    sys.upsert_template(t).unwrap();
    let resultants = sys.expand(&deployment_obj()).unwrap();
    assert_eq!(
        resultants[0].obj["metadata"]["name"],
        "nginx-deployment-myresource"
    );
}
