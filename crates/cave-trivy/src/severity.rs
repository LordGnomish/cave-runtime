// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Severity scale + ordering + parse + filter helpers.
//!
//! Mirrors trivy's `pkg/types/severity.go` ordering:
//! UNKNOWN < LOW < MEDIUM < HIGH < CRITICAL.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn rank(&self) -> u8 {
        match self {
            Severity::Unknown => 0,
            Severity::Low => 1,
            Severity::Medium => 2,
            Severity::High => 3,
            Severity::Critical => 4,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Unknown => "UNKNOWN",
            Severity::Low => "LOW",
            Severity::Medium => "MEDIUM",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }

    pub fn all() -> [Severity; 5] {
        [
            Severity::Unknown,
            Severity::Low,
            Severity::Medium,
            Severity::High,
            Severity::Critical,
        ]
    }

    /// Inclusive: returns true when `self.rank() >= floor.rank()`.
    pub fn at_least(self, floor: Severity) -> bool {
        self.rank() >= floor.rank()
    }
}

impl Ord for Severity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl PartialOrd for Severity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl FromStr for Severity {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "UNKNOWN" | "NONE" => Ok(Severity::Unknown),
            "LOW" => Ok(Severity::Low),
            "MEDIUM" | "MODERATE" => Ok(Severity::Medium),
            "HIGH" | "IMPORTANT" => Ok(Severity::High),
            "CRITICAL" => Ok(Severity::Critical),
            other => Err(format!("unknown severity: {}", other)),
        }
    }
}

/// CSV severity selector (`--severity HIGH,CRITICAL`) → predicate.
pub fn parse_csv(csv: &str) -> Result<Vec<Severity>, String> {
    let mut out = Vec::new();
    for part in csv.split(',') {
        let s = part.trim();
        if s.is_empty() {
            continue;
        }
        out.push(Severity::from_str(s)?);
    }
    if out.is_empty() {
        return Err("severity selector empty".into());
    }
    Ok(out)
}

/// Pre-defined Trivy "exit code" gate: returns whether a scan result has
/// any vulnerability at-or-above `floor`.
pub fn any_at_least<'a>(
    severities: impl IntoIterator<Item = &'a Severity>,
    floor: Severity,
) -> bool {
    severities.into_iter().any(|s| s.at_least(floor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Unknown);
    }

    #[test]
    fn parsing_synonyms() {
        assert_eq!("HIGH".parse::<Severity>().unwrap(), Severity::High);
        assert_eq!("important".parse::<Severity>().unwrap(), Severity::High);
        assert_eq!("Moderate".parse::<Severity>().unwrap(), Severity::Medium);
        assert_eq!("none".parse::<Severity>().unwrap(), Severity::Unknown);
        assert!("nope".parse::<Severity>().is_err());
    }

    #[test]
    fn at_least_floor() {
        assert!(Severity::Critical.at_least(Severity::High));
        assert!(Severity::High.at_least(Severity::High));
        assert!(!Severity::Medium.at_least(Severity::High));
    }

    #[test]
    fn csv_parse() {
        let s = parse_csv("HIGH, CRITICAL").unwrap();
        assert_eq!(s.len(), 2);
        assert!(parse_csv("").is_err());
        assert!(parse_csv("HIGH,bogus").is_err());
    }

    #[test]
    fn any_floor() {
        let v = [Severity::Low, Severity::High];
        assert!(any_at_least(&v, Severity::High));
        assert!(!any_at_least(&v, Severity::Critical));
    }

    #[test]
    fn serde_round_trip() {
        let j = serde_json::to_string(&Severity::Critical).unwrap();
        assert_eq!(j, "\"CRITICAL\"");
        let back: Severity = serde_json::from_str(&j).unwrap();
        assert_eq!(back, Severity::Critical);
    }

    #[test]
    fn all_listing() {
        assert_eq!(Severity::all().len(), 5);
    }
}
