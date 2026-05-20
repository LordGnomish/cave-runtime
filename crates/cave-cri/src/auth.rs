// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Registry authentication.
//!
//! Mirrors containerd's `pkg/cri/server/image_pull.go` resolver and the
//! `Resolver` interface from `containerd/remotes/docker/authorizer.go`.
//! Each known registry vendor uses a different credential exchange:
//!
//! - **Docker Hub** → token endpoint `auth.docker.io/token` with
//!   `service=registry.docker.io` and `scope=repository:<repo>:pull`.
//! - **AWS ECR** → AWS Signature V4 against `ecr:GetAuthorizationToken`,
//!   then bearer with the returned `authorization_data.token`.
//! - **GCP GCR / Artifact Registry** → bearer with a Google OAuth2
//!   access token (`oauth2.googleapis.com/token` for service-account
//!   flows, gcloud-style `_dcgcloud_token` for end-user creds).
//! - **Azure ACR** → exchange an AAD access token at
//!   `https://<registry>/oauth2/exchange` for an ACR refresh token.
//! - **GitHub Container Registry** → bearer using a Personal Access
//!   Token directly.
//! - **Generic / on-prem** → HTTP Basic with stored username:password.

use serde::{Deserialize, Serialize};

/// One of the credential schemes cave-cri knows how to resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scheme")]
pub enum AuthScheme {
    /// No credentials. Public registry.
    None,
    /// HTTP Basic with username and password (or PAT).
    Basic { username: String, password: String },
    /// Static bearer token (e.g. GitHub PAT in `Authorization: Bearer`).
    Bearer { token: String },
    /// Docker Hub style OAuth2 — exchange anonymous request for a
    /// short-lived bearer token at `token_url`.
    Oauth2 {
        token_url: String,
        service: String,
        scope: String,
        username: Option<String>,
        password: Option<String>,
    },
    /// AWS ECR — exchange access key + secret for a short-lived
    /// `authorization_data.token` via `ecr:GetAuthorizationToken`.
    AwsEcr {
        access_key_id: String,
        secret_access_key: String,
        region: String,
    },
    /// GCP GCR / Artifact Registry — bearer using Google OAuth2 access
    /// token, typically rotated by the GCE metadata server.
    GcpGcr { access_token: String },
    /// Azure ACR — bearer with an AAD access token rebuilt as an ACR
    /// refresh token.
    AzureAcr {
        aad_access_token: String,
        tenant: String,
    },
}

impl AuthScheme {
    /// Pick the scheme to *try first* for `registry_host` based on its
    /// hostname. Callers may override via configured credentials.
    pub fn default_for_registry(registry_host: &str) -> AuthScheme {
        let host = registry_host.to_ascii_lowercase();
        if host == "docker.io" || host.ends_with(".docker.io") {
            return AuthScheme::Oauth2 {
                token_url: "https://auth.docker.io/token".into(),
                service: "registry.docker.io".into(),
                scope: String::new(),
                username: None,
                password: None,
            };
        }
        if host.contains(".dkr.ecr.") {
            // <account>.dkr.ecr.<region>.amazonaws.com
            let region = host
                .split('.')
                .skip_while(|p| *p != "ecr")
                .nth(1)
                .unwrap_or("us-east-1")
                .to_string();
            return AuthScheme::AwsEcr {
                access_key_id: String::new(),
                secret_access_key: String::new(),
                region,
            };
        }
        if host.ends_with(".gcr.io") || host == "gcr.io" || host.ends_with("-docker.pkg.dev") {
            return AuthScheme::GcpGcr {
                access_token: String::new(),
            };
        }
        if host.ends_with(".azurecr.io") {
            let tenant = host.trim_end_matches(".azurecr.io").to_string();
            return AuthScheme::AzureAcr {
                aad_access_token: String::new(),
                tenant,
            };
        }
        if host == "ghcr.io" {
            return AuthScheme::Bearer {
                token: String::new(),
            };
        }
        AuthScheme::None
    }

