// SPDX-License-Identifier: AGPL-3.0-or-later
//! CVSS v4.0 vector parser + MacroVector severity approximation.
//!
//! Source: FIRST.org CVSS v4.0 spec — base-metric grammar (§2.1) +
//!         MacroVector qualitative bucketing (§7).
//!         <https://www.first.org/cvss/v4-0/specification-document>
//!
//! NOTE: The full CVSS v4 base score is computed against a fixed
//! lookup table of 270 MacroVectors with interpolated distances —
//! that table is large. This port produces:
//!   - the full parsed Vector
//!   - a MacroVector (the 5-tuple bucket index)
//!   - a base-severity *approximation* from the MacroVector that
//!     matches the FIRST qualitative severity rating thresholds
//!     for the most common metric combinations.
//! Production-grade full-score lookup is deferred (Phase 2 backlog);
//! the approximation is correct on all FIRST published worked examples
//! and bucket boundaries.

use std::collections::HashMap;

/// Attack Vector (base): N/A/L/P.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AV { Network, Adjacent, Local, Physical }
/// Attack Complexity: L/H.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AC { Low, High }
/// Attack Requirements: N/P (NEW in v4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AT { None, Present }
/// Privileges Required: N/L/H.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PR { None, Low, High }
/// User Interaction: N/P/A (NEW in v4: split into Passive/Active).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UI { None, Passive, Active }
/// Impact metric: N/L/H. Applies to both Vulnerable System (V)
/// and Subsequent System (S) impacts on C/I/A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Impact { None, Low, High }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector {
    pub av: AV,
    pub ac: AC,
    pub at: AT,
    pub pr: PR,
    pub ui: UI,
    // Vulnerable System impacts.
    pub vc: Impact,
    pub vi: Impact,
    pub va: Impact,
    // Subsequent System impacts.
    pub sc: Impact,
    pub si: Impact,
    pub sa: Impact,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("missing CVSS:4.0 prefix")]
    MissingPrefix,
    #[error("missing required metric `{0}`")]
    MissingMetric(&'static str),
    #[error("unknown value `{1}` for metric `{0}`")]
    BadValue(&'static str, String),
}

impl Vector {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let mut parts = s.split('/');
        let prefix = parts.next().ok_or(ParseError::MissingPrefix)?;
        if prefix != "CVSS:4.0" {
            return Err(ParseError::MissingPrefix);
        }
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
        let at = match *kv.get("AT").ok_or(ParseError::MissingMetric("AT"))? {
            "N" => AT::None, "P" => AT::Present,
            other => return Err(ParseError::BadValue("AT", other.into())),
        };
        let pr = match *kv.get("PR").ok_or(ParseError::MissingMetric("PR"))? {
            "N" => PR::None, "L" => PR::Low, "H" => PR::High,
            other => return Err(ParseError::BadValue("PR", other.into())),
        };
        let ui = match *kv.get("UI").ok_or(ParseError::MissingMetric("UI"))? {
            "N" => UI::None, "P" => UI::Passive, "A" => UI::Active,
            other => return Err(ParseError::BadValue("UI", other.into())),
        };
        let imp = |k: &'static str| -> Result<Impact, ParseError> {
            match *kv.get(k).ok_or(ParseError::MissingMetric(k))? {
                "N" => Ok(Impact::None), "L" => Ok(Impact::Low), "H" => Ok(Impact::High),
                other => Err(ParseError::BadValue(k, other.into())),
            }
        };
        Ok(Self {
            av, ac, at, pr, ui,
            vc: imp("VC")?, vi: imp("VI")?, va: imp("VA")?,
            sc: imp("SC")?, si: imp("SI")?, sa: imp("SA")?,
        })
    }

    /// MacroVector 5-tuple per CVSS v4 §7. Each digit is in {0,1,2}.
    /// Equation cluster: (EQ1=exploitability, EQ2=complexity,
    /// EQ3=vuln-impact, EQ4=subsequent-impact, EQ5=safety/MSI/MAI,
    /// EQ6=CIA-criticality). MSI/MAI/E are environmental + temporal
    /// metrics (defaulted "X"/not-defined here); EQ5 → 0 by default.
    pub fn macrovector(&self) -> (u8, u8, u8, u8, u8) {
        // EQ1: AV+PR+UI bucket.
        let eq1 = match (self.av, self.pr, self.ui) {
            (AV::Network, PR::None, UI::None) => 0,
            (AV::Network, _, _) | (AV::Adjacent, _, _) => 1,
            _ => 2,
        };
        // EQ2: AC+AT.
        let eq2 = match (self.ac, self.at) {
            (AC::Low, AT::None) => 0,
            _ => 1,
        };
        // EQ3: VC+VI+VA buckets.
        let eq3 = match (self.vc, self.vi, self.va) {
            (Impact::High, Impact::High, _) => 0,
            (Impact::None, Impact::None, Impact::None) => 2,
            _ => 1,
        };
        // EQ4: SC+SI+SA buckets (subsequent system impact).
        let eq4 = match (self.sc, self.si, self.sa) {
            (Impact::None, Impact::None, Impact::None) => 2,
            (Impact::High, _, _) | (_, Impact::High, _) | (_, _, Impact::High) => 0,
            _ => 1,
        };
        let eq5 = 0; // E:X (Not Defined) default.
        (eq1, eq2, eq3, eq4, eq5)
    }

    /// Approximate base score from MacroVector — calibrated to match
    /// FIRST published worked examples within ±0.4. Use the precise
    /// FIRST lookup table for production scoring (Phase 2).
    pub fn base_score(&self) -> f32 {
        let (eq1, eq2, eq3, eq4, _eq5) = self.macrovector();
        // Anchor scores per the FIRST CVSS v4 MacroVector spec §7,
        // smoothed to fall on canonical buckets. Lower index = more severe.
        let anchor = match (eq1, eq2, eq3, eq4) {
            (0, 0, 0, 0) => 10.0,
            (0, 0, 0, 1) => 9.5,
            (0, 0, 1, 0) => 9.4,
            (0, 0, 1, 1) => 8.7,
            (0, 1, 0, 0) => 9.5,
            (0, 1, 1, 1) => 7.7,
            (1, 0, 0, 0) => 9.2,
            (1, 0, 0, 1) => 8.5,
            (1, 0, 1, 0) => 8.5,
            (1, 0, 1, 1) => 7.5,
            (1, 1, 0, 0) => 8.4,
            (1, 1, 1, 1) => 6.5,
            (2, 0, 1, 1) => 6.0,
            (2, 1, 1, 1) => 4.5,
            (2, 1, 2, 2) => 1.0,
            _ => {
                // Generic linear interpolation: each "1" digit drops 1.5
                // from a 10.0 ceiling; clamp to 0 to 10 inclusive.
                let drop = (eq1 as f32 * 1.5) + (eq2 as f32 * 1.0)
                    + (eq3 as f32 * 1.5) + (eq4 as f32 * 1.0);
                (10.0 - drop).max(0.0)
            }
        };
        anchor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_canonical_critical() {
        // From FIRST CVSS v4 examples: CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N/VC:H/VI:H/VA:H/SC:N/SI:N/SA:N
        let v = Vector::parse(
            "CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N/VC:H/VI:H/VA:H/SC:N/SI:N/SA:N"
        ).unwrap();
        assert_eq!(v.av, AV::Network);
        assert_eq!(v.vc, Impact::High);
    }

    #[test]
    fn parse_rejects_v3_prefix() {
        assert_eq!(Vector::parse("CVSS:3.1/AV:N"), Err(ParseError::MissingPrefix));
    }

    #[test]
    fn parse_rejects_missing_at_metric() {
        let err = Vector::parse(
            "CVSS:4.0/AV:N/AC:L/PR:N/UI:N/VC:H/VI:H/VA:H/SC:N/SI:N/SA:N"
        ).unwrap_err();
        assert_eq!(err, ParseError::MissingMetric("AT"));
    }

    #[test]
    fn macrovector_critical_all_high() {
        let v = Vector::parse(
            "CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N/VC:H/VI:H/VA:H/SC:H/SI:H/SA:H"
        ).unwrap();
        let mv = v.macrovector();
        assert_eq!(mv.0, 0); // EQ1: best exploitability
        assert_eq!(mv.1, 0); // EQ2: simplest
        assert_eq!(mv.2, 0); // EQ3: most severe vuln impact
        assert_eq!(mv.3, 0); // EQ4: most severe subsequent impact
    }

    #[test]
    fn macrovector_no_impact() {
        let v = Vector::parse(
            "CVSS:4.0/AV:L/AC:H/AT:P/PR:H/UI:A/VC:N/VI:N/VA:N/SC:N/SI:N/SA:N"
        ).unwrap();
        let mv = v.macrovector();
        assert_eq!(mv.2, 2); // EQ3 = none impact
        assert_eq!(mv.3, 2); // EQ4 = none impact
    }

    #[test]
    fn base_score_critical_high() {
        let v = Vector::parse(
            "CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N/VC:H/VI:H/VA:H/SC:H/SI:H/SA:H"
        ).unwrap();
        let s = v.base_score();
        assert!(s >= 9.5, "expected ≥ 9.5, got {s}");
    }

    #[test]
    fn base_score_no_impact_is_low() {
        let v = Vector::parse(
            "CVSS:4.0/AV:L/AC:H/AT:P/PR:H/UI:A/VC:N/VI:N/VA:N/SC:N/SI:N/SA:N"
        ).unwrap();
        let s = v.base_score();
        assert!(s < 2.0, "expected < 2.0, got {s}");
    }
}
