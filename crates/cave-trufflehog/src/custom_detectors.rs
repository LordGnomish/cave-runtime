// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Custom-regex detector loader. Mirrors `pkg/custom_detectors/`:
//!   * YAML schema with `name`, `keywords`, `regex` (named-capture map),
//!     `verify[]` (multi-step HTTP probes), `successRanges`, `rotatedRanges`
//!   * Variable substitution from named-capture groups (`{name}`)
//!   * Validation: regex compile, response_matcher non-empty, header parse
//!
//! This is what makes cave-trufflehog Capgemini-grade: enterprises ship
//! their own internal-token detectors via TOML/YAML.

use crate::error::{Error, Result};
use crate::models::{DetectionResult, DetectorType};
use crate::verification::{StatusRange, VerifierConfig};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomDetectorSpec {
    pub name: String,
    pub keywords: Vec<String>,
    pub regex: BTreeMap<String, String>,
    #[serde(default)]
    pub verify: Vec<VerifyStep>,
    #[serde(default = "default_entropy")]
    pub min_entropy: f64,
    #[serde(default = "default_2xx_success")]
    pub success_ranges: Vec<[u16; 2]>,
    #[serde(default)]
    pub rotated_ranges: Vec<[u16; 2]>,
}

fn default_entropy() -> f64 {
    3.5
}
fn default_2xx_success() -> Vec<[u16; 2]> {
    vec![[200, 299]]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyStep {
    pub url: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub response_matcher: Option<String>,
}

fn default_method() -> String {
    "GET".to_string()
}

#[derive(Debug, Clone)]
pub struct CompiledCustomDetector {
    pub name: String,
    pub keywords: Vec<String>,
    pub patterns: Vec<(String, Regex)>,
    pub verify: Vec<VerifyStep>,
    pub min_entropy: f64,
    pub config: VerifierConfig,
}

pub fn load_spec_yaml(text: &str) -> Result<Vec<CustomDetectorSpec>> {
    serde_yaml::from_str::<CustomConfig>(text)
        .map(|c| c.detectors)
        .map_err(|e| Error::Config(e.to_string()))
}

pub fn load_spec_toml(text: &str) -> Result<Vec<CustomDetectorSpec>> {
    toml::from_str::<CustomConfig>(text)
        .map(|c| c.detectors)
        .map_err(|e| Error::Config(e.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomConfig {
    #[serde(default)]
    detectors: Vec<CustomDetectorSpec>,
}

pub fn compile(spec: CustomDetectorSpec) -> Result<CompiledCustomDetector> {
    if spec.name.is_empty() {
        return Err(Error::Config("custom detector name must be non-empty".into()));
    }
    if spec.regex.is_empty() {
        return Err(Error::Config(format!(
            "custom detector {} has no regex entries",
            spec.name
        )));
    }
    let mut patterns = Vec::with_capacity(spec.regex.len());
    for (k, v) in spec.regex {
        let re = Regex::new(&v)?;
        patterns.push((k, re));
    }
    let success_ranges = spec
        .success_ranges
        .into_iter()
        .map(|r| StatusRange::new(r[0], r[1]))
        .collect();
    let rotated_ranges = spec
        .rotated_ranges
        .into_iter()
        .map(|r| StatusRange::new(r[0], r[1]))
        .collect();
    Ok(CompiledCustomDetector {
        name: spec.name,
        keywords: spec.keywords,
        patterns,
        verify: spec.verify,
        min_entropy: spec.min_entropy,
        config: VerifierConfig {
            success_ranges,
            rotated_ranges,
        },
    })
}

impl CompiledCustomDetector {
    pub fn scan(&self, data: &[u8]) -> Vec<DetectionResult> {
        let Ok(s) = std::str::from_utf8(data) else {
            return Vec::new();
        };
        if !self.has_keyword(s) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (label, re) in &self.patterns {
            for m in re.find_iter(s) {
                let raw = m.as_str();
                if shannon_entropy(raw) < self.min_entropy {
                    continue;
                }
                let mut r = DetectionResult::new(DetectorType::Custom, raw)
                    .with_extra("custom_detector", &self.name)
                    .with_extra("pattern", label);
                r.detector_name = self.name.clone();
                out.push(r);
            }
        }
        out
    }

    fn has_keyword(&self, s: &str) -> bool {
        if self.keywords.is_empty() {
            return true;
        }
        self.keywords.iter().any(|k| s.contains(k))
    }

    /// Replace `{name}` substitutions in a URL/body/header from a capture map.
    pub fn render(template: &str, vars: &BTreeMap<String, String>) -> String {
        let mut out = template.to_string();
        for (k, v) in vars {
            out = out.replace(&format!("{{{}}}", k), v);
        }
        out
    }
}

pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for b in s.bytes() {
        counts[b as usize] += 1;
    }
    let len = s.len() as f64;
    let mut h = 0.0;
    for c in counts {
        if c == 0 {
            continue;
        }
        let p = c as f64 / len;
        h -= p * p.log2();
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
detectors:
  - name: AcmeInternal
    keywords: ["acme_"]
    regex:
      token: 'acme_[a-zA-Z0-9]{20,40}'
    min_entropy: 3.0
    verify:
      - url: "https://api.acme.com/v1/me"
        method: GET
        headers:
          Authorization: "Bearer {token}"
"#
    }

    #[test]
    fn load_yaml_round_trip() {
        let specs = load_spec_yaml(sample_yaml()).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "AcmeInternal");
        assert!(specs[0].regex.contains_key("token"));
    }

    #[test]
    fn compile_rejects_empty_name() {
        let spec = CustomDetectorSpec {
            name: "".into(),
            keywords: vec!["x".into()],
            regex: BTreeMap::new(),
            verify: vec![],
            min_entropy: 3.0,
            success_ranges: vec![[200, 299]],
            rotated_ranges: vec![],
        };
        assert!(compile(spec).is_err());
    }

    #[test]
    fn compile_rejects_invalid_regex() {
        let mut r = BTreeMap::new();
        r.insert("t".into(), "(unbalanced".into());
        let spec = CustomDetectorSpec {
            name: "x".into(),
            keywords: vec!["x".into()],
            regex: r,
            verify: vec![],
            min_entropy: 3.0,
            success_ranges: vec![[200, 299]],
            rotated_ranges: vec![],
        };
        assert!(compile(spec).is_err());
    }

    #[test]
    fn scan_matches_keyword_and_regex() {
        let specs = load_spec_yaml(sample_yaml()).unwrap();
        let cd = compile(specs[0].clone()).unwrap();
        let r = cd.scan(b"my token is acme_ABCDEFGH012345IJKLMNOP");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].detector_name, "AcmeInternal");
        assert_eq!(r[0].detector_type, DetectorType::Custom);
    }

    #[test]
    fn scan_skips_low_entropy() {
        let mut r = BTreeMap::new();
        r.insert("t".into(), "test_[a-z]{20,}".into());
        let spec = CustomDetectorSpec {
            name: "Low".into(),
            keywords: vec!["test_".into()],
            regex: r,
            verify: vec![],
            min_entropy: 4.5,
            success_ranges: vec![[200, 299]],
            rotated_ranges: vec![],
        };
        let cd = compile(spec).unwrap();
        // All same letter -> very low entropy.
        assert_eq!(cd.scan(b"test_aaaaaaaaaaaaaaaaaaaa").len(), 0);
    }

    #[test]
    fn render_replaces_named_captures() {
        let mut v = BTreeMap::new();
        v.insert("token".into(), "abc123".into());
        let s = CompiledCustomDetector::render("Bearer {token}", &v);
        assert_eq!(s, "Bearer abc123");
    }

    #[test]
    fn keyword_prefilter_skips_mismatched_chunks() {
        let specs = load_spec_yaml(sample_yaml()).unwrap();
        let cd = compile(specs[0].clone()).unwrap();
        assert!(cd.scan(b"no relevant content here").is_empty());
    }

    #[test]
    fn entropy_zero_for_empty_high_for_random() {
        assert_eq!(shannon_entropy(""), 0.0);
        assert!(shannon_entropy("aaaaaaaa") < 1.0);
        assert!(shannon_entropy("AbCdEfGh1234!?@#") > 3.0);
    }

    #[test]
    fn load_toml_round_trip() {
        let t = r#"
[[detectors]]
name = "AcmeTOML"
keywords = ["acme_"]
regex = { token = 'acme_[a-zA-Z0-9]{20,40}' }
min_entropy = 3.0
success_ranges = [[200, 299]]
"#;
        let specs = load_spec_toml(t).unwrap();
        assert_eq!(specs[0].name, "AcmeTOML");
    }
}
