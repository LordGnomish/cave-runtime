//! SPIFFE / SVID — RFC 9712 + SPIFFE-ID specification parity.
//!
//! Validates SPIFFE IDs, parses URI-SAN-encoded x509-SVIDs, manages trust
//! bundles per trust domain, and decides whether an inbound mTLS handshake
//! should be authorized against a configured trust domain set.
//!
//! Out of scope here: actual x509 certificate parsing (lives in `tls.rs`
//! when feature-gated). This module's input is the URI string carried in
//! the SAN extension — already extracted by rustls' client-cert verifier
//! callback in `tls.rs`.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

/// A parsed `spiffe://<trust-domain>/<path>` ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpiffeId {
    pub trust_domain: String,
    pub path: String,
}

impl SpiffeId {
    /// Parse a SPIFFE ID URI. Per spec:
    ///
    /// - Scheme MUST be `spiffe`.
    /// - Trust domain is the host component, lowercase, no port, no userinfo,
    ///   non-empty, max 255 characters, only `a-z 0-9 - . _`.
    /// - Path is `/`-separated; segments must be non-empty;
    ///   characters limited per the spec.
    pub fn parse(uri: &str) -> Result<SpiffeId, SpiffeError> {
        let rest = uri
            .strip_prefix("spiffe://")
            .ok_or(SpiffeError::WrongScheme)?;
        if rest.is_empty() {
            return Err(SpiffeError::EmptyTrustDomain);
        }
        // Reject userinfo / port / fragment / query.
        if rest.contains('@') {
            return Err(SpiffeError::UserInfoNotAllowed);
        }
        if let Some(_) = rest.find('?') {
            return Err(SpiffeError::QueryNotAllowed);
        }
        if let Some(_) = rest.find('#') {
            return Err(SpiffeError::FragmentNotAllowed);
        }

        // Split host and path on first '/'.
        let (td, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };

        if td.contains(':') {
            return Err(SpiffeError::PortNotAllowed);
        }
        if td.is_empty() {
            return Err(SpiffeError::EmptyTrustDomain);
        }
        if td.len() > 255 {
            return Err(SpiffeError::TrustDomainTooLong);
        }
        validate_trust_domain_charset(td)?;

        if !path.is_empty() {
            validate_path(path)?;
        }

        Ok(SpiffeId {
            trust_domain: td.to_ascii_lowercase(),
            path: path.to_string(),
        })
    }

    /// Reconstruct the canonical URI (lowercase trust domain).
    pub fn as_uri(&self) -> String {
        format!("spiffe://{}{}", self.trust_domain, self.path)
    }

    pub fn first_segment(&self) -> Option<&str> {
        self.path
            .strip_prefix('/')
            .and_then(|p| p.split('/').next())
            .filter(|s| !s.is_empty())
    }

    pub fn segments(&self) -> Vec<&str> {
        self.path
            .strip_prefix('/')
            .map(|p| p.split('/').collect())
            .unwrap_or_default()
    }
}

