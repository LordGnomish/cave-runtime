// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! deeper-001: HCL Policy — comments, denied/required/allowed parameters,
//! sudo on root-protected paths, min_wrapping_ttl, multi-statement bodies.
//! Pinned to openbao v2.5.3.

use cave_vault::core::policy::{Capability, Policy, PolicyRule, PolicyStore};

const TENANT: &str = "tenant-acme-prod";

fn tenant_path(suffix: &str) -> String {
    format!("{}/secret/{}", TENANT, suffix)
}

/// Cite: openbao `vault/policy.go:253` (ParseACLPolicy) — `#` and `//`
/// line comments inside HCL must be stripped before regex extraction so
/// reviewer-friendly policies parse cleanly.
#[test]
fn hcl_comments_are_stripped_before_parsing() {
    let hcl = format!(r#"
        # tenant_id: {tenant}
        // ops policy — covers the secret/* path
        path "{path}/*" {{
            // allow read+list, no writes
            capabilities = ["read", "list"]   # tightened 2026-04
        }}
    "#, tenant = TENANT, path = tenant_path(""));
    let p = Policy::parse("ops", &hcl).unwrap();
    assert_eq!(p.rules.len(), 1);
    assert_eq!(p.rules[0].path, format!("{}/*", tenant_path("")));
    assert!(p.rules[0].capabilities.contains(&Capability::Read));
    assert!(p.rules[0].capabilities.contains(&Capability::List));
    assert!(!p.rules[0].capabilities.contains(&Capability::Create));
}

/// Cite: openbao `vault/policy.go:140` (`DeniedParametersHCL`) — denied
/// parameters take precedence over allowed_parameters; even if a key is
/// in `allowed`, presence in `denied` blocks the request.
#[test]
fn denied_parameters_block_even_when_allowed_lists_them() {
    let hcl = format!(r#"
        path "{path}" {{
            capabilities = ["create", "update"]
            allowed_parameters = {{
                "*" = []
            }}
            denied_parameters = {{
                "secret_token" = []
                "private_key" = []
            }}
        }}
    "#, path = tenant_path("creds"));
    let p = Policy::parse("api", &hcl).unwrap();
    assert_eq!(p.rules[0].denied_parameters, vec!["secret_token", "private_key"]);
    assert_eq!(p.rules[0].allowed_parameters, vec!["*"]);

    let r = &p.rules[0];
    assert!(r.check_parameters(&["public_id", "name"]).is_ok());
    let err = r.check_parameters(&["public_id", "secret_token"]).unwrap_err();
    assert!(err.contains("'secret_token' is denied"));
}

/// Cite: openbao `vault/policy.go:141` (`RequiredParametersHCL`) — every
/// listed key must appear in the request body, otherwise the operation
/// is rejected with `missing required parameter`.
#[test]
fn required_parameters_must_all_be_present() {
    let hcl = format!(r#"
        path "{path}" {{
            capabilities = ["create"]
            required_parameters = ["customer_id", "billing_account"]
        }}
    "#, path = tenant_path("invoices"));
    let p = Policy::parse("billing", &hcl).unwrap();
    assert_eq!(p.rules[0].required_parameters, vec!["customer_id", "billing_account"]);

    let r = &p.rules[0];
    assert!(r.check_parameters(&["customer_id", "billing_account", "amount"]).is_ok());
    let err = r.check_parameters(&["customer_id"]).unwrap_err();
    assert!(err.contains("missing required parameter: billing_account"));
}

/// Cite: openbao `vault/policy.go:32` (`SudoCapability`) — sudo is the
/// canonical capability for unlocking root-protected paths (e.g. `sys/raw`,
/// `sys/seal`). The default `default` policy never grants it; root does.
/// A custom policy with `sudo` grants access only on its matched paths.
#[test]
fn sudo_capability_unlocks_root_protected_paths() {
    let hcl = r#"
        path "sys/seal" {
            capabilities = ["sudo", "update"]
        }
        path "sys/step-down" {
            capabilities = ["sudo", "update"]
        }
    "#;
    let p = Policy::parse("ops-sudo", hcl).unwrap();
    assert!(p.allows("sys/seal", &Capability::Sudo));
    assert!(p.allows("sys/seal", &Capability::Update));
    assert!(p.allows("sys/step-down", &Capability::Sudo));
    // Other root-protected paths NOT covered ⇒ no access.
    assert!(!p.allows("sys/raw/cubbyhole", &Capability::Sudo));

    // Default policy never grants sudo on arbitrary paths.
    let store = PolicyStore::new();
    assert!(!store.check(&["default".into()], "sys/seal", &Capability::Sudo));
    // Root grants sudo everywhere.
    assert!(store.check(&["root".into()], "sys/seal", &Capability::Sudo));
}

/// Cite: openbao `vault/policy.go:137` (`MinWrappingTTLHCL`) +
/// `vault/policy.go::ParseACLPolicy` numeric-or-duration parsing —
/// accepts bare integers (seconds), `Ns`, `Nm`, `Nh`, `Nd` suffixes.
#[test]
fn min_wrapping_ttl_parses_bare_seconds_and_h_suffix() {
    let hcl_seconds = format!(r#"
        path "{path}" {{
            capabilities = ["read"]
            min_wrapping_ttl = "3600"
        }}
    "#, path = tenant_path("a"));
    let p = Policy::parse("a", &hcl_seconds).unwrap();
    assert_eq!(p.rules[0].min_wrapping_ttl_seconds, 3600);

    let hcl_hours = format!(r#"
        path "{path}" {{
            capabilities = ["read"]
            min_wrapping_ttl = "2h"
        }}
    "#, path = tenant_path("b"));
    let p = Policy::parse("b", &hcl_hours).unwrap();
    assert_eq!(p.rules[0].min_wrapping_ttl_seconds, 2 * 3600);
}

/// Cite: openbao `vault/policy.go::parsePaths` glob precedence — when
/// both a `*` (prefix) and a `+` (single segment) rule could match, the
/// longest literal prefix wins. cave's `Policy::allows` mirrors that.
#[test]
fn glob_star_vs_plus_precedence_resolves_via_longest_prefix() {
    let p = Policy {
        name: "tiered".into(),
        rules: vec![
            PolicyRule {
                path: "secret/+".into(),
                capabilities: vec![Capability::Read],
                ..Default::default()
            },
            PolicyRule {
                path: "secret/admin/*".into(),
                capabilities: vec![Capability::Read, Capability::Update, Capability::Delete],
                ..Default::default()
            },
        ],
        raw: String::new(),
    };
    assert!(p.allows("secret/admin/key", &Capability::Update),
        "deeper rule wins for matching admin path");
    assert!(p.allows("secret/foo", &Capability::Read),
        "+ rule still matches single-segment paths");
    assert!(!p.allows("secret/foo", &Capability::Update),
        "+ rule does NOT grant update");
}

/// Cite: openbao `vault/policy.go::parsePaths` multi-statement body —
/// `capabilities`, `required_parameters`, `denied_parameters` and
/// `min_wrapping_ttl` may all coexist inside a single `path { … }` block.
#[test]
fn multiple_statements_inside_one_path_block() {
    let hcl = format!(r#"
        path "{path}" {{
            capabilities       = ["create", "update"]
            required_parameters = ["request_id"]
            denied_parameters  = {{
                "raw_secret" = []
            }}
            min_wrapping_ttl   = "30m"
        }}
    "#, path = tenant_path("multi"));
    let p = Policy::parse("multi", &hcl).unwrap();
    let r = &p.rules[0];
    assert!(r.capabilities.contains(&Capability::Create));
    assert!(r.capabilities.contains(&Capability::Update));
    assert_eq!(r.required_parameters, vec!["request_id"]);
    assert_eq!(r.denied_parameters, vec!["raw_secret"]);
    assert_eq!(r.min_wrapping_ttl_seconds, 30 * 60);

    // End-to-end: request body with required key + an extra is OK.
    assert!(r.check_parameters(&["request_id", "comment"]).is_ok());
    // Missing required ⇒ rejected.
    assert!(r.check_parameters(&["comment"]).is_err());
    // Includes denied ⇒ rejected even though required is present.
    assert!(r.check_parameters(&["request_id", "raw_secret"]).is_err());
}
