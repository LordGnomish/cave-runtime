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
}