    /// Build the value for the `Authorization` header. Returns `None` if
    /// no credentials are configured (callers should still attempt the
    /// request without auth).
    pub fn authorization_header(&self) -> Option<String> {
        match self {
            AuthScheme::None => None,
            AuthScheme::Basic { username, password } => {
                let raw = format!("{}:{}", username, password);
                let encoded = base64_encode(raw.as_bytes());
                Some(format!("Basic {}", encoded))
            }
            AuthScheme::Bearer { token } if !token.is_empty() => Some(format!("Bearer {}", token)),
            AuthScheme::Bearer { .. } => None,
            AuthScheme::Oauth2 {
                username: Some(u),
                password: Some(p),
                ..
            } => {
                // Pre-token: send Basic for the exchange request.
                let raw = format!("{}:{}", u, p);
                Some(format!("Basic {}", base64_encode(raw.as_bytes())))
            }
            AuthScheme::Oauth2 { .. } => None,
            AuthScheme::AwsEcr { .. } => None, // Signed separately; no static header.
            AuthScheme::GcpGcr { access_token } if !access_token.is_empty() => {
                Some(format!("Bearer {}", access_token))
            }
            AuthScheme::GcpGcr { .. } => None,
            AuthScheme::AzureAcr {
                aad_access_token, ..
            } if !aad_access_token.is_empty() => Some(format!("Bearer {}", aad_access_token)),
            AuthScheme::AzureAcr { .. } => None,
        }
    }

    /// Build the URL the runtime should hit to exchange credentials for a
    /// short-lived registry token. `repo` is the repository being pulled
    /// (used for OAuth2 scope generation).
    pub fn token_exchange_url(&self, repo: &str) -> Option<String> {
        match self {
            AuthScheme::Oauth2 {
                token_url, service, ..
            } => Some(format!(
                "{}?service={}&scope=repository:{}:pull",
                token_url, service, repo
            )),
            AuthScheme::AwsEcr { region, .. } => {
                Some(format!("https://api.ecr.{}.amazonaws.com/", region))
            }
            AuthScheme::AzureAcr { tenant, .. } => {
                Some(format!("https://{}.azurecr.io/oauth2/exchange", tenant))
            }
            _ => None,
        }
    }
}

/// Vendor classification used for routing token-exchange flows. Decoupled
/// from `AuthScheme` so callers can introspect what they're up against
/// without matching on every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistryVendor {
    DockerHub,
    AwsEcr,
    GcpGcr,
    AzureAcr,
    Ghcr,
    Generic,
}

impl RegistryVendor {
    pub fn from_host(host: &str) -> RegistryVendor {
        let h = host.to_ascii_lowercase();
        if h == "docker.io" || h.ends_with(".docker.io") {
            RegistryVendor::DockerHub
        } else if h.contains(".dkr.ecr.") {
            RegistryVendor::AwsEcr
        } else if h.ends_with(".gcr.io") || h == "gcr.io" || h.ends_with("-docker.pkg.dev") {
            RegistryVendor::GcpGcr
        } else if h.ends_with(".azurecr.io") {
            RegistryVendor::AzureAcr
        } else if h == "ghcr.io" {
            RegistryVendor::Ghcr
        } else {
            RegistryVendor::Generic
        }
    }
}

/// In-process credential store keyed by registry hostname.
#[derive(Debug, Default)]
pub struct CredentialStore {
    creds: std::sync::RwLock<std::collections::HashMap<String, AuthScheme>>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, registry_host: &str, scheme: AuthScheme) {
        self.creds
            .write()
            .unwrap()
            .insert(registry_host.to_string(), scheme);
    }

    pub fn get(&self, registry_host: &str) -> AuthScheme {
        self.creds
            .read()
            .unwrap()
            .get(registry_host)
            .cloned()
            .unwrap_or_else(|| AuthScheme::default_for_registry(registry_host))
    }

    pub fn remove(&self, registry_host: &str) -> Option<AuthScheme> {
        self.creds.write().unwrap().remove(registry_host)
    }

    pub fn len(&self) -> usize {
        self.creds.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.creds.read().unwrap().is_empty()
    }
}

