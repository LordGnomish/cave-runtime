// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak v22.0.0 close-out — Phase 3 cavectl wiring
//
// FINALIZE smoke: each cave-auth Phase 3 protocol must expose its `cavectl
// auth <proto>` PATH constants behind a shared `/api/auth/<proto>/` prefix so
// the binary's dispatch table stays a single match arm per variant.

use cavectl::auth::{
    admin_flows, admin_idp, dpop, email_listener, jwe, oauth_endpoints, oid4vc, persistence,
    token_exchange, uma, wsfed,
};

fn prefix_invariant(prefix: &str, paths: &[&str]) {
    for p in paths {
        assert!(
            p.starts_with(prefix),
            "{p} must share prefix {prefix} so the dispatch arm stays one-liner"
        );
    }
}

#[test]
fn oauth_endpoints_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/oauth/",
        &[
            oauth_endpoints::PATH_PAR,
            oauth_endpoints::PATH_DEVICE,
            oauth_endpoints::PATH_CIBA,
            oauth_endpoints::PATH_REVOKE,
        ],
    );
}

#[test]
fn wsfed_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/wsfed/",
        &[wsfed::PATH_METADATA, wsfed::PATH_SIGNIN, wsfed::PATH_SIGNOUT],
    );
}

#[test]
fn oid4vc_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/oid4vc/",
        &[
            oid4vc::PATH_ISSUE,
            oid4vc::PATH_CREDENTIAL,
            oid4vc::PATH_PRESENT,
            oid4vc::PATH_METADATA,
        ],
    );
}

#[test]
fn uma_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/uma/",
        &[
            uma::PATH_RESOURCE_SET,
            uma::PATH_PERMISSION_TICKET,
            uma::PATH_RPT,
        ],
    );
}

#[test]
fn token_exchange_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/token-exchange/",
        &[token_exchange::PATH_EXCHANGE],
    );
}

#[test]
fn dpop_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/dpop/",
        &[dpop::PATH_VERIFY_PROOF, dpop::PATH_THUMBPRINT],
    );
}

#[test]
fn jwe_paths_share_prefix() {
    prefix_invariant("/api/auth/jwe/", &[jwe::PATH_ENCRYPT, jwe::PATH_DECRYPT]);
}

#[test]
fn admin_idp_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/admin/identity-provider/",
        &[admin_idp::PATH_INSTANCES, admin_idp::PATH_MAPPERS],
    );
}

#[test]
fn admin_flows_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/admin/authentication/",
        &[
            admin_flows::PATH_FLOWS,
            admin_flows::PATH_EXECUTIONS,
            admin_flows::PATH_REQUIRED_ACTIONS,
        ],
    );
}

#[test]
fn email_listener_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/email/",
        &[email_listener::PATH_QUEUE, email_listener::PATH_TEST_SEND],
    );
}

#[test]
fn persistence_paths_share_prefix() {
    prefix_invariant(
        "/api/auth/persistence/",
        &[persistence::PATH_STATUS, persistence::PATH_MIGRATE],
    );
}

#[test]
fn every_phase3_module_uses_distinct_prefix() {
    // Coupled tests: prevent accidental shared prefix between protocols.
    let prefixes = [
        "/api/auth/oauth/",
        "/api/auth/wsfed/",
        "/api/auth/oid4vc/",
        "/api/auth/uma/",
        "/api/auth/token-exchange/",
        "/api/auth/dpop/",
        "/api/auth/jwe/",
        "/api/auth/admin/identity-provider/",
        "/api/auth/admin/authentication/",
        "/api/auth/email/",
        "/api/auth/persistence/",
    ];
    for (i, a) in prefixes.iter().enumerate() {
        for b in prefixes.iter().skip(i + 1) {
            assert_ne!(a, b, "prefixes must be unique");
        }
    }
}
