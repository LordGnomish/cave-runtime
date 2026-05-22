// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Projected service-account-token volumes — `pkg/serviceaccount/jwt.go`
//! plus `pkg/api/v1/types.go::ServiceAccountTokenProjection`.
//!
//! BoundServiceAccountTokens (KEP-1205) replaces the legacy
//! `kubernetes.io/service-account-token` Secret model with short-lived
//! tokens projected straight into the pod's volume. Each projection
//! carries a `path`, optional `audience` (defaults to API server), and
//! `expirationSeconds` (clamped to 600..86400, default 3600).

use crate::types::{Cite, ControllerError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenProjection {
    pub path: String,
    pub audience: Option<String>,
    pub expiration_sec: u32,
}

pub const DEFAULT_AUDIENCE: &str = "https://kubernetes.default.svc";
pub const DEFAULT_EXPIRATION_SEC: u32 = 3600;
pub const MIN_EXPIRATION_SEC: u32 = 600;
pub const MAX_EXPIRATION_SEC: u32 = 24 * 60 * 60;
pub const ROTATION_TRIGGER_FRACTION: f64 = 0.8;

/// Validate a projected-token request and produce an `effective`
/// projection with defaults applied (audience, expiration clamped).
pub fn normalize(p: &TokenProjection) -> Result<TokenProjection, ControllerError> {
    if p.path.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "ServiceAccountTokenProjection",
            reason: "path required".into(),
        });
    }
    let aud = p
        .audience
        .clone()
        .unwrap_or_else(|| DEFAULT_AUDIENCE.to_string());
    let exp = p
        .expiration_sec
        .clamp(MIN_EXPIRATION_SEC, MAX_EXPIRATION_SEC);
    Ok(TokenProjection {
        path: p.path.clone(),
        audience: Some(aud),
        expiration_sec: exp,
    })
}

/// Determine if the kubelet should rotate a projected token. Mirrors the
/// `tokenRequiresRefresh` predicate — refresh once 80% of expiration has
/// elapsed.
pub fn needs_rotation(issued_at_sec: u64, expiration_sec: u32, now_sec: u64) -> bool {
    let elapsed = now_sec.saturating_sub(issued_at_sec);
    let trigger = (expiration_sec as f64 * ROTATION_TRIGGER_FRACTION) as u64;
    elapsed >= trigger
}

/// TokenReview helper — `pkg/apis/authentication/v1/types.go::TokenReview`.
/// Validate basic shape: a TokenReview with empty `token` is invalid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenReview {
    pub token: String,
    pub audiences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenReviewStatus {
    pub authenticated: bool,
    pub username: Option<String>,
    pub uid: Option<String>,
    pub groups: Vec<String>,
    pub audiences: Vec<String>,
    pub error: Option<String>,
}

/// Lightweight token review — accepts a closure that returns the
/// per-token verdict. The result preserves `audiences` only when the
/// token authenticates AND at least one requested audience is allowed by
/// the verdict (mirrors `tokenAuthenticator.AuthenticateToken`).
pub fn review_token(
    review: &TokenReview,
    verdict: impl FnOnce(&str) -> Option<(String, String, Vec<String>, Vec<String>)>,
) -> Result<TokenReviewStatus, ControllerError> {
    if review.token.is_empty() {
        return Err(ControllerError::InvalidSpec {
            kind: "TokenReview",
            reason: "token required".into(),
        });
    }
    match verdict(&review.token) {
        None => Ok(TokenReviewStatus {
            authenticated: false,
            username: None,
            uid: None,
            groups: vec![],
            audiences: vec![],
            error: Some("invalid token".into()),
        }),
        Some((user, uid, groups, allowed_audiences)) => {
            // Intersect requested audiences with allowed.
            let aud: Vec<String> = if review.audiences.is_empty() {
                allowed_audiences
            } else {
                review
                    .audiences
                    .iter()
                    .filter(|a| allowed_audiences.contains(a))
                    .cloned()
                    .collect()
            };
            Ok(TokenReviewStatus {
                authenticated: true,
                username: Some(user),
                uid: Some(uid),
                groups,
                audiences: aud,
                error: None,
            })
        }
    }
}

