// SPDX-License-Identifier: AGPL-3.0-or-later
//
// UMA 2.0 — User-Managed Access.
//
// Standards:
//   - UMA 2.0 Grant for OAuth 2.0 Authorization
//     <https://docs.kantarainitiative.org/uma/wg/rec-oauth-uma-grant-2.0.html>
//   - UMA 2.0 Federated Authorization
//     <https://docs.kantarainitiative.org/uma/wg/rec-oauth-uma-federated-authz-2.0.html>
//
// Upstream parity:
//   - keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38  (v22.0.0)
//     services/src/main/java/org/keycloak/authorization/protection/resource/
//     services/src/main/java/org/keycloak/authorization/protection/permission/
//     services/src/main/java/org/keycloak/protocol/oidc/grants/UmaTicketGrantType.java
//     services/src/main/java/org/keycloak/authorization/policy/evaluation/
//
// Sub-modules:
//   - [`resource`]    — Resource registration (UMA-FedAuthz §2)
//   - [`permission`]  — Permission ticket endpoint (UMA-Grant §3.2)
//   - [`rpt`]         — Requesting Party Token issuance (UMA-Grant §3.3)
//   - [`claim_token`] — Claim-token + claim-pushing
//   - [`policy`]      — Scope / role / time policy evaluation
//
// Out-of-scope (Phase 2):
//   - JavaScript policies (Keycloak's Nashorn engine)
//   - Per-tenant authorization client policies (`AuthzClient`)

pub mod claim_token;
pub mod permission;
pub mod policy;
pub mod resource;
pub mod rpt;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum UmaError {
    #[error("not_found")]
    NotFound,
    #[error("forbidden")]
    Forbidden,
    #[error("invalid_request: {0}")]
    InvalidRequest(&'static str),
    #[error("invalid_grant")]
    InvalidGrant,
    #[error("policy_denied")]
    PolicyDenied,
    #[error("invalid_token")]
    InvalidToken,
}
