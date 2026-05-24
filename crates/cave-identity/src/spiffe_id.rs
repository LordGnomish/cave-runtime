// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0); algorithm + character set
// line-ported from spiffe/go-spiffe/v2/spiffeid + pkg/common/idutil/spiffeid.go.
//
//! SPIFFE ID parser + validator (SPIFFE-ID-RFC).
//!
//! A SPIFFE ID is a URI of the form `spiffe://<trust-domain>/<path>` where:
//! - `<trust-domain>` is a non-empty, lowercase hostname (a-z, 0-9, ., _, -)
//!   no longer than 255 octets.
//! - `<path>` is zero or more segments separated by `/`. Each segment is
//!   non-empty and contains only `a-z`, `A-Z`, `0-9`, `.`, `_`, `-`. The
//!   special segments `.` and `..` are forbidden. Trailing slash is forbidden.
//! - The full ID must be no longer than 2048 octets.

use crate::error::{IdentityError, Result};
use crate::models::{SpiffeId, TrustDomain};

const MAX_ID_LEN: usize = 2048;
const MAX_TD_LEN: usize = 255;

/// Parsed components of a SPIFFE ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSpiffeId {
    pub trust_domain: TrustDomain,
    pub path: String,
}

impl ParsedSpiffeId {
    pub fn as_spiffe_id(&self) -> SpiffeId {
        SpiffeId::new(format!("spiffe://{}{}", self.trust_domain.as_str(), self.path))
    }
}

/// Validates the trust-domain component.
///
/// Rules:
/// - non-empty
/// - length <= 255
/// - chars: a-z 0-9 . _ -
pub fn validate_trust_domain(td: &str) -> Result<()> {
    if td.is_empty() {
        return Err(IdentityError::InvalidTrustDomain("empty".into()));
    }
    if td.len() > MAX_TD_LEN {
        return Err(IdentityError::InvalidTrustDomain(format!(
            "length {} > {}",
            td.len(),
            MAX_TD_LEN
        )));
    }
    for ch in td.chars() {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '_' || ch == '-')
        {
            return Err(IdentityError::InvalidTrustDomain(format!(
                "illegal character {:?}",
                ch
            )));
        }
    }
    Ok(())
}

/// Validates a single path segment (between two `/`).
pub fn validate_path_segment(seg: &str) -> Result<()> {
    if seg.is_empty() {
        return Err(IdentityError::InvalidSpiffeId("empty segment".into()));
    }
    if seg == "." || seg == ".." {
        return Err(IdentityError::InvalidSpiffeId(format!(
            "reserved segment {:?}",
            seg
        )));
    }
    for ch in seg.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-') {
            return Err(IdentityError::InvalidSpiffeId(format!(
                "illegal segment char {:?}",
                ch
            )));
        }
    }
    Ok(())
}

/// Validates the path part (starts with `/` or empty).
pub fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    if !path.starts_with('/') {
        return Err(IdentityError::InvalidSpiffeId(
            "path must start with /".into(),
        ));
    }
    if path.ends_with('/') && path.len() > 1 {
        return Err(IdentityError::InvalidSpiffeId(
            "trailing slash forbidden".into(),
        ));
    }
    for seg in path.split('/').skip(1) {
        validate_path_segment(seg)?;
    }
    Ok(())
}

