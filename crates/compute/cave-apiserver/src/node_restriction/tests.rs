// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NodeRestriction admission tests.

use super::*;
use crate::admission::{AdmissionRequest, Operation};
use crate::resources::{ConfigMap, Namespace, ObjectMeta, Resource};
use std::collections::HashMap;
use std::sync::Arc;

fn req(user: &str, kind: &str, namespace: &str, name: &str, op: Operation) -> AdmissionRequest {
    AdmissionRequest {
        uid: "uid".into(),
        tenant_id: "acme".into(),
        namespace: namespace.into(),
        kind: kind.into(),
        name: name.into(),
        operation: op,
        object: Some(Resource::ConfigMap(ConfigMap {
            api_version: "v1".into(),
            kind: "ConfigMap".into(),
            metadata: ObjectMeta::new(name, namespace),
            data: HashMap::new(),
        })),
        old_object: None,
        user: user.into(),
        dry_run: false,
    }
}

fn lister_for(node: &str, refs: OwnPodReferences) -> Arc<dyn NodeReferenceLister> {
    let mut s = StaticNodeRefs::default();
    s.set(node, refs);
    Arc::new(s)
}

#[test]
fn nr_allows_non_node_users() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = req("alice", "Node", "", "n1", Operation::Update);
    assert!(nr.validate(&r).allowed);
}

#[test]
fn nr_node_can_update_self() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = req("system:node:n1", "Node", "", "n1", Operation::Update);
    assert!(nr.validate(&r).allowed);
}

#[test]
fn nr_node_cannot_update_other_node() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = req("system:node:n1", "Node", "", "n2", Operation::Update);
    assert!(!nr.validate(&r).allowed);
}

#[test]
fn nr_node_cannot_delete_other_node() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = req("system:node:n1", "Node", "", "n2", Operation::Delete);
    assert!(!nr.validate(&r).allowed);
}

#[test]
fn nr_pod_must_be_owned() {
    let mut refs = OwnPodReferences::default();
    refs.pods.insert(("default".into(), "p1".into()));
    let nr = NodeRestriction::new(lister_for("n1", refs));
    let allowed = nr.validate(&req(
        "system:node:n1",
        "Pod",
        "default",
        "p1",
        Operation::Update,
    ));
    assert!(allowed.allowed);
    let denied = nr.validate(&req(
        "system:node:n1",
        "Pod",
        "default",
        "other",
        Operation::Update,
    ));
    assert!(!denied.allowed);
}

#[test]
fn nr_secret_mutation_is_denied() {
    let mut refs = OwnPodReferences::default();
    refs.secrets.insert(("default".into(), "s".into()));
    let nr = NodeRestriction::new(lister_for("n1", refs));
    let r = req(
        "system:node:n1",
        "Secret",
        "default",
        "s",
        Operation::Create,
    );
    assert!(!nr.validate(&r).allowed);
}

#[test]
fn nr_secret_read_must_be_referenced() {
    let mut refs = OwnPodReferences::default();
    refs.secrets.insert(("default".into(), "s1".into()));
    let nr = NodeRestriction::new(lister_for("n1", refs));
    let allowed = nr.validate(&req(
        "system:node:n1",
        "Secret",
        "default",
        "s1",
        Operation::Connect,
    ));
    assert!(allowed.allowed);
    let denied = nr.validate(&req(
        "system:node:n1",
        "Secret",
        "default",
        "other",
        Operation::Connect,
    ));
    assert!(!denied.allowed);
}

#[test]
fn nr_configmap_read_must_be_referenced() {
    let mut refs = OwnPodReferences::default();
    refs.configmaps.insert(("default".into(), "cm1".into()));
    let nr = NodeRestriction::new(lister_for("n1", refs));
    let denied = nr.validate(&req(
        "system:node:n1",
        "ConfigMap",
        "default",
        "cm-other",
        Operation::Connect,
    ));
    assert!(!denied.allowed);
}

#[test]
fn nr_lease_only_in_kube_node_lease_namespace() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let bad_ns = nr.validate(&req(
        "system:node:n1",
        "Lease",
        "default",
        "n1",
        Operation::Update,
    ));
    assert!(!bad_ns.allowed);
    let good = nr.validate(&req(
        "system:node:n1",
        "Lease",
        "kube-node-lease",
        "n1",
        Operation::Update,
    ));
    assert!(good.allowed);
}

#[test]
fn nr_lease_must_be_own_name() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = nr.validate(&req(
        "system:node:n1",
        "Lease",
        "kube-node-lease",
        "n2",
        Operation::Update,
    ));
    assert!(!r.allowed);
}

#[test]
fn nr_csinode_must_be_own_name() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let bad = nr.validate(&req(
        "system:node:n1",
        "CSINode",
        "",
        "n2",
        Operation::Update,
    ));
    assert!(!bad.allowed);
    let ok = nr.validate(&req(
        "system:node:n1",
        "CSINode",
        "",
        "n1",
        Operation::Update,
    ));
    assert!(ok.allowed);
}

#[test]
fn nr_unknown_kind_is_allowed() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = nr.validate(&req(
        "system:node:n1",
        "Whatever",
        "ns",
        "x",
        Operation::Create,
    ));
    assert!(r.allowed);
}

#[test]
fn nr_node_name_extraction() {
    assert_eq!(node_name_from_user("system:node:foo"), Some("foo"));
    assert_eq!(node_name_from_user("alice"), None);
    assert_eq!(node_name_from_user("system:node:"), Some(""));
}

#[test]
fn nr_mutating_phase_mirrors_validating() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let mut r = req("system:node:n1", "Node", "", "n2", Operation::Update);
    let resp = MutatingWebhook::admit(&nr, &mut r);
    assert!(!resp.allowed);
}

#[test]
fn nr_pod_create_for_unowned_pod_denied() {
    let nr = NodeRestriction::new(Arc::new(StaticNodeRefs::default()));
    let r = req("system:node:n1", "Pod", "default", "p", Operation::Create);
    assert!(!nr.validate(&r).allowed);
}

#[test]
#[cfg(feature = "live-integration")]
fn nr_pod_status_only_allowed_on_own_pods() {
    // pending: requires subresource (status) modelling on AdmissionRequest
}

#[test]
#[cfg(feature = "live-integration")]
fn nr_pod_eviction_subresource() {
    // pending: requires subresource (eviction) modelling — kubelet evict-self
}