fn validate_trust_domain_charset(td: &str) -> Result<(), SpiffeError> {
    for c in td.chars() {
        if !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') {
            return Err(SpiffeError::InvalidTrustDomainChar(c));
        }
        if c.is_ascii_uppercase() {
            return Err(SpiffeError::TrustDomainNotLowercase);
        }
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), SpiffeError> {
    if !path.starts_with('/') {
        return Err(SpiffeError::PathMustBeAbsolute);
    }
    let segments: Vec<&str> = path.strip_prefix('/').unwrap_or("").split('/').collect();
    for s in &segments {
        if s.is_empty() {
            return Err(SpiffeError::EmptyPathSegment);
        }
        if *s == "." || *s == ".." {
            return Err(SpiffeError::DotPathSegment);
        }
        for c in s.chars() {
            if c.is_ascii_alphanumeric() {
                continue;
            }
            if matches!(c, '.' | '-' | '_' | '~' | '%') {
                continue;
            }
            return Err(SpiffeError::InvalidPathChar(c));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SpiffeError {
    #[error("scheme must be spiffe://")]
    WrongScheme,
    #[error("trust domain is empty")]
    EmptyTrustDomain,
    #[error("trust domain longer than 255 characters")]
    TrustDomainTooLong,
    #[error("port not allowed in SPIFFE URI")]
    PortNotAllowed,
    #[error("userinfo not allowed in SPIFFE URI")]
    UserInfoNotAllowed,
    #[error("query string not allowed in SPIFFE URI")]
    QueryNotAllowed,
    #[error("fragment not allowed in SPIFFE URI")]
    FragmentNotAllowed,
    #[error("trust domain must be lowercase")]
    TrustDomainNotLowercase,
    #[error("invalid character in trust domain: {0:?}")]
    InvalidTrustDomainChar(char),
    #[error("path must start with /")]
    PathMustBeAbsolute,
    #[error("empty path segment")]
    EmptyPathSegment,
    #[error("'.' or '..' path segment")]
    DotPathSegment,
    #[error("invalid character in path: {0:?}")]
    InvalidPathChar(char),
    #[error("trust domain {0} not in allow-list")]
    TrustDomainNotTrusted(String),
    #[error("ID matches deny rule: {0}")]
    DenyRuleMatched(String),
    #[error("federation bundle not found for {0}")]
    NoFederationBundle(String),
}

// ── Trust bundle ────────────────────────────────────────────────────────────

/// SPIFFE trust bundle — one per trust domain. Production code stores PEM
/// roots; here we keep the bundle envelope (sequence number, refresh hint,
/// issuer set as opaque bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustBundle {
    pub trust_domain: String,
    pub sequence_number: u64,
    pub refresh_hint_seconds: u64,
    /// PEM-encoded root CAs trusted in this domain.
    pub roots: Vec<String>,
    /// Federated trust domains we accept SVIDs from.
    pub federates_with: Vec<String>,
}

impl TrustBundle {
    pub fn new(trust_domain: impl Into<String>) -> Self {
        Self {
            trust_domain: trust_domain.into(),
            sequence_number: 0,
            refresh_hint_seconds: 600,
            roots: vec![],
            federates_with: vec![],
        }
    }

    pub fn with_root(mut self, pem: impl Into<String>) -> Self {
        self.roots.push(pem.into());
        self
    }

    pub fn validate(&self) -> Result<(), SpiffeError> {
        if self.trust_domain.is_empty() {
            return Err(SpiffeError::EmptyTrustDomain);
        }
        validate_trust_domain_charset(&self.trust_domain)?;
        Ok(())
    }
}

// ── Trust-domain allow/deny policy ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SvidAuthorizationPolicy {
    /// Trust domains accepted at the listener edge.
    pub allowed_trust_domains: Vec<String>,
    /// Path prefixes that, even within an allowed domain, are denied.
    /// Useful for blocking known-stale or compromised workload IDs.
    pub denied_path_prefixes: Vec<String>,
    /// If non-empty, only paths matching one of these prefixes are allowed.
    pub allowed_path_prefixes: Vec<String>,
    /// Require federation bundle entry for any cross-domain SVID.
    pub require_federation_bundle: bool,
}

impl Default for SvidAuthorizationPolicy {
    fn default() -> Self {
        Self {
            allowed_trust_domains: vec![],
            denied_path_prefixes: vec![],
            allowed_path_prefixes: vec![],
            require_federation_bundle: true,
        }
    }
}

impl SvidAuthorizationPolicy {
    pub fn validate(&self) -> Result<(), SpiffeError> {
        for td in &self.allowed_trust_domains {
            validate_trust_domain_charset(td)?;
        }
        for p in self.denied_path_prefixes.iter().chain(&self.allowed_path_prefixes) {
            if !p.starts_with('/') {
                return Err(SpiffeError::PathMustBeAbsolute);
            }
        }
        Ok(())
    }
}

// ── Bundle registry + decision engine ───────────────────────────────────────

#[derive(Debug, Default)]
pub struct BundleRegistry {
    bundles: RwLock<BTreeMap<String, Arc<TrustBundle>>>,
}

impl BundleRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn upsert(&self, bundle: TrustBundle) -> Result<(), SpiffeError> {
        bundle.validate()?;
        self.bundles
            .write()
            .unwrap()
            .insert(bundle.trust_domain.clone(), Arc::new(bundle));
        Ok(())
    }

    pub fn get(&self, trust_domain: &str) -> Option<Arc<TrustBundle>> {
        self.bundles.read().unwrap().get(trust_domain).cloned()
    }

    pub fn len(&self) -> usize {
        self.bundles.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.bundles.read().unwrap().is_empty()
    }

    pub fn list_trust_domains(&self) -> Vec<String> {
        self.bundles.read().unwrap().keys().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    Allow,
    Deny(SpiffeError),
}

/// Decide whether to admit a peer presenting `id` against the policy and
/// the given local trust domain.
pub fn authorize(
    policy: &SvidAuthorizationPolicy,
    bundles: &BundleRegistry,
    local_domain: &str,
    id: &SpiffeId,
) -> AuthDecision {
    // 1. Trust-domain allow-list.
    let allowed: HashSet<&String> = policy.allowed_trust_domains.iter().collect();
    if !allowed.contains(&id.trust_domain) && id.trust_domain != local_domain {
        return AuthDecision::Deny(SpiffeError::TrustDomainNotTrusted(id.trust_domain.clone()));
    }

    // 2. If cross-domain and federation is required, ensure the bundle exists.
    if id.trust_domain != local_domain && policy.require_federation_bundle {
        let local_bundle = match bundles.get(local_domain) {
            Some(b) => b,
            None => {
                return AuthDecision::Deny(SpiffeError::NoFederationBundle(local_domain.into()))
            }
        };
        if !local_bundle.federates_with.contains(&id.trust_domain) {
            return AuthDecision::Deny(SpiffeError::NoFederationBundle(id.trust_domain.clone()));
        }
    }

    // 3. Deny prefixes win.
    for d in &policy.denied_path_prefixes {
        if id.path.starts_with(d) {
            return AuthDecision::Deny(SpiffeError::DenyRuleMatched(d.clone()));
        }
    }

    // 4. Allow-prefix allowlist (if non-empty, ID must match one).
    if !policy.allowed_path_prefixes.is_empty() {
        let hit = policy
            .allowed_path_prefixes
            .iter()
            .any(|a| id.path.starts_with(a));
        if !hit {
            return AuthDecision::Deny(SpiffeError::DenyRuleMatched(
                "no allowed_path_prefixes match".into(),
            ));
        }
    }

    AuthDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn td(name: &str) -> TrustBundle {
        TrustBundle::new(name)
    }

    fn policy_allow(domains: Vec<&str>) -> SvidAuthorizationPolicy {
        SvidAuthorizationPolicy {
            allowed_trust_domains: domains.into_iter().map(String::from).collect(),
            ..Default::default()
        }
    }

    // ── Parse: success cases ──

    #[test]
    fn parse_root_only() {
        let id = SpiffeId::parse("spiffe://example.org").unwrap();
        assert_eq!(id.trust_domain, "example.org");
        assert!(id.path.is_empty());
    }

    #[test]
    fn parse_with_path() {
        let id = SpiffeId::parse("spiffe://example.org/workload/api-server").unwrap();
        assert_eq!(id.trust_domain, "example.org");
        assert_eq!(id.path, "/workload/api-server");
    }

    #[test]
    fn parse_uppercase_trust_domain_rejects() {
        // SPIFFE-ID spec §2.1: trust domain MUST be lowercase.
        assert_eq!(
            SpiffeId::parse("spiffe://Example.ORG/x").unwrap_err(),
            SpiffeError::TrustDomainNotLowercase
        );
    }

    #[test]
    fn parse_dashes_underscores_dots_in_td() {
        SpiffeId::parse("spiffe://my_zone.east-1.example.org").unwrap();
    }

    #[test]
    fn parse_segments_helper() {
        let id = SpiffeId::parse("spiffe://x.com/a/b/c").unwrap();
        assert_eq!(id.segments(), vec!["a", "b", "c"]);
        assert_eq!(id.first_segment(), Some("a"));
    }

    #[test]
    fn parse_first_segment_no_path() {
        let id = SpiffeId::parse("spiffe://x.com").unwrap();
        assert_eq!(id.first_segment(), None);
    }

    #[test]
    fn parse_url_encoded_path_chars() {
        // `%` is allowed (percent-encoded sequences aren't decoded by us).
        SpiffeId::parse("spiffe://x.com/a%20b").unwrap();
    }

    // ── Parse: failure cases ──

    #[test]
    fn parse_wrong_scheme_rejects() {
        assert_eq!(
            SpiffeId::parse("https://example.org/x").unwrap_err(),
            SpiffeError::WrongScheme
        );
    }

    #[test]
    fn parse_empty_td_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe:///x").unwrap_err(),
            SpiffeError::EmptyTrustDomain
        );
    }

    #[test]
    fn parse_no_td_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://").unwrap_err(),
            SpiffeError::EmptyTrustDomain
        );
    }

    #[test]
    fn parse_userinfo_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://user@x.com/a").unwrap_err(),
            SpiffeError::UserInfoNotAllowed
        );
    }

    #[test]
    fn parse_port_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://x.com:443/a").unwrap_err(),
            SpiffeError::PortNotAllowed
        );
    }

    #[test]
    fn parse_query_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://x.com/a?z=1").unwrap_err(),
            SpiffeError::QueryNotAllowed
        );
    }

    #[test]
    fn parse_fragment_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://x.com/a#frag").unwrap_err(),
            SpiffeError::FragmentNotAllowed
        );
    }

    #[test]
    fn parse_invalid_td_char_rejects() {
        assert!(matches!(
            SpiffeId::parse("spiffe://x!.com/a").unwrap_err(),
            SpiffeError::InvalidTrustDomainChar('!')
        ));
    }

    #[test]
    fn parse_double_slash_in_path_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://x.com/a//b").unwrap_err(),
            SpiffeError::EmptyPathSegment
        );
    }

    #[test]
    fn parse_dot_segment_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://x.com/./a").unwrap_err(),
            SpiffeError::DotPathSegment
        );
    }

    #[test]
    fn parse_dotdot_segment_rejects() {
        assert_eq!(
            SpiffeId::parse("spiffe://x.com/../a").unwrap_err(),
            SpiffeError::DotPathSegment
        );
    }

    #[test]
    fn parse_invalid_path_char_rejects() {
        assert!(matches!(
            SpiffeId::parse("spiffe://x.com/a/b!c").unwrap_err(),
            SpiffeError::InvalidPathChar('!')
        ));
    }

    #[test]
    fn parse_long_td_rejects() {
        let long = "a".repeat(256);
        let uri = format!("spiffe://{}/x", long);
        assert_eq!(
            SpiffeId::parse(&uri).unwrap_err(),
            SpiffeError::TrustDomainTooLong
        );
    }

    // ── as_uri ──

    #[test]
    fn as_uri_round_trip() {
        let id = SpiffeId::parse("spiffe://example.org/a/b").unwrap();
        let again = SpiffeId::parse(&id.as_uri()).unwrap();
        assert_eq!(id, again);
    }

    // ── TrustBundle ──

    #[test]
    fn bundle_validate_default() {
        td("example.org").validate().unwrap();
    }

    #[test]
    fn bundle_validate_empty_rejects() {
        let b = TrustBundle::new("");
        assert!(b.validate().is_err());
    }

    #[test]
    fn bundle_with_root() {
        let b = td("example.org").with_root("PEM-1").with_root("PEM-2");
        assert_eq!(b.roots.len(), 2);
    }

    // ── BundleRegistry ──

    #[test]
    fn registry_upsert_and_get() {
        let r = BundleRegistry::new();
        r.upsert(td("a.com")).unwrap();
        assert!(r.get("a.com").is_some());
        assert!(r.get("b.com").is_none());
    }

    #[test]
    fn registry_invalid_bundle_rejects() {
        let r = BundleRegistry::new();
        assert!(r.upsert(TrustBundle::new("")).is_err());
    }

    #[test]
    fn registry_list_trust_domains_sorted() {
        let r = BundleRegistry::new();
        r.upsert(td("z.com")).unwrap();
        r.upsert(td("a.com")).unwrap();
        let names = r.list_trust_domains();
        assert_eq!(names, vec!["a.com", "z.com"]);
    }

    // ── Policy ──

    #[test]
    fn policy_default_validates() {
        SvidAuthorizationPolicy::default().validate().unwrap();
    }

    #[test]
    fn policy_relative_prefix_rejects() {
        let mut p = SvidAuthorizationPolicy::default();
        p.allowed_path_prefixes = vec!["api".into()];
        assert!(p.validate().is_err());
    }

    // ── authorize ──

    #[test]
    fn authorize_local_td_passes() {
        let policy = policy_allow(vec![]);
        let bundles = BundleRegistry::new();
        let id = SpiffeId::parse("spiffe://local.zone/api").unwrap();
        assert_eq!(
            authorize(&policy, &bundles, "local.zone", &id),
            AuthDecision::Allow
        );
    }

    #[test]
    fn authorize_unknown_td_denied() {
        let policy = policy_allow(vec!["other.zone"]);
        let bundles = BundleRegistry::new();
        let id = SpiffeId::parse("spiffe://hacker.zone/api").unwrap();
        match authorize(&policy, &bundles, "local.zone", &id) {
            AuthDecision::Deny(SpiffeError::TrustDomainNotTrusted(t)) => {
                assert_eq!(t, "hacker.zone");
            }
            other => panic!("expected deny, got {:?}", other),
        }
    }

    #[test]
    fn authorize_cross_td_requires_federation_bundle() {
        let policy = policy_allow(vec!["partner.org"]);
        let bundles = BundleRegistry::new();
        let mut local = td("local.zone");
        local.federates_with = vec![]; // no federation
        bundles.upsert(local).unwrap();
        let id = SpiffeId::parse("spiffe://partner.org/api").unwrap();
        match authorize(&policy, &bundles, "local.zone", &id) {
            AuthDecision::Deny(SpiffeError::NoFederationBundle(_)) => {}
            other => panic!("expected federation deny, got {:?}", other),
        }
    }

    #[test]
    fn authorize_cross_td_allowed_with_federation() {
        let policy = policy_allow(vec!["partner.org"]);
        let bundles = BundleRegistry::new();
        let mut local = td("local.zone");
        local.federates_with = vec!["partner.org".into()];
        bundles.upsert(local).unwrap();
        let id = SpiffeId::parse("spiffe://partner.org/api").unwrap();
        assert_eq!(
            authorize(&policy, &bundles, "local.zone", &id),
            AuthDecision::Allow
        );
    }

    #[test]
    fn authorize_deny_prefix_blocks() {
        let mut policy = policy_allow(vec![]);
        policy.denied_path_prefixes = vec!["/admin".into()];
        let bundles = BundleRegistry::new();
        let id = SpiffeId::parse("spiffe://local.zone/admin/x").unwrap();
        match authorize(&policy, &bundles, "local.zone", &id) {
            AuthDecision::Deny(SpiffeError::DenyRuleMatched(_)) => {}
            other => panic!("expected deny rule, got {:?}", other),
        }
    }

    #[test]
    fn authorize_allow_prefix_required() {
        let mut policy = policy_allow(vec![]);
        policy.allowed_path_prefixes = vec!["/api".into()];
        let bundles = BundleRegistry::new();
        let id_in = SpiffeId::parse("spiffe://local.zone/api/x").unwrap();
        let id_out = SpiffeId::parse("spiffe://local.zone/internal/x").unwrap();
        assert_eq!(
            authorize(&policy, &bundles, "local.zone", &id_in),
            AuthDecision::Allow
        );
        assert!(matches!(
            authorize(&policy, &bundles, "local.zone", &id_out),
            AuthDecision::Deny(_)
        ));
    }

    #[test]
    fn authorize_no_local_bundle_when_required() {
        let policy = policy_allow(vec!["partner.org"]);
        let bundles = BundleRegistry::new();
        let id = SpiffeId::parse("spiffe://partner.org/api").unwrap();
        match authorize(&policy, &bundles, "local.zone", &id) {
            AuthDecision::Deny(SpiffeError::NoFederationBundle(_)) => {}
            other => panic!("expected deny, got {:?}", other),
        }
    }

    // ── Serde ──

    #[test]
    fn trust_bundle_json_roundtrip() {
        let b = td("example.org").with_root("PEM");
        let s = serde_json::to_string(&b).unwrap();
        let b2: TrustBundle = serde_json::from_str(&s).unwrap();
        assert_eq!(b, b2);
    }

    #[test]
    fn policy_json_roundtrip() {
        let mut p = policy_allow(vec!["x.com"]);
        p.denied_path_prefixes = vec!["/admin".into()];
        let s = serde_json::to_string(&p).unwrap();
        let p2: SvidAuthorizationPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, p2);
    }
}
