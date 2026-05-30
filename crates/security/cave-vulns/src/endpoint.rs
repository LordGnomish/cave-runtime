// SPDX-License-Identifier: AGPL-3.0-or-later
//! DefectDojo-parity Endpoint model — RFC-3986 URI decomposition,
//! default-port normalization, and per-finding Endpoint_Status tracking.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738
//!         dojo/models.py (`class Endpoint`, `class Endpoint_Status`)
//!         and dojo/endpoint/utils.py (`SCHEME_PORT_MAP`, `endpoint_filter`).
//!
//! Upstream parses with the `hyperlink` library; we hand-roll a
//! dependency-free RFC-3986 split into the same 7 components
//! (protocol / userinfo / host / port / path / query / fragment) and
//! mirror the `clean()` normalization: scheme-default ports collapse to
//! `None`, paths are stored root-less (leading slashes stripped), and
//! empty path/query/fragment become `None`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Errors raised while parsing a URI into an [`Endpoint`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EndpointError {
    /// `host` is the only RFC-3986 component DefectDojo marks non-null.
    #[error("endpoint host is required")]
    MissingHost,
    /// Port present but outside the valid 0..=65535 range / non-numeric.
    #[error("invalid port: {0}")]
    InvalidPort(String),
}

/// Scheme → default port, mirroring DefectDojo's `SCHEME_PORT_MAP`
/// (the subset the importer actually normalizes against).
pub fn scheme_default_port(scheme: &str) -> Option<u16> {
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        "ftps" => Some(990),
        "ssh" | "sftp" => Some(22),
        "telnet" => Some(23),
        "smtp" => Some(25),
        "dns" => Some(53),
        "ldap" => Some(389),
        "ldaps" => Some(636),
        _ => None,
    }
}

/// A DefectDojo-parity Endpoint: the seven RFC-3986 components plus an
/// optional owning product. Equality / hashing fold over every field, so
/// (after `from_uri` normalization) two endpoints compare equal iff their
/// canonical URL string and product match — same contract as upstream's
/// `__eq__`/`__hash__` over `str(self)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct Endpoint {
    pub protocol: Option<String>,
    pub userinfo: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub fragment: Option<String>,
    pub product_id: Option<Uuid>,
}

impl Endpoint {
    /// Parse a URI string into its components, applying DefectDojo's
    /// `clean()` normalization. Mirrors `Endpoint.from_uri`.
    pub fn from_uri(_uri: &str) -> Result<Self, EndpointError> {
        Err(EndpointError::MissingHost) // stub — implemented in GREEN commit
    }

    /// The port that actually applies: the explicit port if stored,
    /// otherwise the scheme default. Mirrors upstream port inference.
    pub fn effective_port(&self) -> Option<u16> {
        None // stub
    }

    /// `True` when the endpoint can never resolve (no host).
    pub fn is_broken(&self) -> bool {
        true // stub
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.host) // stub
    }
}

/// Per-(finding, endpoint) triage state. Mirrors `class Endpoint_Status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointStatus {
    pub endpoint_id: Uuid,
    pub finding_id: Uuid,
    pub date: DateTime<Utc>,
    pub last_modified: DateTime<Utc>,
    pub mitigated: bool,
    pub mitigated_time: Option<DateTime<Utc>>,
    pub mitigated_by: Option<String>,
    pub false_positive: bool,
    pub out_of_scope: bool,
    pub risk_accepted: bool,
}

impl EndpointStatus {
    pub fn new(_endpoint_id: Uuid, _finding_id: Uuid) -> Self {
        // stub — implemented in GREEN commit
        Self {
            endpoint_id: Uuid::nil(),
            finding_id: Uuid::nil(),
            date: Utc::now(),
            last_modified: Utc::now(),
            mitigated: false,
            mitigated_time: None,
            mitigated_by: None,
            false_positive: false,
            out_of_scope: false,
            risk_accepted: false,
        }
    }

