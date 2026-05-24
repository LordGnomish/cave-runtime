// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Package URL parser — `pkg:type/namespace/name@version?qual=val#sub`.
//!
//! Spec: <https://github.com/package-url/purl-spec>.
//! Mirrors `com.github.packageurl.PackageURL`.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Purl {
    pub r#type: String,
    pub namespace: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub qualifiers: BTreeMap<String, String>,
    pub subpath: Option<String>,
}

impl Purl {
    pub fn parse(raw: &str) -> Result<Self> {
        let rest = raw
            .strip_prefix("pkg:")
            .ok_or_else(|| Error::Parse(format!("purl: missing pkg: prefix in {}", raw)))?;

        let (head, subpath) = match rest.split_once('#') {
            Some((h, t)) => (h, Some(percent_decode(t))),
            None => (rest, None),
        };
        let (head, qualifiers_raw) = match head.split_once('?') {
            Some((h, q)) => (h, Some(q)),
            None => (head, None),
        };
        let (head, version) = match head.rsplit_once('@') {
            Some((h, v)) => (h, Some(percent_decode(v))),
            None => (head, None),
        };
        let mut parts = head.split('/').filter(|p| !p.is_empty());
        let r#type = parts
            .next()
            .ok_or_else(|| Error::Parse("purl: missing type".into()))?
            .to_ascii_lowercase();
        let segments: Vec<String> = parts.map(percent_decode).collect();
        if segments.is_empty() {
            return Err(Error::Parse("purl: missing name".into()));
        }
        let name = segments
            .last()
            .ok_or_else(|| Error::Parse("purl: missing name".into()))?
            .clone();
        let namespace = if segments.len() > 1 {
            Some(segments[..segments.len() - 1].join("/"))
        } else {
            None
        };
        let mut qualifiers = BTreeMap::new();
        if let Some(qs) = qualifiers_raw {
            for kv in qs.split('&').filter(|p| !p.is_empty()) {
                if let Some((k, v)) = kv.split_once('=') {
                    qualifiers.insert(k.to_ascii_lowercase(), percent_decode(v));
                }
            }
        }
        Ok(Self {
            r#type,
            namespace,
            name,
            version,
            qualifiers,
            subpath,
        })
    }

    pub fn canonical(&self) -> String {
        let mut out = String::from("pkg:");
        out.push_str(&self.r#type);
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
            let mut first = true;
            for (k, v) in &self.qualifiers {
                if !first {
                    out.push('&');
                }
                first = false;
                out.push_str(k);
                out.push('=');
                out.push_str(v);
            }
        }
        if let Some(s) = &self.subpath {
            out.push('#');
            out.push_str(s);
        }
        out
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(10 + c - b'a'),
        b'A'..=b'F' => Some(10 + c - b'A'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_cargo_purl() {
        let p = Purl::parse("pkg:cargo/serde@1.0").unwrap();
        assert_eq!(p.r#type, "cargo");
        assert_eq!(p.name, "serde");
        assert_eq!(p.version.as_deref(), Some("1.0"));
        assert!(p.namespace.is_none());
    }

    #[test]
    fn parses_namespaced_maven_purl() {
        let p = Purl::parse("pkg:maven/org.apache.commons/commons-lang3@3.12.0").unwrap();
        assert_eq!(p.r#type, "maven");
        assert_eq!(p.namespace.as_deref(), Some("org.apache.commons"));
        assert_eq!(p.name, "commons-lang3");
        assert_eq!(p.version.as_deref(), Some("3.12.0"));
    }

    #[test]
    fn parses_qualifiers_and_subpath() {
        let p = Purl::parse(
            "pkg:deb/debian/curl@7.50.3-1?arch=amd64&distro=jessie#path/to/thing",
        )
        .unwrap();
        assert_eq!(p.qualifiers.get("arch").map(String::as_str), Some("amd64"));
        assert_eq!(p.qualifiers.get("distro").map(String::as_str), Some("jessie"));
        assert_eq!(p.subpath.as_deref(), Some("path/to/thing"));
    }

    #[test]
    fn rejects_missing_prefix() {
        assert!(matches!(Purl::parse("cargo/serde"), Err(Error::Parse(_))));
    }

    #[test]
    fn rejects_missing_name() {
        assert!(matches!(Purl::parse("pkg:cargo"), Err(Error::Parse(_))));
    }

    #[test]
    fn canonical_roundtrip() {
        let original = "pkg:maven/org.apache.commons/commons-lang3@3.12.0";
        let p = Purl::parse(original).unwrap();
        assert_eq!(p.canonical(), original);
    }

    #[test]
    fn lowercases_type() {
        let p = Purl::parse("pkg:NPM/lodash@1").unwrap();
        assert_eq!(p.r#type, "npm");
    }

    #[test]
    fn percent_decoded_namespace() {
        let p = Purl::parse("pkg:maven/org.foo%2Fbar/baz@1").unwrap();
        assert_eq!(p.namespace.as_deref(), Some("org.foo/bar"));
    }
}