/// Parses a SPIFFE ID string into trust-domain + path.
pub fn parse_spiffe_id(s: &str) -> Result<ParsedSpiffeId> {
    if s.len() > MAX_ID_LEN {
        return Err(IdentityError::InvalidSpiffeId(format!(
            "length {} > {}",
            s.len(),
            MAX_ID_LEN
        )));
    }
    let rest = s
        .strip_prefix("spiffe://")
        .ok_or_else(|| IdentityError::InvalidSpiffeId("missing spiffe:// prefix".into()))?;
    if rest.is_empty() {
        return Err(IdentityError::InvalidSpiffeId("missing trust domain".into()));
    }
    let (td, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    validate_trust_domain(td)?;
    validate_path(path)?;
    Ok(ParsedSpiffeId {
        trust_domain: TrustDomain::new(td.to_string()),
        path: path.to_string(),
    })
}

/// Returns `true` when `child` is a descendant SPIFFE ID of `parent`.
pub fn is_descendant(parent: &SpiffeId, child: &SpiffeId) -> bool {
    let Ok(p) = parse_spiffe_id(parent.as_str()) else {
        return false;
    };
    let Ok(c) = parse_spiffe_id(child.as_str()) else {
        return false;
    };
    if p.trust_domain != c.trust_domain {
        return false;
    }
    if p.path.is_empty() {
        return !c.path.is_empty();
    }
    c.path.starts_with(&format!("{}/", p.path))
}

/// Returns `true` when the id is a valid trust-domain identity (no path).
pub fn is_trust_domain_id(id: &SpiffeId) -> bool {
    parse_spiffe_id(id.as_str())
        .map(|p| p.path.is_empty())
        .unwrap_or(false)
}

/// Constructs an SVID-ready identity for an agent attestor (`<td>/spire/agent/<type>/<id>`).
pub fn agent_id(td: &TrustDomain, attestor_type: &str, agent_uid: &str) -> Result<SpiffeId> {
    validate_trust_domain(td.as_str())?;
    let id = format!(
        "spiffe://{}/spire/agent/{}/{}",
        td.as_str(),
        attestor_type,
        agent_uid
    );
    parse_spiffe_id(&id)?;
    Ok(SpiffeId::new(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_id() {
        let p = parse_spiffe_id("spiffe://example.org/workload/foo").unwrap();
        assert_eq!(p.trust_domain.as_str(), "example.org");
        assert_eq!(p.path, "/workload/foo");
    }

    #[test]
    fn parses_td_only() {
        let p = parse_spiffe_id("spiffe://example.org").unwrap();
        assert_eq!(p.path, "");
        assert!(is_trust_domain_id(&SpiffeId::new("spiffe://example.org")));
    }

    #[test]
    fn rejects_missing_prefix() {
        assert!(parse_spiffe_id("https://example.org/foo").is_err());
    }

    #[test]
    fn rejects_empty_td() {
        assert!(parse_spiffe_id("spiffe:///foo").is_err());
    }

    #[test]
    fn rejects_dot_segments() {
        assert!(parse_spiffe_id("spiffe://example.org/./foo").is_err());
        assert!(parse_spiffe_id("spiffe://example.org/../foo").is_err());
    }

    #[test]
    fn rejects_trailing_slash() {
        assert!(parse_spiffe_id("spiffe://example.org/foo/").is_err());
    }

    #[test]
    fn rejects_uppercase_td() {
        assert!(parse_spiffe_id("spiffe://Example.org/foo").is_err());
    }

    #[test]
    fn rejects_too_long_id() {
        let big = format!("spiffe://example.org/{}", "a".repeat(MAX_ID_LEN));
        assert!(parse_spiffe_id(&big).is_err());
    }

    #[test]
    fn descendant_check() {
        let p = SpiffeId::new("spiffe://example.org/team");
        let c = SpiffeId::new("spiffe://example.org/team/svc");
        assert!(is_descendant(&p, &c));
        let other = SpiffeId::new("spiffe://example.org/other/svc");
        assert!(!is_descendant(&p, &other));
    }

    #[test]
    fn descendant_td_root() {
        let p = SpiffeId::new("spiffe://example.org");
        let c = SpiffeId::new("spiffe://example.org/any");
        assert!(is_descendant(&p, &c));
    }

    #[test]
    fn agent_id_compose() {
        let td = TrustDomain::new("example.org");
        let id = agent_id(&td, "k8s_psat", "node-1").unwrap();
        assert_eq!(id.as_str(), "spiffe://example.org/spire/agent/k8s_psat/node-1");
    }

    #[test]
    fn rejects_illegal_segment_char() {
        assert!(parse_spiffe_id("spiffe://example.org/foo bar").is_err());
        assert!(parse_spiffe_id("spiffe://example.org/foo%20bar").is_err());
    }

    #[test]
    fn rejects_illegal_td_char() {
        assert!(validate_trust_domain("foo bar").is_err());
        assert!(validate_trust_domain("FOO").is_err());
    }
}
