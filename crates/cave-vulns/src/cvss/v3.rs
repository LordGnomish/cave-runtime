// SPDX-License-Identifier: AGPL-3.0-or-later
//! CVSS v3.1 vector parser + base-score calculator.
//!
//! Source: FIRST.org CVSS v3.1 spec, §7 (base score equation).
//!         <https://www.first.org/cvss/v3.1/specification-document>
//!
//! DefectDojo wraps the third-party `cvss` Python library; this is
//! a clean-room Rust implementation of the same equations so we
//! don't take a runtime Python dep.

use std::collections::HashMap;

/// Attack vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AV { Network, Adjacent, Local, Physical }
/// Attack complexity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AC { Low, High }
/// Privileges required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PR { None, Low, High }
/// User interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UI { None, Required }
/// Scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S { Unchanged, Changed }
/// CIA impact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CIA { None, Low, High }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector {
    pub version: (u8, u8),
    pub av: AV,
    pub ac: AC,
    pub pr: PR,
    pub ui: UI,
    pub s: S,
    pub c: CIA,
    pub i: CIA,
    pub a: CIA,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("missing CVSS:3.x prefix")]
    MissingPrefix,
    #[error("unsupported CVSS version: {0}")]
    UnsupportedVersion(String),
    #[error("missing required metric `{0}`")]
    MissingMetric(&'static str),
    #[error("unknown value `{1}` for metric `{0}`")]
    BadValue(&'static str, String),
}

impl Vector {
    /// Parse a canonical CVSS v3.x vector string like
    /// `CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H`.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let mut parts = s.split('/');
        let prefix = parts.next().ok_or(ParseError::MissingPrefix)?;
        let version = match prefix {
            "CVSS:3.0" => (3, 0),
            "CVSS:3.1" => (3, 1),
            other if other.starts_with("CVSS:") => {
                return Err(ParseError::UnsupportedVersion(other.into()));
            }
            _ => return Err(ParseError::MissingPrefix),
        };
        let mut kv: HashMap<&str, &str> = HashMap::new();
        for p in parts {
            if let Some((k, v)) = p.split_once(':') {
                kv.insert(k, v);
            }
        }
        let av = match *kv.get("AV").ok_or(ParseError::MissingMetric("AV"))? {
            "N" => AV::Network, "A" => AV::Adjacent, "L" => AV::Local, "P" => AV::Physical,
            other => return Err(ParseError::BadValue("AV", other.into())),
        };
        let ac = match *kv.get("AC").ok_or(ParseError::MissingMetric("AC"))? {
            "L" => AC::Low, "H" => AC::High,
            other => return Err(ParseError::BadValue("AC", other.into())),
        };
        let pr = match *kv.get("PR").ok_or(ParseError::MissingMetric("PR"))? {
            "N" => PR::None, "L" => PR::Low, "H" => PR::High,
            other => return Err(ParseError::BadValue("PR", other.into())),
        };
        let ui = match *kv.get("UI").ok_or(ParseError::MissingMetric("UI"))? {
            "N" => UI::None, "R" => UI::Required,
            other => return Err(ParseError::BadValue("UI", other.into())),
        };
        let s_metric = match *kv.get("S").ok_or(ParseError::MissingMetric("S"))? {
            "U" => S::Unchanged, "C" => S::Changed,
            other => return Err(ParseError::BadValue("S", other.into())),
        };
        let cia = |k: &'static str| -> Result<CIA, ParseError> {
            match *kv.get(k).ok_or(ParseError::MissingMetric(k))? {
                "N" => Ok(CIA::None), "L" => Ok(CIA::Low), "H" => Ok(CIA::High),
                other => Err(ParseError::BadValue(k, other.into())),
            }
        };
        Ok(Self {
            version,
            av,
            ac,
            pr,
            ui,
            s: s_metric,
            c: cia("C")?,
            i: cia("I")?,
            a: cia("A")?,
        })
    }

    /// Base score per FIRST CVSS v3.1 §7.1.
    pub fn base_score(&self) -> f32 {
        let av: f32 = match self.av {
            AV::Network => 0.85,
            AV::Adjacent => 0.62,
            AV::Local => 0.55,
            AV::Physical => 0.2,
        };
        let ac: f32 = match self.ac {
            AC::Low => 0.77,
            AC::High => 0.44,
        };
        // PR depends on Scope (changed vector with PR raises the weight).
        let pr: f32 = match (self.pr, self.s) {
            (PR::None, _) => 0.85,
            (PR::Low, S::Unchanged) => 0.62,
            (PR::Low, S::Changed) => 0.68,
            (PR::High, S::Unchanged) => 0.27,
            (PR::High, S::Changed) => 0.5,
        };
        let ui: f32 = match self.ui {
            UI::None => 0.85,
            UI::Required => 0.62,
        };
        let cia = |x: CIA| -> f32 { match x {
            CIA::None => 0.0,
            CIA::Low => 0.22,
            CIA::High => 0.56,
        }};
        let c = cia(self.c);
        let i = cia(self.i);
        let a = cia(self.a);
        let isc_base: f32 = 1.0_f32 - ((1.0_f32 - c) * (1.0_f32 - i) * (1.0_f32 - a));
        let impact: f32 = match self.s {
            S::Unchanged => 6.42_f32 * isc_base,
            S::Changed => 7.52_f32 * (isc_base - 0.029_f32) - 3.25_f32 * (isc_base - 0.02_f32).powf(15.0),
        };
        let exploitability: f32 = 8.22_f32 * av * ac * pr * ui;
        if impact <= 0.0 {
            return 0.0;
        }
        let raw: f32 = match self.s {
            S::Unchanged => (impact + exploitability).min(10.0_f32),
            S::Changed => (1.08_f32 * (impact + exploitability)).min(10.0_f32),
        };
        // Round up to one decimal (FIRST roundup).
        (raw * 10.0_f32).ceil() / 10.0_f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_canonical_critical_vector() {
        let v = Vector::parse("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H").unwrap();
        assert_eq!(v.version, (3, 1));
        assert_eq!(v.av, AV::Network);
        assert_eq!(v.c, CIA::High);
    }

    #[test]
    fn parse_rejects_missing_prefix() {
        assert_eq!(Vector::parse("AV:N/AC:L"), Err(ParseError::MissingPrefix));
    }

    #[test]
    fn parse_rejects_v40_string() {
        assert_eq!(
            Vector::parse("CVSS:4.0/AV:N"),
            Err(ParseError::UnsupportedVersion("CVSS:4.0".into()))
        );
    }

    #[test]
    fn parse_rejects_unknown_value() {
        let err = Vector::parse("CVSS:3.1/AV:Q/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H").unwrap_err();
        assert_eq!(err, ParseError::BadValue("AV", "Q".into()));
    }

    #[test]
    fn base_score_critical_log4shell_like() {
        // Log4Shell CVE-2021-44228: 10.0
        let v = Vector::parse("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H").unwrap();
        let score = v.base_score();
        assert!((score - 10.0).abs() < 0.01, "got {score}");
    }

    #[test]
    fn base_score_classic_critical_no_scope_change() {
        // AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H ⇒ 9.8
        let v = Vector::parse("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H").unwrap();
        let score = v.base_score();
        assert!((score - 9.8).abs() < 0.01, "got {score}");
    }

    #[test]
    fn base_score_medium_xss_like() {
        // Reflected XSS: AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N → 6.1
        let v = Vector::parse("CVSS:3.1/AV:N/AC:L/PR:N/UI:R/S:C/C:L/I:L/A:N").unwrap();
        let score = v.base_score();
        assert!((score - 6.1).abs() < 0.01, "got {score}");
    }

    #[test]
    fn base_score_zero_when_no_impact() {
        // All CIA = None ⇒ 0.0
        let v = Vector::parse("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:N").unwrap();
        assert_eq!(v.base_score(), 0.0);
    }

    #[test]
    fn base_score_capped_at_ten() {
        let v = Vector::parse("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H").unwrap();
        assert!(v.base_score() <= 10.0);
    }
}
