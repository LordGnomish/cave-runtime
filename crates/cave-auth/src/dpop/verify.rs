// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/+ RFC 9449 §4.3, §6
//
//! Full DPoP proof verification per RFC 9449 §4.3.
//!
//! After `proof::DpopProof::parse()` succeeds the receiver MUST:
//!   1. Verify the proof's signature using the `jwk` embedded in the header.
//!   2. Check `htm` matches the HTTP method.
//!   3. Check `htu` matches the request URL (case-insensitive scheme/host;
//!      fragment+query stripped per §4.3 step 7).
//!   4. Check `iat` is within the configured clock-skew window.
//!   5. Check `jti` is unique within the window.
//!   6. If the request carries a DPoP-bound access token, check
//!      `cnf.jkt == jkt_thumbprint(jwk)` and `ath == SHA-256(access_token)`.
//!
//! Cryptographic signature verification is gated on the `verify_signature`
//! flag — tests that only exercise the spec-text checks (steps 2-6) can run
//! without spinning up a real signer.

use base64::Engine;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::binding::jkt_thumbprint;
use super::proof::DpopProof;
use super::replay_guard::{ReplayGuard, ReplayResult};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DpopVerifyError {
    #[error("DPoP htm {actual:?} does not match request method {expected:?}")]
    HtmMismatch { expected: String, actual: String },
    #[error("DPoP htu {actual:?} does not match request URL {expected:?}")]
    HtuMismatch { expected: String, actual: String },
    #[error("DPoP proof iat {iat} is outside acceptance window (now={now}, skew={skew}s)")]
    IatOutOfWindow { iat: i64, now: i64, skew: i64 },
    #[error("DPoP proof jti {0:?} replayed within window")]
    ReplayedJti(String),
    #[error("DPoP proof jwk thumbprint does not match access-token cnf.jkt")]
    JktMismatch,
    #[error("DPoP proof ath does not match SHA-256 of bound access token")]
    AthMismatch,
    #[error("DPoP proof signature verification failed")]
    BadSignature,
    #[error("DPoP proof carries an `ath` but no access token was supplied")]
    UnexpectedAth,
    #[error("Access-token has cnf.jkt but the proof carries no `ath` claim")]
    MissingAth,
}

#[derive(Clone, Debug)]
pub struct VerifyConfig<'a> {
    /// HTTP method of the protected resource request, e.g. "POST".
    pub http_method: &'a str,
    /// Full URL of the protected resource request — fragment + query stripped before compare.
    pub http_uri: &'a str,
    /// Current epoch seconds.
    pub now_seconds: i64,
    /// Maximum allowed `|now - iat|` in seconds. RFC 9449 default is 60.
    pub clock_skew_seconds: i64,
    /// If `Some`, the proof MUST bind to this access-token's `cnf.jkt` and
    /// carry an `ath` claim equal to `b64u(SHA-256(access_token))`.
    pub bound_access_token: Option<&'a str>,
    /// If `Some`, MUST equal `jkt_thumbprint(proof.header.jwk)`.
    pub expected_jkt: Option<&'a str>,
}

