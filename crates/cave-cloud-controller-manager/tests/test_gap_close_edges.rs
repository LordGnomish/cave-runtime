// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge coverage for cave-cloud-controller-manager — types, provider config,
//! NodeAddressType precedence, InstanceState classification, ClusterId.

use cave_cloud_controller_manager::node_controller::{
    InstanceState, NodeAddress, NodeAddressType,
};
use cave_cloud_controller_manager::provider::{ClusterId, CloudConfig, InstanceMetadata, ZoneInfo};
use cave_cloud_controller_manager::types::{
    Cite, CloudError, ProviderName, Reconcile, TenantId, UPSTREAM_VERSION,
};

fn tenant() -> TenantId {
    TenantId::new("acme").unwrap()
}

// ---------------------------------------------------------------------------
// Cite
// ---------------------------------------------------------------------------

#[test]
fn cite_k8s_uses_pinned_upstream_version() {
    let c = Cite::k8s("pkg/x.go", "Foo");
    assert_eq!(c.repo, "kubernetes/kubernetes");
    assert_eq!(c.version, UPSTREAM_VERSION);
}

#[test]
fn cite_ext_carries_provider_repo_and_version() {
    let c = Cite::ext("hetznercloud/hcloud-cloud-controller-manager", "x.go", "Sym", "v1.30.1");
    assert_eq!(c.repo, "hetznercloud/hcloud-cloud-controller-manager");
    assert_eq!(c.version, "v1.30.1");
    let url = c.url();
    assert!(url.contains("hetznercloud"));
    assert!(url.contains("v1.30.1"));
}

#[test]
fn cite_display_includes_repo_and_version() {
    let c = Cite::k8s("a/b.go", "Sym");
    let s = format!("{}", c);
    assert!(s.contains("kubernetes/kubernetes"));
    assert!(s.contains(UPSTREAM_VERSION));
}

// ---------------------------------------------------------------------------
// ProviderName
// ---------------------------------------------------------------------------

#[test]
fn provider_name_uri_schemes() {
    assert_eq!(ProviderName::Hetzner.provider_id_scheme(), "hcloud");
    assert_eq!(ProviderName::Azure.provider_id_scheme(), "azure");
}

#[test]
fn provider_name_display_lowercase() {
    assert_eq!(format!("{}", ProviderName::Hetzner), "hetzner");
    assert_eq!(format!("{}", ProviderName::Azure), "azure");
}

#[test]
fn provider_name_serde_round_trip() {
    let j = serde_json::to_string(&ProviderName::Hetzner).unwrap();
    let back: ProviderName = serde_json::from_str(&j).unwrap();
    assert_eq!(back, ProviderName::Hetzner);
}

// ---------------------------------------------------------------------------
// CloudConfig::validate
// ---------------------------------------------------------------------------

fn cfg(region: &str, cred: &str) -> CloudConfig {
    CloudConfig {
        tenant: tenant(),
        provider: ProviderName::Hetzner,
        region: region.into(),
        credential_ref: cred.into(),
    }
}

#[test]
fn cloud_config_validate_empty_region_errors() {
    assert!(cfg("", "vault://x").validate().is_err());
    assert!(cfg("   ", "vault://x").validate().is_err());
}

#[test]
fn cloud_config_validate_requires_vault_or_secret_ref() {
    assert!(cfg("eu", "plain-text").validate().is_err());
    assert!(cfg("eu", "vault://x").validate().is_ok());
    assert!(cfg("eu", "secret://y").validate().is_ok());
}

#[test]
fn cloud_config_validate_rejects_empty_credential_ref() {
    assert!(cfg("eu", "").validate().is_err());
}

// ---------------------------------------------------------------------------
// Reconcile + CloudError
// ---------------------------------------------------------------------------

#[test]
fn reconcile_variants_serde_round_trip() {
    for r in [
        Reconcile::NoOp,
        Reconcile::Annotate(2),
        Reconcile::Untaint(1),
        Reconcile::AllocateIp(3),
        Reconcile::Update(4),
        Reconcile::Delete(5),
        Reconcile::Requeue,
    ] {
        let j = serde_json::to_string(&r).unwrap();
        let back: Reconcile = serde_json::from_str(&j).unwrap();
        assert_eq!(r, back);
    }
}

#[test]
fn cloud_error_invalid_config_display_includes_provider() {
    let e = CloudError::InvalidConfig {
        provider: ProviderName::Hetzner,
        reason: "missing token".into(),
    };
    let s = e.to_string();
    assert!(s.contains("hetzner"));
    assert!(s.contains("missing token"));
}

#[test]
fn cloud_error_upstream_display() {
    let e = CloudError::Upstream {
        provider: ProviderName::Azure,
        reason: "503".into(),
    };
    let s = e.to_string();
    assert!(s.contains("azure"));
    assert!(s.contains("503"));
}

