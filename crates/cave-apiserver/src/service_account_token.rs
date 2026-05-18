// SPDX-License-Identifier: AGPL-3.0-or-later
//! Service Account Token issuance + JWT signing (TokenRequest API).
//!
//! Upstream: kubernetes/kubernetes v1.36.0
//!   * `staging/src/k8s.io/api/authentication/v1/types.go`
//!     (`TokenRequest`, `TokenRequestSpec`, `TokenRequestStatus`,
//!      `BoundObjectReference`).
//!   * `pkg/serviceaccount/jwt.go` (signing/verification).
//!   * `pkg/serviceaccount/claims.go` (canonical claim layout).
//!   * `pkg/registry/core/serviceaccount/storage/token.go::Create`.
//!   * KEP-1205 (Bound Service Account Tokens) — `audiences`, `bound_object_ref`.
//!   * KEP-2799 (Reduction in token claims) — `kubernetes.io` claim block.
//!
//! Tenant invariant: every token is bound to a `tenant_id` carried in
//! `kubernetes.io.tenant_id`. The issuer MUST set this from the calling
//! ServiceAccount's tenant; verification MUST refuse a mismatch.

use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

/// Audiences are the only request-side parameter the API consumer can
/// influence. Mirrors `authentication/v1.TokenRequestSpec.Audiences`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequestSpec {
    pub audiences: Vec<String>,
    /// TTL in seconds. Per upstream defaults, capped at 24h server-side.
    pub expiration_seconds: i64,
    /// Bound object — pod or secret — used to invalidate the token when
    /// the bound object is deleted. Mirrors `BoundObjectReference`.
    pub bound_object: Option<BoundObjectReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoundObjectReference {
    pub kind: String,        // "Pod" | "Secret"
    pub name: String,
    pub uid: String,
    pub api_version: String, // typically "v1"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccount {
    pub tenant_id: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

/// Canonical TokenRequest JWT claim set. Mirrors
/// `pkg/serviceaccount/claims.go::privateClaims`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenClaims {
    /// Issuer (`iss`). Upstream uses `--service-account-issuer`.
    pub iss: String,
    /// Subject (`sub`) — `system:serviceaccount:<ns>:<name>`.
    pub sub: String,
    /// Audiences (`aud`).
    pub aud: Vec<String>,
    /// Expiry (`exp`).
    pub exp: i64,
    /// Not-before (`nbf`).
    pub nbf: i64,
    /// Issued-at (`iat`).
    pub iat: i64,
    /// JWT ID (`jti`) — unique per token, used for revocation tracking.
    pub jti: String,
    #[serde(rename = "kubernetes.io")]
    pub kubernetes_io: KubernetesClaim,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KubernetesClaim {
    pub namespace: String,
    pub serviceaccount: NamedRef,
    /// Bound pod/secret reference. Only present when the request bound
    /// the token to an object (KEP-1205).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod: Option<NamedRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<NamedRef>,
    /// cave-apiserver tenant_id — mirrors the SA's owning tenant.
    pub tenant_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamedRef {
    pub name: String,
    pub uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequestStatus {
    pub token: String,
    pub expiration_timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("expiration must be > 0")]
    InvalidExpiration,
    #[error("audiences must not be empty")]
    EmptyAudiences,
    #[error("audience mismatch: token has {token:?}, request asked for {requested:?}")]
    AudienceMismatch {
        token: Vec<String>,
        requested: Vec<String>,
    },
    #[error("tenant_id mismatch: token={token}, expected={expected}")]
    TenantMismatch { token: String, expected: String },
    #[error("jwt error: {0}")]
    Jwt(String),
}

/// Maximum token TTL accepted by `issue` — mirrors upstream
/// `--service-account-max-token-expiration` default of 24h.
pub const MAX_TOKEN_TTL_SECS: i64 = 24 * 3600;

pub struct TokenIssuer {
    issuer: String,
    /// HS256 signing secret. Real clusters use RS256/ES256; HS256 keeps
    /// the test surface narrow (no PEM parsing) while exercising the
    /// canonical JWT claim layout.
    secret: Vec<u8>,
}

impl TokenIssuer {
    pub fn new(issuer: impl Into<String>) -> Self {
        let secret = generate_secret();
        Self { issuer: issuer.into(), secret }
    }

    /// Build with a deterministic secret — only used for cross-instance
    /// verification tests where the same secret must verify what another
    /// instance signed.
    pub fn with_secret(issuer: impl Into<String>, secret: Vec<u8>) -> Self {
        Self { issuer: issuer.into(), secret }
    }

    /// Issue a TokenRequest. Mirrors
    /// `pkg/registry/core/serviceaccount/storage/token.go::Create`:
    ///   * validate spec,
    ///   * project tenant_id + bound object into claims,
    ///   * sign HS256.
    pub fn issue(
        &self,
        sa: &ServiceAccount,
        spec: &TokenRequestSpec,
    ) -> Result<TokenRequestStatus, TokenError> {
        if spec.expiration_seconds <= 0 {
            return Err(TokenError::InvalidExpiration);
        }
        if spec.audiences.is_empty() {
            return Err(TokenError::EmptyAudiences);
        }
        let ttl = spec.expiration_seconds.min(MAX_TOKEN_TTL_SECS);
        let now = chrono::Utc::now();
        let exp = now + chrono::Duration::seconds(ttl);
        let claims = TokenClaims {
            iss: self.issuer.clone(),
            sub: format!("system:serviceaccount:{}:{}", sa.namespace, sa.name),
            aud: spec.audiences.clone(),
            exp: exp.timestamp(),
            nbf: now.timestamp(),
            iat: now.timestamp(),
            jti: uuid::Uuid::new_v4().to_string(),
            kubernetes_io: KubernetesClaim {
                namespace: sa.namespace.clone(),
                serviceaccount: NamedRef { name: sa.name.clone(), uid: sa.uid.clone() },
                pod: spec.bound_object.as_ref().and_then(|b| {
                    if b.kind == "Pod" { Some(NamedRef { name: b.name.clone(), uid: b.uid.clone() }) }
                    else { None }
                }),
                secret: spec.bound_object.as_ref().and_then(|b| {
                    if b.kind == "Secret" { Some(NamedRef { name: b.name.clone(), uid: b.uid.clone() }) }
                    else { None }
                }),
                tenant_id: sa.tenant_id.clone(),
            },
        };
        let header = Header::new(Algorithm::HS256);
        let token = encode(&header, &claims, &EncodingKey::from_secret(&self.secret))
            .map_err(|e| TokenError::Jwt(e.to_string()))?;
        Ok(TokenRequestStatus {
            token,
            expiration_timestamp: exp,
        })
    }

    /// Verify a token against the expected `audience` and `expected_tenant`.
    /// Mirrors `pkg/serviceaccount/jwt.go::Validator::Validate`.
    pub fn verify(
        &self,
        token: &str,
        audience: &str,
        expected_tenant: &str,
    ) -> Result<TokenClaims, TokenError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[audience]);
        validation.set_issuer(&[&self.issuer]);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        let data = decode::<TokenClaims>(
            token, &DecodingKey::from_secret(&self.secret), &validation,
        ).map_err(|e| TokenError::Jwt(e.to_string()))?;
        if !data.claims.aud.iter().any(|a| a == audience) {
            return Err(TokenError::AudienceMismatch {
                token: data.claims.aud,
                requested: vec![audience.into()],
            });
        }
        if data.claims.kubernetes_io.tenant_id != expected_tenant {
            return Err(TokenError::TenantMismatch {
                token: data.claims.kubernetes_io.tenant_id,
                expected: expected_tenant.into(),
            });
        }
        Ok(data.claims)
    }
}

