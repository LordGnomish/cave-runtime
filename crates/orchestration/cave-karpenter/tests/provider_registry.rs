// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-cloud provider abstraction — faithful to
//! kubernetes-sigs/karpenter v1.4.0 `pkg/cloudprovider/types.go`
//! (CloudProvider interface + GetSupportedNodeClasses group/kind dispatch).
//!
//! The provider registry maps a NodeClass envelope (group + kind) to the
//! cloud provider that owns it, and dispatches create/delete through the
//! `CloudProvider` trait. AWS / GCP / Azure / Hetzner are registered at the
//! same level — no provider is privileged over another.

use cave_karpenter::models::NodeClass;
use cave_karpenter::provider::{
    AzureNodeClassSpec, Ec2NodeClassSpec, GceNodeClassSpec, HetznerNodeClassSpec,
    NodeClassKind, ProviderRegistration, ProviderRegistry, StaticProvider, default_registry,
};

fn node_class(group: &str, kind: &str) -> NodeClass {
    NodeClass {
        group: group.to_string(),
        kind: kind.to_string(),
        name: "default".to_string(),
        spec: serde_json::Value::Null,
    }
}

#[test]
fn default_registry_registers_four_equal_providers() {
    let r = default_registry();
    // Equal-level: providers are listed alphabetically with no privileged
    // first-party — see memory runtime_oss_no_hetzner_branding.
    assert_eq!(r.names(), vec!["aws", "azure", "gcp", "hetzner"]);
}

#[test]
fn dispatch_ec2_nodeclass_to_aws() {
    let r = default_registry();
    let nc = node_class(Ec2NodeClassSpec::GROUP, Ec2NodeClassSpec::KIND);
    let reg = r.for_node_class(&nc).expect("EC2NodeClass must dispatch");
    assert_eq!(reg.name, "aws");
}

#[test]
fn dispatch_gce_nodeclass_to_gcp() {
    let r = default_registry();
    let nc = node_class(GceNodeClassSpec::GROUP, GceNodeClassSpec::KIND);
    assert_eq!(r.for_node_class(&nc).unwrap().name, "gcp");
}

#[test]
fn dispatch_hetzner_and_azure() {
    let r = default_registry();
    let h = node_class(HetznerNodeClassSpec::GROUP, HetznerNodeClassSpec::KIND);
    let a = node_class(AzureNodeClassSpec::GROUP, AzureNodeClassSpec::KIND);
    assert_eq!(r.for_node_class(&h).unwrap().name, "hetzner");
    assert_eq!(r.for_node_class(&a).unwrap().name, "azure");
}

#[test]
fn unknown_nodeclass_returns_none() {
    let r = default_registry();
    let nc = node_class("example.com", "MysteryNodeClass");
    assert!(r.for_node_class(&nc).is_none());
}

#[test]
fn create_for_dispatches_then_delete_is_idempotent() {
    let r = default_registry();
    let nc = node_class(Ec2NodeClassSpec::GROUP, Ec2NodeClassSpec::KIND);
    let id = r.create_for(&nc, "m5.large", "us-east-1a").unwrap();
    assert!(r.exists_for(&nc, &id).unwrap());
    r.delete_for(&nc, &id).unwrap();
    assert!(!r.exists_for(&nc, &id).unwrap());
    // second delete is a no-op
    r.delete_for(&nc, &id).unwrap();
}

#[test]
fn create_for_unknown_nodeclass_errors() {
    let r = default_registry();
    let nc = node_class("example.com", "MysteryNodeClass");
    assert!(r.create_for(&nc, "x", "y").is_err());
}

#[test]
fn custom_registration_overrides_dispatch() {
    let mut r = ProviderRegistry::new();
    r.register(ProviderRegistration {
        name: "aws".into(),
        kind: NodeClassKind {
            group: Ec2NodeClassSpec::GROUP.into(),
            kind: Ec2NodeClassSpec::KIND.into(),
        },
        provider: Box::new(StaticProvider::new()),
    });
    assert_eq!(r.names(), vec!["aws"]);
    let nc = node_class(Ec2NodeClassSpec::GROUP, Ec2NodeClassSpec::KIND);
    assert!(r.for_node_class(&nc).is_some());
}

#[test]
fn ec2_and_gce_specs_serde_roundtrip() {
    let ec2 = Ec2NodeClassSpec {
        instance_profile: "KarpenterNodeRole".into(),
        ami_family: "AL2023".into(),
        subnet_selector_terms: vec!["subnet-abc".into()],
        security_group_selector_terms: vec!["sg-abc".into()],
        ..Default::default()
    };
    let j = serde_json::to_value(&ec2).unwrap();
    let back: Ec2NodeClassSpec = serde_json::from_value(j).unwrap();
    assert_eq!(back.ami_family, "AL2023");

    let gce = GceNodeClassSpec {
        machine_family: "n2".into(),
        image_family: "cos-stable".into(),
        region: "us-central1".into(),
        service_account: Some("karpenter@proj.iam".into()),
        ..Default::default()
    };
    let j = serde_json::to_value(&gce).unwrap();
    let back: GceNodeClassSpec = serde_json::from_value(j).unwrap();
    assert_eq!(back.machine_family, "n2");
}

#[test]
fn every_spec_exposes_its_node_class_kind() {
    // Each provider-specific spec declares the group/kind that the
    // registry keys on — mirrors GetSupportedNodeClasses() upstream.
    assert_eq!(Ec2NodeClassSpec::KIND, "EC2NodeClass");
    assert_eq!(AzureNodeClassSpec::KIND, "AKSNodeClass");
    assert_eq!(GceNodeClassSpec::KIND, "GCENodeClass");
    assert_eq!(HetznerNodeClassSpec::KIND, "HetznerNodeClass");
    // groups are distinct
    let groups = [
        Ec2NodeClassSpec::GROUP,
        AzureNodeClassSpec::GROUP,
        GceNodeClassSpec::GROUP,
        HetznerNodeClassSpec::GROUP,
    ];
    let mut uniq = groups.to_vec();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 4);
}