#[test]
fn cloud_error_tenant_denied_display() {
    let e = CloudError::TenantDenied { tenant: tenant(), kind: "Node", name: "n1".into() };
    let s = e.to_string();
    assert!(s.contains("acme"));
    assert!(s.contains("Node"));
    assert!(s.contains("n1"));
}

// ---------------------------------------------------------------------------
// NodeAddressType precedence
// ---------------------------------------------------------------------------

#[test]
fn node_address_type_precedence_is_canonical_order() {
    assert!(NodeAddressType::InternalIP.precedence() < NodeAddressType::ExternalIP.precedence());
    assert!(NodeAddressType::ExternalIP.precedence() < NodeAddressType::InternalDNS.precedence());
    assert!(NodeAddressType::InternalDNS.precedence() < NodeAddressType::ExternalDNS.precedence());
    assert!(NodeAddressType::ExternalDNS.precedence() < NodeAddressType::Hostname.precedence());
}

#[test]
fn node_address_type_key_strings() {
    assert_eq!(NodeAddressType::InternalIP.key(), "InternalIP");
    assert_eq!(NodeAddressType::ExternalIP.key(), "ExternalIP");
    assert_eq!(NodeAddressType::Hostname.key(), "Hostname");
}

#[test]
fn node_address_new_carries_kind_and_address() {
    let a = NodeAddress::new(NodeAddressType::InternalIP, "10.0.0.1");
    assert_eq!(a.kind, NodeAddressType::InternalIP);
    assert_eq!(a.address, "10.0.0.1");
}

// ---------------------------------------------------------------------------
// InstanceState classification
// ---------------------------------------------------------------------------

#[test]
fn instance_state_requires_deletion_for_terminated_and_notfound() {
    assert!(InstanceState::Terminated.requires_deletion());
    assert!(InstanceState::NotFound.requires_deletion());
    assert!(!InstanceState::Running.requires_deletion());
    assert!(!InstanceState::Shutdown.requires_deletion());
    assert!(!InstanceState::Unreachable.requires_deletion());
}

#[test]
fn instance_state_healthy_only_for_running() {
    assert!(InstanceState::Running.is_healthy());
    for s in [
        InstanceState::Shutdown,
        InstanceState::Terminated,
        InstanceState::NotFound,
        InstanceState::Unreachable,
    ] {
        assert!(!s.is_healthy(), "{:?} should not be healthy", s);
    }
}

#[test]
fn instance_state_failure_taint_for_shutdown_and_unreachable() {
    assert!(InstanceState::Shutdown.failure_taint().is_some());
    assert!(InstanceState::Unreachable.failure_taint().is_some());
    for s in [InstanceState::Running, InstanceState::Terminated, InstanceState::NotFound] {
        assert!(s.failure_taint().is_none(), "{:?} should not taint", s);
    }
}

// ---------------------------------------------------------------------------
// InstanceMetadata validate
// ---------------------------------------------------------------------------

#[test]
fn instance_metadata_not_found_skips_other_checks() {
    let mut m = InstanceMetadata::new("", "", "", "", vec![]);
    m.not_found = true;
    assert!(m.validate().is_ok());
}

#[test]
fn instance_metadata_validate_requires_provider_id() {
    let m = InstanceMetadata::new("", "cx21", "eu", "fsn1", vec![]);
    assert!(m.validate().is_err());
}

#[test]
fn instance_metadata_validate_requires_instance_type() {
    let m = InstanceMetadata::new("hcloud://1", "", "eu", "fsn1", vec![]);
    assert!(m.validate().is_err());
}

#[test]
fn instance_metadata_validate_requires_region_and_zone() {
    let m_no_region = InstanceMetadata::new("hcloud://1", "cx21", "", "fsn1", vec![]);
    assert!(m_no_region.validate().is_err());
    let m_no_zone = InstanceMetadata::new("hcloud://1", "cx21", "eu", "", vec![]);
    assert!(m_no_zone.validate().is_err());
}

#[test]
fn instance_metadata_validate_ok_when_complete() {
    let m = InstanceMetadata::new("hcloud://1", "cx21", "eu", "fsn1", vec![]);
    assert!(m.validate().is_ok());
}

// ---------------------------------------------------------------------------
// ZoneInfo + ClusterId
// ---------------------------------------------------------------------------

#[test]
fn zone_info_new_carries_region_and_failure_domain() {
    let z = ZoneInfo::new("eu", "fsn1-dc1");
    assert_eq!(z.region, "eu");
    assert_eq!(z.failure_domain, "fsn1-dc1");
}

#[test]
fn cluster_id_new_and_as_str() {
    let id = ClusterId::new("cluster-7");
    assert_eq!(id.as_str(), "cluster-7");
    assert_eq!(id.0, "cluster-7");
}