    /// Mark mitigated by `actor`, stamping `mitigated_time`/`last_modified`.
    pub fn mitigate(&mut self, _actor: &str) {
        // stub
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_http_with_inferred_default_port() {
        let e = Endpoint::from_uri("http://example.com").unwrap();
        assert_eq!(e.protocol.as_deref(), Some("http"));
        assert_eq!(e.userinfo, None);
        assert_eq!(e.host, "example.com");
        // Default port collapses to None (matches scheme default)…
        assert_eq!(e.port, None);
        // …but the effective port infers 80.
        assert_eq!(e.effective_port(), Some(80));
        assert_eq!(e.path, None);
        assert_eq!(e.query, None);
        assert_eq!(e.fragment, None);
    }

    #[test]
    fn parses_full_https_with_all_components() {
        let e =
            Endpoint::from_uri("https://api.example.com:8443/v1/users?filter=active#section1")
                .unwrap();
        assert_eq!(e.protocol.as_deref(), Some("https"));
        assert_eq!(e.host, "api.example.com");
        assert_eq!(e.port, Some(8443));
        assert_eq!(e.path.as_deref(), Some("v1/users"));
        assert_eq!(e.query.as_deref(), Some("filter=active"));
        assert_eq!(e.fragment.as_deref(), Some("section1"));
        assert_eq!(e.userinfo, None);
    }

    #[test]
    fn parses_userinfo_and_null_protocol() {
        let e = Endpoint::from_uri("//user:pass@localhost:3000/admin").unwrap();
        assert_eq!(e.protocol, None);
        assert_eq!(e.userinfo.as_deref(), Some("user:pass"));
        assert_eq!(e.host, "localhost");
        assert_eq!(e.port, Some(3000));
        assert_eq!(e.path.as_deref(), Some("admin"));
    }

    #[test]
    fn infers_ftp_default_port() {
        let e = Endpoint::from_uri("ftp://ftp.example.org/pub/file.txt?type=a").unwrap();
        assert_eq!(e.protocol.as_deref(), Some("ftp"));
        assert_eq!(e.host, "ftp.example.org");
        assert_eq!(e.port, None);
        assert_eq!(e.effective_port(), Some(21));
        assert_eq!(e.path.as_deref(), Some("pub/file.txt"));
        assert_eq!(e.query.as_deref(), Some("type=a"));
    }

    #[test]
    fn parses_ipv4_host() {
        let e = Endpoint::from_uri("https://192.168.1.1").unwrap();
        assert_eq!(e.host, "192.168.1.1");
        assert_eq!(e.effective_port(), Some(443));
        assert_eq!(e.path, None);
    }

    #[test]
    fn parses_ipv6_host_with_explicit_port() {
        let e = Endpoint::from_uri("https://[::1]:8080/path").unwrap();
        assert_eq!(e.host, "::1");
        assert_eq!(e.port, Some(8080));
        assert_eq!(e.path.as_deref(), Some("path"));
    }

    #[test]
    fn explicit_default_port_collapses_to_none() {
        // http + :80 → port stored null (matches scheme default).
        let e = Endpoint::from_uri("http://example.com:80/").unwrap();
        assert_eq!(e.port, None);
        assert_eq!(e.effective_port(), Some(80));
    }

    #[test]
    fn preserves_valueless_query_params() {
        let e = Endpoint::from_uri("https://example.com:443/path?a=1&b&c=3").unwrap();
        // :443 is https default → collapses.
        assert_eq!(e.port, None);
        assert_eq!(e.query.as_deref(), Some("a=1&b&c=3"));
    }

    #[test]
    fn missing_host_is_error() {
        assert_eq!(Endpoint::from_uri("http://"), Err(EndpointError::MissingHost));
    }

    #[test]
    fn out_of_range_port_is_error() {
        assert!(matches!(
            Endpoint::from_uri("http://example.com:99999"),
            Err(EndpointError::InvalidPort(_))
        ));
    }

    #[test]
    fn leading_slashes_stripped_from_path() {
        let e = Endpoint::from_uri("https://example.com///deep/path").unwrap();
        assert_eq!(e.path.as_deref(), Some("deep/path"));
    }

    #[test]
    fn display_roundtrips_canonical_url() {
        let e =
            Endpoint::from_uri("https://api.example.com:8443/v1/users?filter=active#section1")
                .unwrap();
        assert_eq!(
            e.to_string(),
            "https://api.example.com:8443/v1/users?filter=active#section1"
        );
    }

    #[test]
    fn display_omits_default_port() {
        let e = Endpoint::from_uri("http://example.com:80/api").unwrap();
        assert_eq!(e.to_string(), "http://example.com/api");
    }

    #[test]
    fn display_brackets_ipv6() {
        let e = Endpoint::from_uri("https://[::1]:8080/path").unwrap();
        assert_eq!(e.to_string(), "https://[::1]:8080/path");
    }

    #[test]
    fn equality_matches_on_canonical_form() {
        // :80 collapses, trailing slash on path normalizes away → equal.
        let a = Endpoint::from_uri("http://example.com:80/").unwrap();
        let b = Endpoint::from_uri("http://example.com").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn equality_differs_on_product_scope() {
        let mut a = Endpoint::from_uri("http://example.com").unwrap();
        let b = Endpoint::from_uri("http://example.com").unwrap();
        a.product_id = Some(Uuid::new_v4());
        assert_ne!(a, b);
    }

    #[test]
    fn usable_in_hashset_for_dedup() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Endpoint::from_uri("http://example.com:80/").unwrap());
        set.insert(Endpoint::from_uri("http://example.com").unwrap());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn is_broken_false_for_valid_host() {
        let e = Endpoint::from_uri("http://example.com").unwrap();
        assert!(!e.is_broken());
    }

    #[test]
    fn status_new_defaults_to_open() {
        let ep = Uuid::new_v4();
        let f = Uuid::new_v4();
        let s = EndpointStatus::new(ep, f);
        assert_eq!(s.endpoint_id, ep);
        assert_eq!(s.finding_id, f);
        assert!(!s.mitigated);
        assert!(!s.false_positive);
        assert!(!s.out_of_scope);
        assert!(!s.risk_accepted);
        assert_eq!(s.mitigated_time, None);
        assert_eq!(s.mitigated_by, None);
    }

    #[test]
    fn status_mitigate_stamps_time_and_actor() {
        let s_date;
        let mut s = EndpointStatus::new(Uuid::new_v4(), Uuid::new_v4());
        s_date = s.last_modified;
        s.mitigate("alice");
        assert!(s.mitigated);
        assert_eq!(s.mitigated_by.as_deref(), Some("alice"));
        assert!(s.mitigated_time.is_some());
        assert!(s.last_modified >= s_date);
    }
}