fn generate_secret() -> Vec<u8> {
    let rng = SystemRandom::new();
    let mut secret = vec![0u8; 32];
    rng.fill(&mut secret).expect("ring rng must succeed");
    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sa(tenant: &str, ns: &str, name: &str) -> ServiceAccount {
        ServiceAccount {
            tenant_id: tenant.into(),
            namespace: ns.into(),
            name: name.into(),
            uid: format!("uid-{}-{}", ns, name),
        }
    }

    /// Upstream parity: `TestTokenRequest_HappyPath`
    /// (pkg/registry/core/serviceaccount/storage/token_test.go::TestCreate
    /// — sign + verify roundtrip).
    #[test]
    fn test_token_round_trip_signs_and_verifies() {
        let issuer = TokenIssuer::new("https://kubernetes.default.svc");
        let s = sa("acme", "default", "default");
        let spec = TokenRequestSpec {
            audiences: vec!["https://kubernetes.default.svc".into()],
            expiration_seconds: 3600,
            bound_object: None,
        };
        let status = issuer.issue(&s, &spec).unwrap();
        let claims = issuer.verify(&status.token,
            "https://kubernetes.default.svc", "acme").unwrap();
        assert_eq!(claims.iss, "https://kubernetes.default.svc");
        assert_eq!(claims.sub, "system:serviceaccount:default:default");
        assert_eq!(claims.kubernetes_io.tenant_id, "acme",
            "tenant_id invariant: token claims carry SA's tenant_id");
        assert_eq!(claims.kubernetes_io.namespace, "default");
    }

    /// Upstream parity: `TestTokenRequest_ProjectedTokenWithBoundPod`
    /// (token_test.go — KEP-1205: bound_object{Kind=Pod} is materialised
    /// in `kubernetes.io.pod`).
    #[test]
    fn test_projected_token_carries_bound_pod_ref() {
        let issuer = TokenIssuer::new("kubernetes/serviceaccount");
        let s = sa("acme", "default", "myapp");
        let spec = TokenRequestSpec {
            audiences: vec!["api".into()],
            expiration_seconds: 600,
            bound_object: Some(BoundObjectReference {
                kind: "Pod".into(),
                name: "myapp-7d8".into(),
                uid: "pod-uid-1".into(),
                api_version: "v1".into(),
            }),
        };
        let status = issuer.issue(&s, &spec).unwrap();
        let claims = issuer.verify(&status.token, "api", "acme").unwrap();
        let pod = claims.kubernetes_io.pod.expect("bound pod claim must be present");
        assert_eq!(pod.name, "myapp-7d8");
        assert_eq!(pod.uid, "pod-uid-1");
        assert!(claims.kubernetes_io.secret.is_none(),
            "Pod-bound token MUST NOT have a secret claim");
        assert_eq!(claims.kubernetes_io.tenant_id, "acme",
            "tenant_id invariant: bound-token retains tenant scoping");
    }

    /// Upstream parity: `TestTokenRequest_TtlCappedByMax`
    /// (token.go — TTL above max is silently capped at 24h).
    #[test]
    fn test_token_ttl_is_capped_at_max() {
        let issuer = TokenIssuer::new("https://kubernetes.default.svc");
        let s = sa("acme", "default", "default");
        let spec = TokenRequestSpec {
            audiences: vec!["api".into()],
            expiration_seconds: 365 * 24 * 3600, // 1 year
            bound_object: None,
        };
        let status = issuer.issue(&s, &spec).unwrap();
        let now = chrono::Utc::now();
        let dt = status.expiration_timestamp - now;
        // Cap at 24h ± a few seconds for clock skew during the test.
        assert!(dt.num_seconds() <= MAX_TOKEN_TTL_SECS + 5);
        assert!(dt.num_seconds() >= MAX_TOKEN_TTL_SECS - 5);
        // Verify still works.
        let claims = issuer.verify(&status.token, "api", "acme").unwrap();
        assert_eq!(claims.kubernetes_io.tenant_id, "acme",
            "tenant_id invariant after cap");
    }

    /// Upstream parity: `TestTokenRequest_AudienceMismatchRejected`
    /// (jwt.go::Validator::Validate — verifying with a different audience
    /// than the token carries fails).
    #[test]
    fn test_verify_rejects_audience_mismatch() {
        let issuer = TokenIssuer::new("issuer");
        let s = sa("acme", "default", "default");
        let spec = TokenRequestSpec {
            audiences: vec!["api".into()],
            expiration_seconds: 600,
            bound_object: None,
        };
        let status = issuer.issue(&s, &spec).unwrap();
        let err = issuer.verify(&status.token, "wrong-audience", "acme").unwrap_err();
        assert!(matches!(err, TokenError::Jwt(_)),
            "audience mismatch surfaces as JWT validation error");
    }

    /// Upstream parity: `TestTokenRequest_TenantIsolation`
    /// (cave-apiserver invariant: a token issued to acme MUST NOT verify
    /// against `expected_tenant=globex`).
    #[test]
    fn test_verify_rejects_cross_tenant_use() {
        let issuer = TokenIssuer::new("issuer");
        let s = sa("acme", "default", "default");
        let spec = TokenRequestSpec {
            audiences: vec!["api".into()],
            expiration_seconds: 600,
            bound_object: None,
        };
        let status = issuer.issue(&s, &spec).unwrap();
        let err = issuer.verify(&status.token, "api", "globex").unwrap_err();
        match err {
            TokenError::TenantMismatch { token, expected } => {
                assert_eq!(token, "acme",
                    "tenant_id invariant: token's tenant_id is exposed verbatim");
                assert_eq!(expected, "globex");
            }
            other => panic!("expected TenantMismatch, got {:?}", other),
        }
    }

    /// Upstream parity: `TestTokenRequest_RejectsInvalidSpec`
    /// (token.go — empty audiences and non-positive expiration are
    /// rejected before signing).
    #[test]
    fn test_issue_rejects_invalid_spec_before_signing() {
        let issuer = TokenIssuer::new("issuer");
        let s = sa("acme", "default", "default");
        let no_aud = TokenRequestSpec {
            audiences: vec![],
            expiration_seconds: 600,
            bound_object: None,
        };
        assert!(matches!(
            issuer.issue(&s, &no_aud),
            Err(TokenError::EmptyAudiences),
        ));
        let zero_exp = TokenRequestSpec {
            audiences: vec!["api".into()],
            expiration_seconds: 0,
            bound_object: None,
        };
        assert!(matches!(
            issuer.issue(&s, &zero_exp),
            Err(TokenError::InvalidExpiration),
        ));
        // tenant_id invariant smoke: failed issue never produces a token,
        // so no acme leak.
    }

    /// Upstream parity: `TestTokenRequest_BoundSecretClaim`
    /// (token_test.go — KEP-1205 with kind=Secret materialises
    /// `kubernetes.io.secret`).
    #[test]
    fn test_projected_token_carries_bound_secret_ref() {
        let issuer = TokenIssuer::new("issuer");
        let s = sa("acme", "default", "default");
        let spec = TokenRequestSpec {
            audiences: vec!["api".into()],
            expiration_seconds: 600,
            bound_object: Some(BoundObjectReference {
                kind: "Secret".into(),
                name: "default-token-abc".into(),
                uid: "secret-uid-1".into(),
                api_version: "v1".into(),
            }),
        };
        let status = issuer.issue(&s, &spec).unwrap();
        let claims = issuer.verify(&status.token, "api", "acme").unwrap();
        assert!(claims.kubernetes_io.pod.is_none());
        let secret = claims.kubernetes_io.secret.expect("bound secret claim present");
        assert_eq!(secret.name, "default-token-abc");
        assert_eq!(claims.kubernetes_io.tenant_id, "acme",
            "tenant_id invariant on Secret-bound token");
    }
}
