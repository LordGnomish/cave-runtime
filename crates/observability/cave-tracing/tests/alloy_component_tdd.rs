// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD port of grafana/alloy `internal/component/registry.go` (v1.5.0).
//!
//! Exercises component-name parsing/validation, prefix-collision detection,
//! the registration rules (duplicate / stability invariants), lookup, and
//! the feature-gate stability ordering.

use cave_tracing::alloy::component::{
    parse_component_name, AllowAtStability, Registration, Registry, Stability,
};

fn reg(name: &str, stability: Stability, community: bool) -> Registration {
    Registration { name: name.to_string(), stability, community }
}

#[test]
fn parse_component_name_splits_and_validates() {
    assert_eq!(parse_component_name("remote.http").unwrap(), vec!["remote", "http"]);
    assert_eq!(parse_component_name("prometheus.scrape").unwrap(), vec!["prometheus", "scrape"]);
    assert_eq!(parse_component_name("local").unwrap(), vec!["local"]);

    // empty identifier
    assert!(parse_component_name("remote.").is_err());
    assert!(parse_component_name(".http").is_err());
    // invalid leading character
    assert!(parse_component_name("1remote.http").is_err());
    assert!(parse_component_name("remote.0http").is_err());
}

#[test]
fn register_two_components_sharing_a_prefix() {
    let mut r = Registry::new();
    r.register(reg("remote.http", Stability::GenerallyAvailable, false)).unwrap();
    // Same prefix length, distinct leaf — allowed.
    r.register(reg("remote.s3", Stability::GenerallyAvailable, false)).unwrap();
    assert!(r.get("remote.http").is_some());
    assert!(r.get("remote.s3").is_some());
    assert!(r.get("remote.gcs").is_none());
}

#[test]
fn prefix_collision_is_rejected() {
    let mut r = Registry::new();
    r.register(reg("remote.http", Stability::GenerallyAvailable, false)).unwrap();
    // "remote" is solely a prefix of "remote.http" — ambiguous.
    let err = r.register(reg("remote", Stability::GenerallyAvailable, false)).unwrap_err();
    assert!(err.contains("remote"), "err was: {err}");
}

#[test]
fn duplicate_registration_is_rejected() {
    let mut r = Registry::new();
    r.register(reg("loki.source.file", Stability::GenerallyAvailable, false)).unwrap();
    let err = r.register(reg("loki.source.file", Stability::Experimental, false)).unwrap_err();
    assert!(err.to_lowercase().contains("already"), "err was: {err}");
}

#[test]
fn stability_invariants_on_registration() {
    let mut r = Registry::new();
    // Non-community component with undefined stability → rejected.
    assert!(r.register(reg("foo.bar", Stability::Undefined, false)).is_err());
    // Community component with a defined stability → rejected.
    assert!(r.register(reg("foo.baz", Stability::GenerallyAvailable, true)).is_err());
    // Community component with undefined stability → allowed.
    assert!(r.register(reg("foo.qux", Stability::Undefined, true)).is_ok());
}

#[test]
fn allow_at_stability_ordering() {
    // A GA component is allowed at any minimum; experimental is only allowed
    // when the minimum permits experimental features.
    assert!(AllowAtStability(Stability::GenerallyAvailable, Stability::Experimental).unwrap());
    assert!(AllowAtStability(Stability::Experimental, Stability::Experimental).unwrap());
    assert!(!AllowAtStability(Stability::Experimental, Stability::GenerallyAvailable).unwrap());
    assert!(AllowAtStability(Stability::PublicPreview, Stability::PublicPreview).unwrap());
    // Undefined on either side is an error.
    assert!(AllowAtStability(Stability::Undefined, Stability::Experimental).is_err());
    assert!(AllowAtStability(Stability::Experimental, Stability::Undefined).is_err());
}
