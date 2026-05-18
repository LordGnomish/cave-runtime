// SPDX-License-Identifier: AGPL-3.0-or-later
//! OIDC authentication flow — authorization code with PKCE.
//!
//! Implements RFC 7636 (PKCE) and RFC 6749 (OAuth2 authorization code grant).
//! The platform acts as an OAuth2 relying party talking to Okta/Keycloak.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// PKCE code verifier (random bytes, base64url encoded, 43-128 chars per RFC 7636).
#[derive(Debug, Clone)]
pub struct PkceVerifier(pub String);

/// PKCE code challenge = BASE64URL(SHA256(verifier)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceChallenge(pub String);

impl PkceVerifier {
    /// Generate a cryptographically random code verifier.
    pub fn new() -> Self {
        let mut bytes = [0u8; 64];
        rand::thread_rng().fill_bytes(&mut bytes);
        PkceVerifier(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// Derive the S256 code challenge from this verifier.
    pub fn challenge(&self) -> PkceChallenge {
        let hash = digest(&SHA256, self.0.as_bytes());
        PkceChallenge(URL_SAFE_NO_PAD.encode(hash.as_ref()))
    }

    /// Verify that this verifier matches the given challenge (S256 method).
    pub fn verify_s256(&self, challenge: &PkceChallenge) -> bool {
        self.challenge() == *challenge
    }
}

impl Default for PkceVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Authorization request parameters sent to the IdP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String, // "code"
    pub scope: String,
    pub state: String,
    pub nonce: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: String, // "S256"
}

impl AuthorizationRequest {
    pub fn new(
        client_id: String,
        redirect_uri: String,
        scope: String,
        challenge: &PkceChallenge,
    ) -> Self {
        Self {
            client_id,
            redirect_uri,
            response_type: "code".to_string(),
            scope,
            state: Uuid::new_v4().to_string(),
            nonce: Some(Uuid::new_v4().to_string()),
            code_challenge: challenge.0.clone(),
            code_challenge_method: "S256".to_string(),
        }
    }

    /// Build query string for redirect to IdP authorization endpoint.
    pub fn to_query_string(&self) -> String {
        let mut params = vec![
            ("client_id", self.client_id.clone()),
            ("redirect_uri", self.redirect_uri.clone()),
            ("response_type", self.response_type.clone()),
            ("scope", self.scope.clone()),
            ("state", self.state.clone()),
            ("code_challenge", self.code_challenge.clone()),
            ("code_challenge_method", self.code_challenge_method.clone()),
        ];
        if let Some(ref nonce) = self.nonce {
            params.push(("nonce", nonce.clone()));
        }
        params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// Authorization code returned by IdP after user authenticates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCode {
    pub code: String,
    pub state: String,
    pub session_state: Option<String>,
}

/// Token exchange request (authorization code → tokens).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenExchangeRequest {
    pub grant_type: String, // "authorization_code" or "refresh_token"
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>, // PKCE
    pub refresh_token: Option<String>,
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Tokens returned by the IdP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub scope: String,
}

/// Pending authorization — stored server-side until code is exchanged.
#[derive(Debug, Clone)]
pub struct PendingAuth {
    pub request: AuthorizationRequest,
    pub verifier: PkceVerifier,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// OIDC client — manages authorization flows for the platform.
pub struct OidcClient {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    /// Pending auth flows keyed by `state`
    pending: Arc<RwLock<HashMap<String, PendingAuth>>>,
    http: reqwest::Client,
}

impl OidcClient {
    pub fn new(
        issuer_url: String,
        client_id: String,
        client_secret: Option<String>,
        redirect_uri: String,
    ) -> Self {
        Self {
            issuer_url,
            client_id,
            client_secret,
            redirect_uri,
            pending: Arc::new(RwLock::new(HashMap::new())),
            http: reqwest::Client::new(),
        }
    }

    /// Begin an authorization code + PKCE flow.
    /// Returns (AuthorizationRequest, state) — caller redirects user to IdP URL.
    pub async fn begin_auth(&self, scope: &str) -> (AuthorizationRequest, PkceVerifier) {
        let verifier = PkceVerifier::new();
        let challenge = verifier.challenge();
        let req = AuthorizationRequest::new(
            self.client_id.clone(),
            self.redirect_uri.clone(),
            scope.to_string(),
            &challenge,
        );
        let state = req.state.clone();
        let mut pending = self.pending.write().await;
        pending.insert(
            state,
            PendingAuth {
                request: req.clone(),
                verifier: verifier.clone(),
                created_at: chrono::Utc::now(),
            },
        );
        (req, verifier)
    }

