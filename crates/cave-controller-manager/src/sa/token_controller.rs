// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ServiceAccount token controller — `pkg/controller/serviceaccount/tokens_controller.go`.
//!
//! For each ServiceAccount, materialize a `kubernetes.io/service-account-token`
//! secret and (since v1.22) issue projected, audience-scoped tokens via the
//! TokenRequest API (`BoundServiceAccountTokens`).
//!
//! The simplified model here:
//!
//! * `Secret`s of type `kubernetes.io/service-account-token` are tied to a SA
//!   by `secret.metadata.annotations["kubernetes.io/service-account.uid"]`.
//! * If the annotation UID doesn't match the current SA UID → secret is
//!   stale and must be deleted (then a fresh one is created).
//! * `Secret.data["token"]` stores the JWT.
//! * Bound tokens carry `audiences[]` and an `expiration_sec` and are NOT
//!   stored as Secrets — they live only in projected volume mounts. They
//!   are evaluated separately.

use crate::types::Cite;
use serde::{Deserialize, Serialize};

pub const SA_TOKEN_TYPE: &str = "kubernetes.io/service-account-token";
pub const SA_UID_ANNOTATION: &str = "kubernetes.io/service-account.uid";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccountSnapshot {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSecret {
    pub namespace: String,
    pub name: String,
    /// Value of `kubernetes.io/service-account.uid` annotation.
    pub annotated_uid: Option<String>,
    /// True when `data["token"]` is populated.
    pub has_token: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenAction {
    /// SA already has a fresh, populated, matching secret — no work.
    NoOp,
    /// No matching secret found — issue one.
    Create,
    /// Secret exists but UID mismatched — delete (a new one will be created
    /// on the next pass).
    DeleteStale,
    /// Secret exists for the correct UID but `data["token"]` is empty —
    /// re-fill it via the signing call.
    PopulateToken,
}

/// Decide what the controller should do with the (SA, observed secrets) pair.
pub fn evaluate(sa: &ServiceAccountSnapshot, observed: &[TokenSecret]) -> TokenAction {
    // First, look for any secret for the SA's namespace whose annotated UID
    // does NOT match the current SA UID → stale.
    for s in observed {
        if s.namespace != sa.namespace {
            continue;
        }
        if let Some(uid) = &s.annotated_uid {
            if uid != &sa.uid {
                return TokenAction::DeleteStale;
            }
        }
    }
    // Look for a fresh secret for this SA.
    let fresh = observed.iter().find(|s| {
        s.namespace == sa.namespace && s.annotated_uid.as_deref() == Some(sa.uid.as_str())
    });
    match fresh {
        Some(s) if !s.has_token => TokenAction::PopulateToken,
        Some(_) => TokenAction::NoOp,
        None => TokenAction::Create,
    }
}

// ── BoundServiceAccountTokens (projected) ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundTokenRequest {
    pub sa_namespace: String,
    pub sa_name: String,
    pub audiences: Vec<String>,
    /// Requested expiration. Upstream caps to 1h..24h; defaults to 1h.
    pub expiration_sec: u32,
}

pub const MIN_BOUND_TOKEN_EXPIRATION_SEC: u32 = 600;
pub const MAX_BOUND_TOKEN_EXPIRATION_SEC: u32 = 24 * 60 * 60;
pub const DEFAULT_BOUND_TOKEN_EXPIRATION_SEC: u32 = 60 * 60;

/// Validate + clamp the requested token expiration to the supported window.
/// Mirrors `pkg/serviceaccount/jwt.go::Generator.GenerateToken`.
pub fn clamp_bound_token_expiration(req: &BoundTokenRequest) -> u32 {
    req.expiration_sec.clamp(
        MIN_BOUND_TOKEN_EXPIRATION_SEC,
        MAX_BOUND_TOKEN_EXPIRATION_SEC,
    )
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new(
    "pkg/controller/serviceaccount/tokens_controller.go",
    "TokensController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn sa(uid: &str) -> ServiceAccountSnapshot {
        ServiceAccountSnapshot {
            namespace: "default".into(),
            name: "default".into(),
            uid: uid.into(),
        }
    }

    fn ts(uid: Option<&str>, has_token: bool) -> TokenSecret {
        TokenSecret {
            namespace: "default".into(),
            name: "default-token-xyz".into(),
            annotated_uid: uid.map(|s| s.to_string()),
            has_token,
        }
    }

    #[test]
    fn no_secret_present_creates() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "ensureReferencedToken",
            "tenant-sa-token-create"
        );
        assert_eq!(evaluate(&sa("u1"), &[]), TokenAction::Create);
    }

    #[test]
    fn matching_uid_with_token_is_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "ensureReferencedToken",
            "tenant-sa-token-noop"
        );
        let s = vec![ts(Some("u1"), true)];
        assert_eq!(evaluate(&sa("u1"), &s), TokenAction::NoOp);
    }

    #[test]
    fn mismatched_uid_triggers_delete_stale() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "deleteSecret",
            "tenant-sa-token-stale"
        );
        let s = vec![ts(Some("OLD"), true)];
        assert_eq!(evaluate(&sa("NEW"), &s), TokenAction::DeleteStale);
    }

    #[test]
    fn matching_uid_without_token_populates() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "generateTokenIfNeeded",
            "tenant-sa-token-populate"
        );
        let s = vec![ts(Some("u1"), false)];
        assert_eq!(evaluate(&sa("u1"), &s), TokenAction::PopulateToken);
    }

    #[test]
    fn cross_namespace_secret_is_ignored() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "ensureReferencedToken",
            "tenant-sa-token-cross-ns"
        );
        let mut s = ts(Some("u1"), true);
        s.namespace = "other".into();
        // No matching secret in "default" namespace → Create.
        assert_eq!(evaluate(&sa("u1"), &[s]), TokenAction::Create);
    }

    #[test]
    fn secret_without_uid_annotation_is_ignored() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "ensureReferencedToken",
            "tenant-sa-token-no-anno"
        );
        let s = vec![ts(None, true)];
        // Without UID annotation, the secret cannot be considered fresh.
        assert_eq!(evaluate(&sa("u1"), &s), TokenAction::Create);
    }

    #[test]
    fn constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "ServiceAccountToken",
            "tenant-sa-token-const"
        );
        assert_eq!(SA_TOKEN_TYPE, "kubernetes.io/service-account-token");
        assert_eq!(SA_UID_ANNOTATION, "kubernetes.io/service-account.uid");
    }

    #[test]
    fn bound_token_request_clamp_within_range() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "Generator.GenerateToken",
            "tenant-sa-token-clamp-mid"
        );
        let req = BoundTokenRequest {
            sa_namespace: "default".into(),
            sa_name: "default".into(),
            audiences: vec!["api".into()],
            expiration_sec: 3 * 60 * 60,
        };
        assert_eq!(clamp_bound_token_expiration(&req), 3 * 60 * 60);
    }

    #[test]
    fn bound_token_request_clamps_below_minimum() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "Generator.GenerateToken",
            "tenant-sa-token-clamp-min"
        );
        let req = BoundTokenRequest {
            sa_namespace: "default".into(),
            sa_name: "default".into(),
            audiences: vec!["api".into()],
            expiration_sec: 60,
        };
        assert_eq!(
            clamp_bound_token_expiration(&req),
            MIN_BOUND_TOKEN_EXPIRATION_SEC
        );
    }

    #[test]
    fn bound_token_request_clamps_above_maximum() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "Generator.GenerateToken",
            "tenant-sa-token-clamp-max"
        );
        let req = BoundTokenRequest {
            sa_namespace: "default".into(),
            sa_name: "default".into(),
            audiences: vec!["api".into()],
            expiration_sec: 7 * 24 * 60 * 60,
        };
        assert_eq!(
            clamp_bound_token_expiration(&req),
            MAX_BOUND_TOKEN_EXPIRATION_SEC
        );
    }

    #[test]
    fn bound_token_default_one_hour() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "DefaultExpirationSeconds",
            "tenant-sa-token-default-exp"
        );
        assert_eq!(DEFAULT_BOUND_TOKEN_EXPIRATION_SEC, 3600);
    }

    #[test]
    fn token_action_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/tokens_controller.go",
            "TokenAction",
            "tenant-sa-token-action-serde"
        );
        for a in [
            TokenAction::NoOp,
            TokenAction::Create,
            TokenAction::DeleteStale,
            TokenAction::PopulateToken,
        ] {
            let s = serde_json::to_string(&a).unwrap();
            let back: TokenAction = serde_json::from_str(&s).unwrap();
            assert_eq!(a, back);
        }
    }
}
