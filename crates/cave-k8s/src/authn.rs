// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Authentication — ServiceAccount token, X.509 client cert, OIDC, and
//! bootstrap-token authenticators.
//!
//! Mirrors `pkg/serviceaccount` + `staging/src/k8s.io/apiserver/pkg/authentication`
//! at the umbrella level: each authenticator returns `Some(Identity)` if
//! the credential resolves; the chain returns the first hit.

use crate::error::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub user: String,
    pub groups: Vec<String>,
    pub uid: Option<String>,
    /// Provenance tag — which authenticator returned this identity.
    pub source: &'static str,
}

pub trait Authenticator: Send + Sync {
    fn name(&self) -> &'static str;
    fn authenticate(&self, token: &str) -> Option<Identity>;
}

pub struct ChainAuthenticator {
    authenticators: Vec<Box<dyn Authenticator>>,
}

impl Default for ChainAuthenticator {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainAuthenticator {
    pub fn new() -> Self {
        Self {
            authenticators: Vec::new(),
        }
    }

    pub fn add(mut self, a: Box<dyn Authenticator>) -> Self {
        self.authenticators.push(a);
        self
    }

    pub fn authenticate(&self, token: &str) -> Result<Identity, Error> {
        for a in &self.authenticators {
            if let Some(id) = a.authenticate(token) {
                return Ok(id);
            }
        }
        Err(Error::Unauthenticated("no authenticator matched".into()))
    }

    pub fn len(&self) -> usize {
        self.authenticators.len()
    }
    pub fn is_empty(&self) -> bool {
        self.authenticators.is_empty()
    }
}

// ─── ServiceAccount token (cave-k8s hybrid PQC) ─────────────────────────────

pub struct ServiceAccountAuthenticator {
    pub issuer: String,
    /// Map of `jti` -> (sa, namespace) — populated by the SA controller
    /// in cave-controller-manager when it mints tokens.
    pub known: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, (String, String)>>>,
}

impl ServiceAccountAuthenticator {
    pub fn new(issuer: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            known: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Record a freshly-minted token jti so subsequent authentications
    /// resolve it.
    pub fn register(&self, jti: impl Into<String>, namespace: impl Into<String>, sa: impl Into<String>) {
        self.known
            .write()
            .expect("sa lock")
            .insert(jti.into(), (sa.into(), namespace.into()));
    }
}

impl Authenticator for ServiceAccountAuthenticator {
    fn name(&self) -> &'static str {
        "ServiceAccountToken"
    }
    fn authenticate(&self, token: &str) -> Option<Identity> {
        // Token format used by `pqc::sign_sa_jwt` — three dot-separated parts.
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let payload =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[1])
                .ok()?;
        let claims: crate::pqc::SaClaims = serde_json::from_slice(&payload).ok()?;
        if claims.iss != self.issuer {
            return None;
        }
        let lookup = self.known.read().expect("sa lock");
        let (sa, namespace) = lookup.get(&claims.jti)?;
        Some(Identity {
            user: format!("system:serviceaccount:{}:{}", namespace, sa),
            groups: vec![
                "system:serviceaccounts".into(),
                format!("system:serviceaccounts:{}", namespace),
            ],
            uid: Some(claims.jti.clone()),
            source: "ServiceAccountToken",
        })
    }
}

// ─── X.509 client cert ──────────────────────────────────────────────────────

pub struct X509ClientCertAuthenticator {
    /// `(CN, [groups])` registry — populated by the cert-manager
    /// controller as client certs are issued/renewed.
    pub registry: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, Vec<String>>>>,
}

impl Default for X509ClientCertAuthenticator {
    fn default() -> Self {
        Self {
            registry: std::sync::Arc::new(std::sync::RwLock::new(Default::default())),
        }
    }
}

impl X509ClientCertAuthenticator {
    pub fn add_cn(&self, cn: impl Into<String>, groups: Vec<String>) {
        self.registry.write().expect("x509 lock").insert(cn.into(), groups);
    }
}

impl Authenticator for X509ClientCertAuthenticator {
    fn name(&self) -> &'static str {
        "X509ClientCert"
    }
    fn authenticate(&self, token: &str) -> Option<Identity> {
        // Convention used by cave-k8s/cavectl tooling: client-cert
        // bearer carries `x509://<CN>` as the bearer body.
        let cn = token.strip_prefix("x509://")?;
        let reg = self.registry.read().expect("x509 lock");
        let groups = reg.get(cn)?.clone();
        Some(Identity {
            user: cn.to_string(),
            groups,
            uid: None,
            source: "X509ClientCert",
        })
    }
}

// ─── OIDC bearer ─────────────────────────────────────────────────────────────

pub struct OidcAuthenticator {
    pub issuer: String,
    pub client_id: String,
    /// Pre-seeded `(token -> (user, groups))` lookup populated by the
    /// integration shim that talks to the real OIDC provider.
    pub cache: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, (String, Vec<String>)>>>,
}

impl OidcAuthenticator {
    pub fn new(issuer: impl Into<String>, client_id: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            cache: std::sync::Arc::new(std::sync::RwLock::new(Default::default())),
        }
    }
    pub fn seed(&self, token: impl Into<String>, user: impl Into<String>, groups: Vec<String>) {
        self.cache
            .write()
            .expect("oidc lock")
            .insert(token.into(), (user.into(), groups));
    }
}