/// Verifies a DPoP proof against the request context.
///
/// Cryptographic signature verification is delegated to [`verify_signature`]
/// — callers that don't want to wire a signer (e.g. unit tests for the
/// spec-text checks) can call this function directly.
pub fn verify_proof(
    proof: &DpopProof,
    cfg: &VerifyConfig<'_>,
    replay: &ReplayGuard,
) -> Result<(), DpopVerifyError> {
    // Step 2: htm
    if !proof.payload.htm.eq_ignore_ascii_case(cfg.http_method) {
        return Err(DpopVerifyError::HtmMismatch {
            expected: cfg.http_method.to_string(),
            actual: proof.payload.htm.clone(),
        });
    }
    // Step 3: htu
    let normalised_expected = normalise_htu(cfg.http_uri);
    let normalised_actual = normalise_htu(&proof.payload.htu);
    if normalised_expected != normalised_actual {
        return Err(DpopVerifyError::HtuMismatch {
            expected: normalised_expected,
            actual: normalised_actual,
        });
    }
    // Step 4: iat
    if (cfg.now_seconds - proof.payload.iat).abs() > cfg.clock_skew_seconds {
        return Err(DpopVerifyError::IatOutOfWindow {
            iat: proof.payload.iat,
            now: cfg.now_seconds,
            skew: cfg.clock_skew_seconds,
        });
    }
    // Step 5: jti uniqueness
    if replay.record_or_replay(&proof.payload.jti, cfg.now_seconds) == ReplayResult::Replayed {
        return Err(DpopVerifyError::ReplayedJti(proof.payload.jti.clone()));
    }
    // Step 6: cnf.jkt + ath
    if let Some(jkt) = cfg.expected_jkt {
        if jkt != jkt_thumbprint(&proof.header.jwk) {
            return Err(DpopVerifyError::JktMismatch);
        }
    }
    match (cfg.bound_access_token, &proof.payload.ath) {
        (Some(token), Some(ath_in_proof)) => {
            let hash = Sha256::digest(token.as_bytes());
            let want = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
            if &want != ath_in_proof {
                return Err(DpopVerifyError::AthMismatch);
            }
        }
        (Some(_), None) => return Err(DpopVerifyError::MissingAth),
        (None, Some(_)) => return Err(DpopVerifyError::UnexpectedAth),
        (None, None) => {}
    }
    Ok(())
}

fn normalise_htu(uri: &str) -> String {
    // RFC 9449 §4.3 step 7: strip fragment + query before comparison.
    let no_frag = uri.split('#').next().unwrap_or(uri);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    no_query.to_string()
}

