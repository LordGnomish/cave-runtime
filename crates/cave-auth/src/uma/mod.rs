// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../authorization/ + Kantara UMA 2.0 Federated Authz spec
//
//! UMA 2.0 — User-Managed Access (Kantara, IETF draft-ietf-oauth-uma).
//!
//! Surfaced sub-modules:
//!
//! | Module | Spec section |
//! |--------|--------------|
//! | [`resource_set`] | Federated Authz §2 — resource registration |
//! | [`permission_ticket`] | Federated Authz §3 — permission tickets |
//! | [`rpt`]              | Grant §3 — Requesting Party Token |
//! | [`policy`]           | Federated Authz §3.2 — policy decision point |
//! | [`claim_token`]      | Grant §3.3 — pushed claims |

pub mod claim_token;
pub mod permission_ticket;
pub mod policy;
pub mod resource_set;
pub mod rpt;