// ── Minimal base64 (RFC 4648) ────────────────────────────────────────────────
//
// We avoid pulling in the `base64` crate just for the basic auth header.
// Implementation mirrors the standard alphabet and handles padding.

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(BASE64_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(BASE64_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── default_for_registry ──────────────────────────────────────────────────

    #[test]
    fn default_for_docker_hub_returns_oauth2() {
        let s = AuthScheme::default_for_registry("docker.io");
        match s {
            AuthScheme::Oauth2 {
                token_url, service, ..
            } => {
                assert_eq!(token_url, "https://auth.docker.io/token");
                assert_eq!(service, "registry.docker.io");
            }
            _ => panic!("expected Oauth2"),
        }
    }

    #[test]
    fn default_for_ecr_extracts_region() {
        let s = AuthScheme::default_for_registry("123456789012.dkr.ecr.us-west-2.amazonaws.com");
        match s {
            AuthScheme::AwsEcr { region, .. } => assert_eq!(region, "us-west-2"),
            _ => panic!("expected AwsEcr"),
        }
    }

    #[test]
    fn default_for_gcr_uses_gcp_scheme() {
        for host in ["gcr.io", "us.gcr.io", "us-central1-docker.pkg.dev"] {
            let s = AuthScheme::default_for_registry(host);
            assert!(
                matches!(s, AuthScheme::GcpGcr { .. }),
                "host {} → {:?}",
                host,
                s
            );
        }
    }

    #[test]
    fn default_for_azure_extracts_tenant() {
        let s = AuthScheme::default_for_registry("myorg.azurecr.io");
        match s {
            AuthScheme::AzureAcr { tenant, .. } => assert_eq!(tenant, "myorg"),
            _ => panic!("expected AzureAcr"),
        }
    }

    #[test]
    fn default_for_ghcr_returns_bearer() {
        let s = AuthScheme::default_for_registry("ghcr.io");
        assert!(matches!(s, AuthScheme::Bearer { .. }));
    }

    #[test]
    fn default_for_unknown_returns_none() {
        let s = AuthScheme::default_for_registry("harbor.internal");
        assert_eq!(s, AuthScheme::None);
    }

    #[test]
    fn default_for_registry_is_case_insensitive() {
        let s = AuthScheme::default_for_registry("MYORG.AZURECR.IO");
        match s {
            AuthScheme::AzureAcr { tenant, .. } => assert_eq!(tenant, "myorg"),
            _ => panic!("expected AzureAcr"),
        }
    }

    // ── authorization_header ──────────────────────────────────────────────────

    #[test]
    fn basic_auth_header_is_base64_userpass() {
        let s = AuthScheme::Basic {
            username: "alice".into(),
            password: "secret".into(),
        };
        let h = s.authorization_header().unwrap();
        // alice:secret = "YWxpY2U6c2VjcmV0"
        assert_eq!(h, "Basic YWxpY2U6c2VjcmV0");
    }

    #[test]
    fn bearer_with_token_renders() {
        let s = AuthScheme::Bearer {
            token: "abc".into(),
        };
        assert_eq!(s.authorization_header(), Some("Bearer abc".to_string()));
    }

    #[test]
    fn bearer_with_empty_token_returns_none() {
        let s = AuthScheme::Bearer {
            token: String::new(),
        };
        assert!(s.authorization_header().is_none());
    }

    #[test]
    fn oauth2_without_creds_returns_none() {
        let s = AuthScheme::Oauth2 {
            token_url: "x".into(),
            service: "x".into(),
            scope: String::new(),
            username: None,
            password: None,
        };
        assert!(s.authorization_header().is_none());
    }

    #[test]
    fn oauth2_with_creds_uses_basic_for_exchange() {
        let s = AuthScheme::Oauth2 {
            token_url: "x".into(),
            service: "x".into(),
            scope: String::new(),
            username: Some("u".into()),
            password: Some("p".into()),
        };
        // u:p = "dTpw"
        assert_eq!(s.authorization_header(), Some("Basic dTpw".to_string()));
    }

    #[test]
    fn aws_ecr_static_header_is_none() {
        let s = AuthScheme::AwsEcr {
            access_key_id: "AKIA...".into(),
            secret_access_key: "secret".into(),
            region: "eu-west-1".into(),
        };
        assert!(s.authorization_header().is_none());
    }

    #[test]
    fn gcp_with_token_renders_bearer() {
        let s = AuthScheme::GcpGcr {
            access_token: "ya29.abc".into(),
        };
        assert_eq!(s.authorization_header(), Some("Bearer ya29.abc".into()));
    }

    #[test]
    fn azure_with_token_renders_bearer() {
        let s = AuthScheme::AzureAcr {
            aad_access_token: "eyJ.aad".into(),
            tenant: "myorg".into(),
        };
        assert_eq!(s.authorization_header(), Some("Bearer eyJ.aad".into()));
    }

    // ── token_exchange_url ────────────────────────────────────────────────────

    #[test]
    fn oauth2_token_exchange_url_includes_scope() {
        let s = AuthScheme::default_for_registry("docker.io");
        let url = s.token_exchange_url("library/nginx").unwrap();
        assert!(url.contains("service=registry.docker.io"));
        assert!(url.contains("scope=repository:library/nginx:pull"));
    }

    #[test]
    fn ecr_token_exchange_url_uses_region() {
        let s = AuthScheme::AwsEcr {
            access_key_id: String::new(),
            secret_access_key: String::new(),
            region: "ap-southeast-1".into(),
        };
        let url = s.token_exchange_url("any").unwrap();
        assert!(url.contains("ecr.ap-southeast-1.amazonaws.com"));
    }

    #[test]
    fn azure_token_exchange_url_uses_tenant() {
        let s = AuthScheme::AzureAcr {
            aad_access_token: String::new(),
            tenant: "myorg".into(),
        };
        let url = s.token_exchange_url("any").unwrap();
        assert!(url.contains("myorg.azurecr.io/oauth2/exchange"));
    }

    #[test]
    fn none_has_no_token_exchange_url() {
        assert!(AuthScheme::None.token_exchange_url("any").is_none());
    }

    // ── RegistryVendor classification ────────────────────────────────────────

    #[test]
    fn vendor_classifies_known_hosts() {
        assert_eq!(
            RegistryVendor::from_host("docker.io"),
            RegistryVendor::DockerHub
        );
        assert_eq!(
            RegistryVendor::from_host("a.dkr.ecr.us-east-1.amazonaws.com"),
            RegistryVendor::AwsEcr
        );
        assert_eq!(RegistryVendor::from_host("gcr.io"), RegistryVendor::GcpGcr);
        assert_eq!(
            RegistryVendor::from_host("us.gcr.io"),
            RegistryVendor::GcpGcr
        );
        assert_eq!(
            RegistryVendor::from_host("us-central1-docker.pkg.dev"),
            RegistryVendor::GcpGcr
        );
        assert_eq!(
            RegistryVendor::from_host("myorg.azurecr.io"),
            RegistryVendor::AzureAcr
        );
        assert_eq!(RegistryVendor::from_host("ghcr.io"), RegistryVendor::Ghcr);
        assert_eq!(
            RegistryVendor::from_host("harbor.internal"),
            RegistryVendor::Generic
        );
    }

    // ── CredentialStore ───────────────────────────────────────────────────────

    #[test]
    fn credential_store_set_and_get() {
        let s = CredentialStore::new();
        s.set("ghcr.io", AuthScheme::Bearer { token: "tk".into() });
        match s.get("ghcr.io") {
            AuthScheme::Bearer { token } => assert_eq!(token, "tk"),
            _ => panic!("expected Bearer"),
        }
    }

    #[test]
    fn credential_store_falls_back_to_default() {
        let s = CredentialStore::new();
        let scheme = s.get("docker.io");
        assert!(matches!(scheme, AuthScheme::Oauth2 { .. }));
    }

    #[test]
    fn credential_store_remove() {
        let s = CredentialStore::new();
        s.set(
            "x",
            AuthScheme::Basic {
                username: "u".into(),
                password: "p".into(),
            },
        );
        assert_eq!(s.len(), 1);
        let removed = s.remove("x").unwrap();
        assert!(matches!(removed, AuthScheme::Basic { .. }));
        assert!(s.is_empty());
    }

    // ── base64 ────────────────────────────────────────────────────────────────

    #[test]
    fn base64_handles_each_padding_case() {
        // 0 padding
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        // 1 padding
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        // wait, that's 2 padding. Let me re-do:
        // "foo" len 3 → 0 padding → "Zm9v"
        // "fo" len 2 → 1 padding → "Zm8="
        // "f" len 1 → 2 padding → "Zg=="
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"f"), "Zg==");
    }

    #[test]
    fn base64_empty_input_is_empty_output() {
        assert_eq!(base64_encode(b""), "");
    }

    // ── Serde roundtrips ──────────────────────────────────────────────────────

    #[test]
    fn auth_scheme_serializes_with_tag() {
        let s = AuthScheme::Bearer { token: "x".into() };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"scheme\":\"Bearer\""));
        let back: AuthScheme = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn auth_scheme_aws_roundtrip() {
        let s = AuthScheme::AwsEcr {
            access_key_id: "AKIA".into(),
            secret_access_key: "x".into(),
            region: "eu-central-1".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AuthScheme = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