impl Authenticator for OidcAuthenticator {
    fn name(&self) -> &'static str {
        "OIDC"
    }
    fn authenticate(&self, token: &str) -> Option<Identity> {
        let c = self.cache.read().expect("oidc lock");
        let (user, groups) = c.get(token)?.clone();
        Some(Identity {
            user,
            groups,
            uid: None,
            source: "OIDC",
        })
    }
}

// ─── Bootstrap tokens ────────────────────────────────────────────────────────

pub struct BootstrapTokenAuthenticator {
    pub tokens: std::sync::Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
}

impl Default for BootstrapTokenAuthenticator {
    fn default() -> Self {
        Self {
            tokens: std::sync::Arc::new(std::sync::RwLock::new(Default::default())),
        }
    }
}

impl BootstrapTokenAuthenticator {
    pub fn add(&self, token: impl Into<String>) {
        self.tokens.write().expect("boot lock").insert(token.into());
    }
}

impl Authenticator for BootstrapTokenAuthenticator {
    fn name(&self) -> &'static str {
        "BootstrapToken"
    }
    fn authenticate(&self, token: &str) -> Option<Identity> {
        if !self.tokens.read().expect("boot lock").contains(token) {
            return None;
        }
        Some(Identity {
            user: "system:bootstrap".into(),
            groups: vec!["system:bootstrappers".into(), "system:bootstrappers:kubeadm:default-node-token".into()],
            uid: None,
            source: "BootstrapToken",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chain_returns_unauthenticated() {
        let c = ChainAuthenticator::new();
        let e = c.authenticate("x").unwrap_err();
        assert!(matches!(e, Error::Unauthenticated(_)));
    }

    #[test]
    fn x509_authenticator_resolves_cn() {
        let a = X509ClientCertAuthenticator::default();
        a.add_cn("alice", vec!["system:masters".into()]);
        let id = a.authenticate("x509://alice").unwrap();
        assert_eq!(id.user, "alice");
        assert_eq!(id.source, "X509ClientCert");
        assert!(a.authenticate("x509://bob").is_none());
        assert!(a.authenticate("alice").is_none());
    }

    #[test]
    fn oidc_seeded_token_resolves() {
        let o = OidcAuthenticator::new("https://oidc.cave.svc", "kube-apiserver");
        o.seed("bearer-xyz", "alice@example.com", vec!["devs".into()]);
        let id = o.authenticate("bearer-xyz").unwrap();
        assert_eq!(id.user, "alice@example.com");
        assert_eq!(id.source, "OIDC");
        assert!(o.authenticate("nope").is_none());
    }

    #[test]
    fn bootstrap_token_resolves_to_bootstrap_user() {
        let b = BootstrapTokenAuthenticator::default();
        b.add("abcdef.0123456789abcdef");
        let id = b.authenticate("abcdef.0123456789abcdef").unwrap();
        assert_eq!(id.user, "system:bootstrap");
        assert!(id.groups.iter().any(|g| g == "system:bootstrappers"));
    }

    #[test]
    fn sa_authenticator_round_trips_with_pqc_jwt() {
        let signer = crate::pqc::HybridSigner::from_seed([42u8; 32]);
        let claims = crate::pqc::SaClaims {
            iss: "cave-k8s".into(),
            sub: "system:serviceaccount:default:cave".into(),
            aud: vec!["kube-apiserver".into()],
            exp: 9_999_999_999,
            iat: 1,
            jti: "tok-1".into(),
        };
        let tok = crate::pqc::sign_sa_jwt(&signer, &claims);
        let a = ServiceAccountAuthenticator::new("cave-k8s");
        a.register("tok-1", "default", "cave");
        let id = a.authenticate(&tok).unwrap();
        assert_eq!(id.user, "system:serviceaccount:default:cave");
        assert!(id.groups.iter().any(|g| g == "system:serviceaccounts"));
        assert_eq!(id.uid.as_deref(), Some("tok-1"));
    }

    #[test]
    fn sa_authenticator_rejects_unknown_jti() {
        let signer = crate::pqc::HybridSigner::from_seed([43u8; 32]);
        let claims = crate::pqc::SaClaims {
            iss: "cave-k8s".into(),
            sub: "x".into(),
            aud: vec![],
            exp: 1,
            iat: 1,
            jti: "never-registered".into(),
        };
        let tok = crate::pqc::sign_sa_jwt(&signer, &claims);
        let a = ServiceAccountAuthenticator::new("cave-k8s");
        assert!(a.authenticate(&tok).is_none());
    }

    #[test]
    fn sa_authenticator_rejects_wrong_issuer() {
        let signer = crate::pqc::HybridSigner::from_seed([44u8; 32]);
        let claims = crate::pqc::SaClaims {
            iss: "other-issuer".into(),
            sub: "x".into(),
            aud: vec![],
            exp: 1,
            iat: 1,
            jti: "tok-x".into(),
        };
        let tok = crate::pqc::sign_sa_jwt(&signer, &claims);
        let a = ServiceAccountAuthenticator::new("cave-k8s");
        a.register("tok-x", "default", "x");
        assert!(a.authenticate(&tok).is_none());
    }

    #[test]
    fn chain_returns_first_match() {
        let c = ChainAuthenticator::new()
            .add(Box::new(BootstrapTokenAuthenticator::default()))
            .add({
                let a = X509ClientCertAuthenticator::default();
                a.add_cn("alice", vec!["devs".into()]);
                Box::new(a)
            });
        let id = c.authenticate("x509://alice").unwrap();
        assert_eq!(id.source, "X509ClientCert");
    }
}