    /// Complete authorization by exchanging code for tokens.
    /// Validates PKCE: ensures code_verifier matches stored challenge.
    pub async fn complete_auth(
        &self,
        code: &str,
        state: &str,
        code_verifier: &str,
        token_endpoint: &str,
    ) -> Result<OidcTokenResponse, String> {
        // Retrieve and remove pending auth
        let pending = {
            let mut map = self.pending.write().await;
            map.remove(state)
                .ok_or_else(|| format!("Unknown state: {state}"))?
        };

        // Validate PKCE
        let verifier = PkceVerifier(code_verifier.to_string());
        let expected_challenge = &pending.request.code_challenge;
        let actual_challenge = verifier.challenge();
        if actual_challenge.0 != *expected_challenge {
            return Err("PKCE code_verifier does not match code_challenge".to_string());
        }

        // Exchange code for tokens
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", self.redirect_uri.clone()),
            ("code_verifier", code_verifier.to_string()),
            ("client_id", self.client_id.clone()),
        ];
        if let Some(ref secret) = self.client_secret {
            form.push(("client_secret", secret.clone()));
        }

        let resp = self
            .http
            .post(token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Token exchange request failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Token endpoint error: {body}"));
        }

        resp.json::<OidcTokenResponse>()
            .await
            .map_err(|e| format!("Token response parse failed: {e}"))
    }

    /// Refresh tokens using a refresh_token.
    pub async fn refresh_tokens(
        &self,
        refresh_token: &str,
        token_endpoint: &str,
    ) -> Result<OidcTokenResponse, String> {
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
            ("client_id", self.client_id.clone()),
        ];
        if let Some(ref secret) = self.client_secret {
            form.push(("client_secret", secret.clone()));
        }

        let resp = self
            .http
            .post(token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| format!("Refresh request failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Refresh endpoint error: {body}"));
        }

        resp.json::<OidcTokenResponse>()
            .await
            .map_err(|e| format!("Refresh response parse failed: {e}"))
    }
}

/// Simple percent-encoding for query string building.
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push_str(&format!("%{byte:02X}"));
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_challenge_roundtrip() {
        let verifier = PkceVerifier::new();
        let challenge = verifier.challenge();
        assert!(verifier.verify_s256(&challenge));
    }

    #[test]
    fn pkce_challenge_is_deterministic() {
        let verifier = PkceVerifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_string());
        let c1 = verifier.challenge();
        let c2 = verifier.challenge();
        assert_eq!(c1, c2);
    }

    #[test]
    fn pkce_wrong_verifier_fails() {
        let v1 = PkceVerifier::new();
        let v2 = PkceVerifier::new();
        let challenge = v1.challenge();
        // v2 must not verify against v1's challenge
        assert!(!v2.verify_s256(&challenge));
    }

    #[test]
    fn pkce_rfc7636_test_vector() {
        // RFC 7636 §B test vector
        let verifier = PkceVerifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_string());
        let challenge = verifier.challenge();
        assert_eq!(challenge.0, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn authorization_request_has_required_fields() {
        let verifier = PkceVerifier::new();
        let challenge = verifier.challenge();
        let req = AuthorizationRequest::new(
            "cave-client".to_string(),
            "https://app.example.com/callback".to_string(),
            "openid profile email".to_string(),
            &challenge,
        );
        assert_eq!(req.response_type, "code");
        assert_eq!(req.code_challenge_method, "S256");
        assert_eq!(req.code_challenge, challenge.0);
        assert!(!req.state.is_empty());
    }

    #[test]
    fn authorization_request_query_string() {
        let verifier = PkceVerifier("test-verifier-abc123".to_string());
        let challenge = verifier.challenge();
        let req = AuthorizationRequest {
            client_id: "myclient".to_string(),
            redirect_uri: "https://example.com/cb".to_string(),
            response_type: "code".to_string(),
            scope: "openid".to_string(),
            state: "mystate".to_string(),
            nonce: None,
            code_challenge: challenge.0.clone(),
            code_challenge_method: "S256".to_string(),
        };
        let qs = req.to_query_string();
        assert!(qs.contains("client_id=myclient"));
        assert!(qs.contains("response_type=code"));
        assert!(qs.contains("code_challenge_method=S256"));
    }

    #[tokio::test]
    async fn begin_auth_stores_pending_flow() {
        let client = OidcClient::new(
            "https://example.okta.com".to_string(),
            "cave-client".to_string(),
            None,
            "https://app.cave.dev/callback".to_string(),
        );
        let (req, _verifier) = client.begin_auth("openid profile email").await;
        let pending = client.pending.read().await;
        assert!(pending.contains_key(&req.state));
    }
}
