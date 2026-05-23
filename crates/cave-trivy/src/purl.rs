// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Pure-Rust subset of [purl-spec](https://github.com/package-url/purl-spec).
//!
//! Mirrors trivy's `pkg/purl/purl.go` for the formats cave-trivy emits:
//! `pkg:<type>/<namespace?>/<name>@<version>?<qualifiers>` with percent-
//! decoded fields. We do not implement the full RFC 3986 percent-encoding
//! roundtrip — only the fragments cave-trivy needs for OS + language pkg
//! cross-referencing and CycloneDX/SPDX emission.

use crate::error::{TrivyError, TrivyResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageUrl {
    pub r#type: String,
    pub namespace: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub qualifiers: Vec<(String, String)>,
    pub subpath: Option<String>,
}

impl PackageUrl {
    pub fn new(r#type: &str, name: &str, version: Option<&str>) -> Self {
        Self {
            r#type: r#type.into(),
            namespace: None,
            name: name.into(),
            version: version.map(|s| s.into()),
            qualifiers: Vec::new(),
            subpath: None,
        }
    }

    pub fn with_namespace(mut self, ns: &str) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn with_qualifier(mut self, k: &str, v: &str) -> Self {
        self.qualifiers.push((k.into(), v.into()));
        self
    }

    pub fn parse(s: &str) -> TrivyResult<Self> {
        let rest = s
            .strip_prefix("pkg:")
            .ok_or_else(|| TrivyError::parse("purl must start with pkg:"))?;

        let (head, subpath) = match rest.split_once('#') {
            Some((h, s)) => (h, Some(s.to_string())),
            None => (rest, None),
        };
        let (head, qual_part) = match head.split_once('?') {
            Some((h, q)) => (h, Some(q)),
            None => (head, None),
        };

        let (type_part, path) = head
            .split_once('/')
            .ok_or_else(|| TrivyError::parse("purl missing '/' after type"))?;
        if type_part.is_empty() {
            return Err(TrivyError::parse("purl type empty"));
        }

        let (name_ver, namespace) = match path.rsplit_once('/') {
            Some((ns, n)) if !ns.is_empty() => (n, Some(ns.replace('/', "/"))),
            _ => (path, None),
        };
        let (name, version) = match name_ver.split_once('@') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (name_ver.to_string(), None),
        };
        if name.is_empty() {
            return Err(TrivyError::parse("purl name empty"));
        }

        let mut qualifiers = Vec::new();
        if let Some(q) = qual_part {
            for kv in q.split('&') {
                if kv.is_empty() {
                    continue;
                }
                let (k, v) = kv
                    .split_once('=')
                    .ok_or_else(|| TrivyError::parse("purl qualifier missing '='"))?;
                qualifiers.push((k.to_string(), v.to_string()));
            }
        }

        Ok(Self {
            r#type: type_part.to_string(),
            namespace,
            name,
            version,
            qualifiers,
            subpath,
        })
    }

    /// Canonical lower-case re-serialise (no percent-encoding round-trip).
    pub fn to_string_canonical(&self) -> String {
        let mut out = String::from("pkg:");
        out.push_str(&self.r#type.to_ascii_lowercase());
        out.push('/');
        if let Some(ns) = &self.namespace {
            out.push_str(ns);
            out.push('/');
        }
        out.push_str(&self.name);
        if let Some(v) = &self.version {
            out.push('@');
            out.push_str(v);
        }
        if !self.qualifiers.is_empty() {
            out.push('?');
            for (i, (k, v)) in self.qualifiers.iter().enumerate() {
                if i > 0 {
                    out.push('&');
                }
                out.push_str(k);
                out.push('=');
                out.push_str(v);
            }
        }
        if let Some(sp) = &self.subpath {
            out.push('#');
            out.push_str(sp);
        }
        out
    }
}

/// Map cave-trivy ecosystem string → purl type.
/// Mirrors `pkg/purl/purl.go::Type`.
pub fn ecosystem_to_purl_type(eco: &str) -> &'static str {
    match eco.to_ascii_lowercase().as_str() {
        "npm" | "nodejs" => "npm",
        "pypi" | "pip" => "pypi",
        "gem" | "rubygems" => "gem",
        "go" | "gomodules" => "golang",
        "cargo" | "crates" => "cargo",
        "composer" | "packagist" => "composer",
        "maven" | "gradle" => "maven",
        "pubspec" | "pub" => "pub",
        "swift" => "swift",
        "mix" | "hex" => "hex",
        "alpine" => "apk",
        "debian" | "ubuntu" => "deb",
        "rhel" | "centos" | "rocky" | "alma" | "amazon" | "oracle" | "photon" | "mariner" => "rpm",
        "suse" | "opensuse" => "rpm",
        _ => "generic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let p = PackageUrl::parse("pkg:npm/lodash@4.17.21").unwrap();
        assert_eq!(p.r#type, "npm");
        assert_eq!(p.name, "lodash");
        assert_eq!(p.version.as_deref(), Some("4.17.21"));
        assert!(p.namespace.is_none());
    }

    #[test]
    fn parse_namespace() {
        let p = PackageUrl::parse("pkg:golang/github.com/foo/bar@v1.2.3").unwrap();
        assert_eq!(p.r#type, "golang");
        assert_eq!(p.name, "bar");
        assert_eq!(p.namespace.as_deref(), Some("github.com/foo"));
        assert_eq!(p.version.as_deref(), Some("v1.2.3"));
    }

    #[test]
    fn parse_qualifiers_subpath() {
        let p = PackageUrl::parse("pkg:apk/alpine/openssl@3.0?arch=x86_64#a/b").unwrap();
        assert_eq!(p.r#type, "apk");
        assert_eq!(p.namespace.as_deref(), Some("alpine"));
        assert_eq!(p.qualifiers, vec![("arch".into(), "x86_64".into())]);
        assert_eq!(p.subpath.as_deref(), Some("a/b"));
    }

    #[test]
    fn parse_rejects() {
        assert!(PackageUrl::parse("npm:foo").is_err());
        assert!(PackageUrl::parse("pkg:").is_err());
        assert!(PackageUrl::parse("pkg:/no-type@1").is_err());
        assert!(PackageUrl::parse("pkg:npm/@1").is_err());
    }

    #[test]
    fn canonical_round_trip() {
        let p = PackageUrl::new("npm", "lodash", Some("4.17.21"));
        assert_eq!(p.to_string_canonical(), "pkg:npm/lodash@4.17.21");
        let p2 = PackageUrl::new("apk", "openssl", Some("3.0"))
            .with_namespace("alpine")
            .with_qualifier("arch", "x86_64");
        assert_eq!(
            p2.to_string_canonical(),
            "pkg:apk/alpine/openssl@3.0?arch=x86_64"
        );
    }

    #[test]
    fn ecosystem_mapping() {
        assert_eq!(ecosystem_to_purl_type("npm"), "npm");
        assert_eq!(ecosystem_to_purl_type("pypi"), "pypi");
        assert_eq!(ecosystem_to_purl_type("cargo"), "cargo");
        assert_eq!(ecosystem_to_purl_type("alpine"), "apk");
        assert_eq!(ecosystem_to_purl_type("debian"), "deb");
        assert_eq!(ecosystem_to_purl_type("rhel"), "rpm");
        assert_eq!(ecosystem_to_purl_type("suse"), "rpm");
        assert_eq!(ecosystem_to_purl_type("?"), "generic");
    }
}