/// `automountServiceAccountToken` resolution. Mirrors
/// `pkg/controller/serviceaccount/admission.go::shouldAutomount`.
/// Pod's setting overrides the SA's setting when explicitly set.
pub fn should_automount(sa_default: bool, pod_override: Option<bool>) -> bool {
    pod_override.unwrap_or(sa_default)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/serviceaccount/jwt.go", "BoundServiceAccountTokens");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    #[test]
    fn normalize_fills_default_audience() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "ServiceAccountTokenProjection",
            "tenant-sa-proj-default-aud"
        );
        let p = TokenProjection {
            path: "token".into(),
            audience: None,
            expiration_sec: DEFAULT_EXPIRATION_SEC,
        };
        let norm = normalize(&p).unwrap();
        assert_eq!(norm.audience.as_deref(), Some(DEFAULT_AUDIENCE));
    }

    #[test]
    fn normalize_clamps_expiration_to_min() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "GenerateToken",
            "tenant-sa-proj-clamp-min"
        );
        let p = TokenProjection {
            path: "token".into(),
            audience: None,
            expiration_sec: 30,
        };
        assert_eq!(normalize(&p).unwrap().expiration_sec, MIN_EXPIRATION_SEC);
    }

    #[test]
    fn normalize_clamps_expiration_to_max() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "GenerateToken",
            "tenant-sa-proj-clamp-max"
        );
        let p = TokenProjection {
            path: "token".into(),
            audience: None,
            expiration_sec: 7 * 24 * 60 * 60,
        };
        assert_eq!(normalize(&p).unwrap().expiration_sec, MAX_EXPIRATION_SEC);
    }

    #[test]
    fn normalize_requires_path() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/validation/validation.go",
            "validateProjectedVolumeSource",
            "tenant-sa-proj-no-path"
        );
        let p = TokenProjection {
            path: String::new(),
            audience: None,
            expiration_sec: DEFAULT_EXPIRATION_SEC,
        };
        assert!(normalize(&p).is_err());
    }

    #[test]
    fn needs_rotation_at_eighty_percent_elapsed() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/token/token_manager.go",
            "tokenRequiresRefresh",
            "tenant-sa-proj-rot-80"
        );
        // exp 1000s → 80% = 800s. issued 0, now 800 → rotate.
        assert!(needs_rotation(0, 1000, 800));
        assert!(!needs_rotation(0, 1000, 799));
    }

    #[test]
    fn needs_rotation_handles_clock_skew() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/kubelet/token/token_manager.go",
            "tokenRequiresRefresh",
            "tenant-sa-proj-rot-skew"
        );
        // now < issued → elapsed clamps to 0, no rotation.
        assert!(!needs_rotation(100, 1000, 50));
    }

    #[test]
    fn review_returns_unauthenticated_for_empty_token() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/authentication/tokenreview/storage/storage.go",
            "Create",
            "tenant-sa-proj-rev-empty"
        );
        let r = TokenReview {
            token: String::new(),
            audiences: vec![],
        };
        assert!(review_token(&r, |_| None).is_err());
    }

    #[test]
    fn review_unknown_token_is_unauthenticated() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/authentication/tokenreview/storage/storage.go",
            "Create",
            "tenant-sa-proj-rev-unknown"
        );
        let r = TokenReview {
            token: "x.y.z".into(),
            audiences: vec![],
        };
        let status = review_token(&r, |_| None).unwrap();
        assert!(!status.authenticated);
        assert!(status.error.is_some());
    }

    #[test]
    fn review_intersects_requested_audiences() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/authentication/tokenreview/storage/storage.go",
            "Create",
            "tenant-sa-proj-rev-aud"
        );
        let r = TokenReview {
            token: "abc".into(),
            audiences: vec!["api".into(), "admin".into()],
        };
        let status = review_token(&r, |_| {
            Some((
                "u1".into(),
                "uid-1".into(),
                vec!["g1".into()],
                vec!["api".into()],
            ))
        })
        .unwrap();
        assert!(status.authenticated);
        assert_eq!(status.audiences, vec!["api"]);
    }

    #[test]
    fn review_with_no_requested_audiences_returns_allowed_set() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/registry/authentication/tokenreview/storage/storage.go",
            "Create",
            "tenant-sa-proj-rev-no-req"
        );
        let r = TokenReview {
            token: "abc".into(),
            audiences: vec![],
        };
        let status = review_token(&r, |_| {
            Some((
                "u".into(),
                "uid".into(),
                vec![],
                vec!["api".into(), "admin".into()],
            ))
        })
        .unwrap();
        assert_eq!(status.audiences, vec!["api", "admin"]);
    }

    #[test]
    fn should_automount_pod_overrides_sa() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/admission.go",
            "shouldAutomount",
            "tenant-sa-proj-autom-override"
        );
        // SA default false, pod true → automount.
        assert!(should_automount(false, Some(true)));
        // SA default true, pod false → no automount.
        assert!(!should_automount(true, Some(false)));
    }

    #[test]
    fn should_automount_falls_back_to_sa_default() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/serviceaccount/admission.go",
            "shouldAutomount",
            "tenant-sa-proj-autom-default"
        );
        assert!(should_automount(true, None));
        assert!(!should_automount(false, None));
    }

    #[test]
    fn projection_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/types.go",
            "ServiceAccountTokenProjection",
            "tenant-sa-proj-serde"
        );
        let p = TokenProjection {
            path: "token".into(),
            audience: Some("api".into()),
            expiration_sec: 1800,
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: TokenProjection = serde_json::from_str(&s).unwrap();
        assert_eq!(p.path, back.path);
        assert_eq!(p.expiration_sec, back.expiration_sec);
    }

    #[test]
    fn constants_match_upstream() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/serviceaccount/jwt.go",
            "constants",
            "tenant-sa-proj-const"
        );
        assert_eq!(DEFAULT_EXPIRATION_SEC, 3600);
        assert_eq!(MIN_EXPIRATION_SEC, 600);
        assert_eq!(MAX_EXPIRATION_SEC, 86_400);
        assert_eq!(DEFAULT_AUDIENCE, "https://kubernetes.default.svc");
    }
}
