// SPDX-License-Identifier: AGPL-3.0-or-later
//
// UMA 2.0 Claim Token + Claim Pushing — UMA-Grant §3.3.1.
//
// The client may push additional claims with the RPT request via
// `claim_token` + `claim_token_format`. Keycloak supports:
//   - `urn:ietf:params:oauth:token-type:jwt` (the default)
//   - `https://openid.net/specs/openid-connect-core-1_0.html#IDToken`
//   - `https://docs.kantarainitiative.org/uma/wg/rec-oauth-uma-claim-token-format-jwt`
//
// We accept JWT-format claim tokens and parse them as a JSON object.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/authorization/authorization/AuthorizationTokenService.java

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

use super::UmaError;

pub const CLAIM_TOKEN_FORMAT_JWT: &str = "urn:ietf:params:oauth:token-type:jwt";
pub const CLAIM_TOKEN_FORMAT_ID_TOKEN: &str =
    "https://openid.net/specs/openid-connect-core-1_0.html#IDToken";

/// Decode the payload portion of a compact-JWS claim token without
/// verifying the signature. Verification is policy: realms that need it
/// must compose `keycloak::token_endpoint::decode_access_token` ahead of
/// this helper.
///
/// Returns the payload as a `serde_json::Value` so policy code can match
/// arbitrary client-supplied claims.
pub fn decode_claim_token(token: &str, format: &str) -> Result<serde_json::Value, UmaError> {
    if format != CLAIM_TOKEN_FORMAT_JWT && format != CLAIM_TOKEN_FORMAT_ID_TOKEN {
        return Err(UmaError::InvalidRequest("unsupported claim_token_format"));
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(UmaError::InvalidToken);
    }
    let raw = B64.decode(parts[1].as_bytes()).map_err(|_| UmaError::InvalidToken)?;
    serde_json::from_slice(&raw).map_err(|_| UmaError::InvalidToken)
}

/// Merge pushed claims (`claims_payload`) into the evaluation context
/// scopes — keycloak permits clients to push additional scopes via the
/// `scope` claim. Other claim keys are surfaced as-is for the policy
/// engine to inspect via custom matchers.
pub fn extract_pushed_scopes(claims: &serde_json::Value) -> Vec<String> {
    if let Some(arr) = claims.get("scope").and_then(|v| v.as_array()) {
        return arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(s) = claims.get("scope").and_then(|v| v.as_str()) {
        return s.split_whitespace().map(|x| x.to_string()).collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(payload: serde_json::Value) -> String {
        let hdr = B64.encode(b"{\"alg\":\"none\"}");
        let pl = B64.encode(serde_json::to_vec(&payload).unwrap());
        let sig = B64.encode(b"");
        format!("{hdr}.{pl}.{sig}")
    }

    // upstream: uma-grant §3.3.1 — JWT claim_token decodes to its payload.
    #[test]
    fn jwt_claim_token_decodes() {
        let t = make_jwt(serde_json::json!({"scope":"view edit","email":"alice@example"}));
        let v = decode_claim_token(&t, CLAIM_TOKEN_FORMAT_JWT).unwrap();
        assert_eq!(v["email"], "alice@example");
    }

    // upstream: uma-grant §3.3.1 — unsupported claim_token_format rejected.
    #[test]
    fn unknown_format_rejected() {
        let err = decode_claim_token("a.b.c", "urn:custom").unwrap_err();
        assert!(matches!(err, UmaError::InvalidRequest(_)));
    }

    // upstream: uma-grant §3.3.1 — malformed (not 3 segments) rejected.
    #[test]
    fn malformed_token_rejected() {
        let err = decode_claim_token("not-a-jwt", CLAIM_TOKEN_FORMAT_JWT).unwrap_err();
        assert_eq!(err, UmaError::InvalidToken);
    }

    // upstream: keycloak AuthorizationTokenService.gatherScopes() — pushed
    // `scope` as space-delimited string is split.
    #[test]
    fn pushed_scopes_from_string() {
        let v = serde_json::json!({"scope":"a b c"});
        assert_eq!(extract_pushed_scopes(&v), vec!["a", "b", "c"]);
    }

    // upstream: keycloak AuthorizationTokenService.gatherScopes() — array
    // form also supported.
    #[test]
    fn pushed_scopes_from_array() {
        let v = serde_json::json!({"scope":["a","b"]});
        assert_eq!(extract_pushed_scopes(&v), vec!["a", "b"]);
    }
}
