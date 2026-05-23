// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! `cavectl cert …` URL builders.
//!
//! Pure functions — no I/O. cavectl drives these through its own
//! [`ApiClient`](https://docs.rs/cavectl/) and reports back. Keeping
//! the URL builders here lets the runtime tests assert the cert-manager
//! surface stays in sync with the routes module.

/// Top-level health endpoint.
pub fn health_path() -> &'static str {
    "/api/cert/health"
}

/// `cavectl cert issuer list <tenant>` — cluster-issuer count surface.
pub fn cluster_issuers_path(tenant: &str) -> String {
    format!("/api/cert/{}/cluster-issuers", tenant)
}

/// `cavectl cert issuer create-namespaced <tenant>` — POST surface for
/// a namespaced Issuer (the routes module wires this to
/// `create_issuer`).
pub fn issuers_path(tenant: &str) -> String {
    format!("/api/cert/{}/issuers", tenant)
}

/// `cavectl cert cert list <tenant>` — Certificate list.
pub fn certificates_path(tenant: &str) -> String {
    format!("/api/cert/{}/certificates", tenant)
}

/// `cavectl cert cert get <tenant> <id>`.
pub fn certificate_get_path(tenant: &str, id: &str) -> String {
    format!("/api/cert/{}/certificates/{}", tenant, id)
}

/// `cavectl cert cert issue <tenant> <id>`.
pub fn certificate_issue_path(tenant: &str, id: &str) -> String {
    format!("/api/cert/{}/certificates/{}/issue", tenant, id)
}

/// `cavectl cert renew <tenant> <id>`.
pub fn certificate_renew_path(tenant: &str, id: &str) -> String {
    format!("/api/cert/{}/certificates/{}/renew", tenant, id)
}

/// `cavectl cert request <tenant>` — CertificateRequest list.
pub fn requests_path(tenant: &str) -> String {
    format!("/api/cert/{}/certificate-requests", tenant)
}

/// `cavectl cert verify <tenant> <id>` — re-check a materialised
/// Certificate against the issuer (chain + notAfter + revocation
/// ledger). Returns the verification report; the route is GET so a
/// scheduled poller can drive it without state mutation.
pub fn certificate_verify_path(tenant: &str, id: &str) -> String {
    format!("/api/cert/{}/certificates/{}/verify", tenant, id)
}

/// `cavectl cert revoke <tenant> <id>` — POST a revocation against
/// the RevocationLedger.
pub fn certificate_revoke_path(tenant: &str, id: &str) -> String {
    format!("/api/cert/{}/certificates/{}/revoke", tenant, id)
}

/// `cavectl cert metrics` — Prometheus exposition surface. Mounted
/// at the conventional `/metrics` path so any scraper config can
/// reach it without extra wiring.
pub fn metrics_path() -> &'static str {
    "/metrics"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_is_constant() {
        assert_eq!(health_path(), "/api/cert/health");
    }

    #[test]
    fn cluster_issuers_path_includes_tenant() {
        assert_eq!(cluster_issuers_path("t-1"), "/api/cert/t-1/cluster-issuers");
    }

    #[test]
    fn issuers_path_includes_tenant() {
        assert_eq!(issuers_path("t-1"), "/api/cert/t-1/issuers");
    }

    #[test]
    fn certificate_get_path_round_trips_id() {
        let id = "11111111-1111-1111-1111-111111111111";
        assert_eq!(
            certificate_get_path("t-1", id),
            "/api/cert/t-1/certificates/11111111-1111-1111-1111-111111111111"
        );
    }

    #[test]
    fn issue_and_renew_paths_distinct() {
        let id = "abc";
        assert!(certificate_issue_path("t-1", id).ends_with("/issue"));
        assert!(certificate_renew_path("t-1", id).ends_with("/renew"));
        assert_ne!(
            certificate_issue_path("t-1", id),
            certificate_renew_path("t-1", id)
        );
    }

    #[test]
    fn certificates_list_path_pluralised() {
        assert_eq!(certificates_path("t-1"), "/api/cert/t-1/certificates");
    }

    #[test]
    fn requests_path_pluralised() {
        assert_eq!(requests_path("t-1"), "/api/cert/t-1/certificate-requests");
    }

    #[test]
    fn verify_path_ends_in_verify_suffix() {
        let id = "abc";
        assert!(certificate_verify_path("t-1", id).ends_with("/verify"));
        assert_eq!(
            certificate_verify_path("t-1", id),
            "/api/cert/t-1/certificates/abc/verify"
        );
    }

    #[test]
    fn revoke_path_distinct_from_renew_and_verify() {
        let id = "abc";
        let revoke = certificate_revoke_path("t-1", id);
        let renew = certificate_renew_path("t-1", id);
        let verify = certificate_verify_path("t-1", id);
        assert_ne!(revoke, renew);
        assert_ne!(revoke, verify);
        assert!(revoke.ends_with("/revoke"));
    }

    #[test]
    fn metrics_path_is_constant() {
        assert_eq!(metrics_path(), "/metrics");
    }

    #[test]
    fn all_builders_share_api_cert_prefix() {
        let tenant = "tenant-x";
        let id = "00000000-0000-0000-0000-000000000000";
        for p in [
            cluster_issuers_path(tenant),
            issuers_path(tenant),
            certificates_path(tenant),
            certificate_get_path(tenant, id),
            certificate_issue_path(tenant, id),
            certificate_renew_path(tenant, id),
            certificate_verify_path(tenant, id),
            certificate_revoke_path(tenant, id),
            requests_path(tenant),
        ] {
            assert!(
                p.starts_with("/api/cert/"),
                "all per-tenant cert routes must share the /api/cert/ prefix; got {p}"
            );
        }
    }

    #[test]
    fn tenant_value_does_not_leak_into_globals() {
        // health + metrics are tenant-independent globals.
        assert!(!health_path().contains("tenant"));
        assert!(!metrics_path().contains("tenant"));
    }
}
