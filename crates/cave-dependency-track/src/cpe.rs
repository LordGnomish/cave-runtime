// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CPE 2.3 parser + matcher.
//!
//! Mirrors `org.dependencytrack.model.ICpe` validation and the
//! `CpeBuilder` parse used by `CpePolicyEvaluator`.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// `cpe:2.3:part:vendor:product:version:update:edition:lang:sw_edition:target_sw:target_hw:other`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cpe23 {
    pub part: String,
    pub vendor: String,
    pub product: String,
    pub version: String,
    pub update: String,
    pub edition: String,
    pub language: String,
    pub sw_edition: String,
    pub target_sw: String,
    pub target_hw: String,
    pub other: String,
}

const ANY: &str = "*";

impl Cpe23 {
    pub fn parse(raw: &str) -> Result<Self> {
        let rest = raw
            .strip_prefix("cpe:2.3:")
            .ok_or_else(|| Error::Parse(format!("cpe: missing cpe:2.3: prefix in {}", raw)))?;
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() != 11 {
            return Err(Error::Parse(format!(
                "cpe: expected 11 fields, got {} in {}",
                parts.len(),
                raw
            )));
        }
        if !matches!(parts[0], "a" | "o" | "h" | "*") {
            return Err(Error::Parse(format!(
                "cpe: part must be a/o/h/*, got {}",
                parts[0]
            )));
        }
        Ok(Self {
            part: parts[0].into(),
            vendor: parts[1].into(),
            product: parts[2].into(),
            version: parts[3].into(),
            update: parts[4].into(),
            edition: parts[5].into(),
            language: parts[6].into(),
            sw_edition: parts[7].into(),
            target_sw: parts[8].into(),
            target_hw: parts[9].into(),
            other: parts[10].into(),
        })
    }

    pub fn to_string_(&self) -> String {
        format!(
            "cpe:2.3:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            self.part,
            self.vendor,
            self.product,
            self.version,
            self.update,
            self.edition,
            self.language,
            self.sw_edition,
            self.target_sw,
            self.target_hw,
            self.other
        )
    }

    /// CPE-2.3 matching — wildcard `*` and `-` (NA) on either side match.
    pub fn matches(&self, other: &Cpe23) -> bool {
        let pairs = [
            (&self.part, &other.part),
            (&self.vendor, &other.vendor),
            (&self.product, &other.product),
            (&self.version, &other.version),
            (&self.update, &other.update),
            (&self.edition, &other.edition),
            (&self.language, &other.language),
            (&self.sw_edition, &other.sw_edition),
            (&self.target_sw, &other.target_sw),
            (&self.target_hw, &other.target_hw),
            (&self.other, &other.other),
        ];
        pairs.iter().all(|(a, b)| field_matches(a, b))
    }
}

fn field_matches(a: &str, b: &str) -> bool {
    if a == ANY || b == ANY {
        return true;
    }
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REF: &str = "cpe:2.3:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*";

    #[test]
    fn parses_canonical_form() {
        let c = Cpe23::parse(REF).unwrap();
        assert_eq!(c.part, "a");
        assert_eq!(c.vendor, "openssl");
        assert_eq!(c.product, "openssl");
        assert_eq!(c.version, "3.0.0");
    }

    #[test]
    fn rejects_bad_prefix() {
        assert!(matches!(
            Cpe23::parse("cpe:2.2:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*"),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn rejects_bad_part() {
        assert!(matches!(
            Cpe23::parse("cpe:2.3:x:openssl:openssl:3.0.0:*:*:*:*:*:*:*"),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn rejects_wrong_field_count() {
        assert!(matches!(
            Cpe23::parse("cpe:2.3:a:openssl"),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn wildcard_matching_symmetric() {
        let needle = Cpe23::parse("cpe:2.3:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*").unwrap();
        let haystack = Cpe23::parse("cpe:2.3:a:openssl:openssl:*:*:*:*:*:*:*:*").unwrap();
        assert!(needle.matches(&haystack));
        assert!(haystack.matches(&needle));
    }

    #[test]
    fn version_mismatch_blocks_match() {
        let a = Cpe23::parse("cpe:2.3:a:openssl:openssl:1.1.1:*:*:*:*:*:*:*").unwrap();
        let b = Cpe23::parse("cpe:2.3:a:openssl:openssl:3.0.0:*:*:*:*:*:*:*").unwrap();
        assert!(!a.matches(&b));
    }

    #[test]
    fn roundtrip_to_string() {
        let c = Cpe23::parse(REF).unwrap();
        assert_eq!(c.to_string_(), REF);
    }
}