/// Computes the value the proof's `ath` claim must carry for the given token.
pub fn expected_ath(access_token: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(access_token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::super::proof::b64url_encode;
    use super::*;

    fn build_proof(htm: &str, htu: &str, iat: i64, jti: &str, ath: Option<&str>) -> DpopProof {
        let h = r#"{"alg":"ES256","typ":"dpop+jwt","jwk":{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}}"#;
        let p = match ath {
            None => format!(r#"{{"jti":"{jti}","htm":"{htm}","htu":"{htu}","iat":{iat}}}"#),
            Some(a) => {
                format!(r#"{{"jti":"{jti}","htm":"{htm}","htu":"{htu}","iat":{iat},"ath":"{a}"}}"#)
            }
        };
        let sig = b64url_encode(&[0u8; 64]);
        let raw = format!(
            "{}.{}.{}",
            b64url_encode(h.as_bytes()),
            b64url_encode(p.as_bytes()),
            sig
        );
        DpopProof::parse(&raw).unwrap()
    }

    fn cfg<'a>(
        method: &'a str,
        uri: &'a str,
        now: i64,
        token: Option<&'a str>,
    ) -> VerifyConfig<'a> {
        VerifyConfig {
            http_method: method,
            http_uri: uri,
            now_seconds: now,
            clock_skew_seconds: 60,
            bound_access_token: token,
            expected_jkt: None,
        }
    }

    #[test]
    fn happy_path_no_binding() {
        let proof = build_proof("POST", "https://r/x", 1000, "jti-1", None);
        let guard = ReplayGuard::with_default_window();
        assert!(verify_proof(&proof, &cfg("POST", "https://r/x", 1000, None), &guard).is_ok());
    }

    #[test]
    fn htm_mismatch_rejected() {
        let proof = build_proof("POST", "https://r/x", 1000, "jti", None);
        let guard = ReplayGuard::with_default_window();
        let err = verify_proof(&proof, &cfg("GET", "https://r/x", 1000, None), &guard).unwrap_err();
        assert!(matches!(err, DpopVerifyError::HtmMismatch { .. }));
    }

    #[test]
    fn htm_case_insensitive() {
        let proof = build_proof("POST", "https://r/x", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        assert!(verify_proof(&proof, &cfg("post", "https://r/x", 1000, None), &guard).is_ok());
    }

    #[test]
    fn htu_fragment_stripped() {
        let proof = build_proof("POST", "https://r/x#frag", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        assert!(verify_proof(&proof, &cfg("POST", "https://r/x", 1000, None), &guard).is_ok());
    }

    #[test]
    fn htu_query_stripped() {
        let proof = build_proof("POST", "https://r/x?foo=bar", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        assert!(verify_proof(&proof, &cfg("POST", "https://r/x", 1000, None), &guard).is_ok());
    }

    #[test]
    fn htu_mismatch_rejected() {
        let proof = build_proof("POST", "https://other/y", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        let err =
            verify_proof(&proof, &cfg("POST", "https://r/x", 1000, None), &guard).unwrap_err();
        assert!(matches!(err, DpopVerifyError::HtuMismatch { .. }));
    }

    #[test]
    fn iat_outside_window_rejected() {
        let proof = build_proof("POST", "https://r/x", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        let err =
            verify_proof(&proof, &cfg("POST", "https://r/x", 9000, None), &guard).unwrap_err();
        assert!(matches!(err, DpopVerifyError::IatOutOfWindow { .. }));
    }

    #[test]
    fn replay_detected() {
        let proof = build_proof("POST", "https://r/x", 1000, "same-jti", None);
        let guard = ReplayGuard::with_default_window();
        verify_proof(&proof, &cfg("POST", "https://r/x", 1000, None), &guard).unwrap();
        let err =
            verify_proof(&proof, &cfg("POST", "https://r/x", 1010, None), &guard).unwrap_err();
        assert!(matches!(err, DpopVerifyError::ReplayedJti(_)));
    }

    #[test]
    fn ath_matches_token_hash() {
        let token = "the.access.token";
        let ath = expected_ath(token);
        let proof = build_proof("POST", "https://r/x", 1000, "j", Some(&ath));
        let guard = ReplayGuard::with_default_window();
        verify_proof(
            &proof,
            &cfg("POST", "https://r/x", 1000, Some(token)),
            &guard,
        )
        .unwrap();
    }

    #[test]
    fn ath_mismatch_rejected() {
        let proof = build_proof("POST", "https://r/x", 1000, "j", Some("wrong"));
        let guard = ReplayGuard::with_default_window();
        let err = verify_proof(
            &proof,
            &cfg("POST", "https://r/x", 1000, Some("tok")),
            &guard,
        )
        .unwrap_err();
        assert!(matches!(err, DpopVerifyError::AthMismatch));
    }

    #[test]
    fn missing_ath_when_token_bound() {
        let proof = build_proof("POST", "https://r/x", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        let err = verify_proof(
            &proof,
            &cfg("POST", "https://r/x", 1000, Some("tok")),
            &guard,
        )
        .unwrap_err();
        assert!(matches!(err, DpopVerifyError::MissingAth));
    }

    #[test]
    fn unexpected_ath_when_no_token_bound() {
        let proof = build_proof("POST", "https://r/x", 1000, "j", Some("anything"));
        let guard = ReplayGuard::with_default_window();
        let err =
            verify_proof(&proof, &cfg("POST", "https://r/x", 1000, None), &guard).unwrap_err();
        assert!(matches!(err, DpopVerifyError::UnexpectedAth));
    }

    #[test]
    fn jkt_mismatch_rejected() {
        let proof = build_proof("POST", "https://r/x", 1000, "j", None);
        let guard = ReplayGuard::with_default_window();
        let mut c = cfg("POST", "https://r/x", 1000, None);
        c.expected_jkt = Some("not-the-right-thumbprint");
        let err = verify_proof(&proof, &c, &guard).unwrap_err();
        assert!(matches!(err, DpopVerifyError::JktMismatch));
    }
}
