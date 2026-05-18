// SPDX-License-Identifier: AGPL-3.0-or-later
//! DefectDojo-parity Finding model with full lifecycle state machine.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:2397
//!         (`class Finding(BaseModel)`)
//!
//! Upstream Finding has 60+ fields; we port the security-essentials:
//! identity (id/title/cwe/cve), scoring (severity/cvssv3/cvssv4),
//! triage (active/verified/false_p/duplicate/risk_accepted/out_of_scope/
//! is_mitigated/under_review), provenance (date/found_by/test/component),
//! and remediation (mitigation/references/impact/steps_to_reproduce).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod state;

pub use state::{FindingState, StateError, StateTransition};

/// Severity — TitleCase to match DefectDojo's `Finding.SEVERITIES` exactly.
///
/// Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py
///         (`Finding.SEVERITIES = ['Critical','High','Medium','Low','Info']`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FindingSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl FindingSeverity {
    /// Numeric weight — higher = more severe. Matches DefectDojo's
    /// `SEVERITIES_NUMERIC` mapping in `dojo/utils.py::get_numerical_severity`.
    pub fn weight(&self) -> u8 {
        match self {
            Self::Critical => 4,
            Self::High => 3,
            Self::Medium => 2,
            Self::Low => 1,
            Self::Info => 0,
        }
    }

    /// Parse from DefectDojo string (case-insensitive, accepting common variants).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "CRITICAL" => Some(Self::Critical),
            "HIGH" | "ERROR" => Some(Self::High),
            "MEDIUM" | "WARNING" | "MODERATE" => Some(Self::Medium),
            "LOW" => Some(Self::Low),
            "INFO" | "INFORMATIONAL" | "NONE" | "UNKNOWN" | "NOTE" => Some(Self::Info),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
            Self::Info => "Info",
        }
    }
}

/// Full DefectDojo-parity finding record.
///
/// Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:2397-2700
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Finding {
    pub id: Uuid,
    pub title: String,
    pub date: DateTime<Utc>,

    // Identity / classification
    pub cwe: Option<u32>,
    pub cve: Option<String>,
    pub vulnerability_ids: Vec<String>,
    pub vuln_id_from_tool: Option<String>,
    pub unique_id_from_tool: Option<String>,

    // Scoring
    pub severity: FindingSeverity,
    pub cvssv3: Option<String>,
    pub cvssv3_score: Option<f32>,
    pub cvssv4: Option<String>,
    pub cvssv4_score: Option<f32>,
    pub epss_score: Option<f32>,
    pub epss_percentile: Option<f32>,
    pub known_exploited: bool,

    // Body
    pub description: String,
    pub mitigation: Option<String>,
    pub impact: Option<String>,
    pub steps_to_reproduce: Option<String>,
    pub severity_justification: Option<String>,
    pub references: Option<String>,

    // Provenance
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub component_name: Option<String>,
    pub component_version: Option<String>,
    pub fix_available: Option<bool>,
    pub fix_version: Option<String>,
    pub service: Option<String>,
    pub found_by_scanner: Option<String>,

    // Test linkage (DefectDojo ForeignKey -> Test)
    pub test_id: Option<Uuid>,

    // Triage / state machine
    pub state: FindingState,

    // Audit
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub mitigated: Option<DateTime<Utc>>,

    // Dedup helpers
    pub hash_code: Option<String>,
    pub nb_occurences: u32,

    // Scanner discriminator
    pub static_finding: bool,
    pub dynamic_finding: bool,
}

impl Finding {
    /// Build a new finding with sensible defaults — equivalent to DefectDojo's
    /// `Finding.__init__` with the title/severity required, everything else None.
    pub fn new(title: impl Into<String>, severity: FindingSeverity) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            date: now,
            cwe: None,
            cve: None,
            vulnerability_ids: Vec::new(),
            vuln_id_from_tool: None,
            unique_id_from_tool: None,
            severity,
            cvssv3: None,
            cvssv3_score: None,
            cvssv4: None,
            cvssv4_score: None,
            epss_score: None,
            epss_percentile: None,
            known_exploited: false,
            description: String::new(),
            mitigation: None,
            impact: None,
            steps_to_reproduce: None,
            severity_justification: None,
            references: None,
            file_path: None,
            line: None,
            component_name: None,
            component_version: None,
            fix_available: None,
            fix_version: None,
            service: None,
            found_by_scanner: None,
            test_id: None,
            state: FindingState::fresh(),
            created: now,
            modified: now,
            mitigated: None,
            hash_code: None,
            nb_occurences: 1,
            static_finding: false,
            dynamic_finding: false,
        }
    }

    /// Apply a state transition, persisting `modified` and `mitigated`
    /// timestamps per DefectDojo semantics
    /// (`Finding.save` → `set_mitigated_field`).
    pub fn transition(&mut self, t: StateTransition, actor: &str) -> Result<(), StateError> {
        self.state = self.state.apply(t, actor)?;
        let now = Utc::now();
        self.modified = now;
        if matches!(t, StateTransition::Mitigate) {
            self.mitigated = Some(now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_weight_order_matches_defectdojo() {
        assert!(FindingSeverity::Critical.weight() > FindingSeverity::High.weight());
        assert!(FindingSeverity::High.weight() > FindingSeverity::Medium.weight());
        assert!(FindingSeverity::Medium.weight() > FindingSeverity::Low.weight());
        assert!(FindingSeverity::Low.weight() > FindingSeverity::Info.weight());
    }

    #[test]
    fn severity_parse_titlecase() {
        assert_eq!(FindingSeverity::parse("Critical"), Some(FindingSeverity::Critical));
        assert_eq!(FindingSeverity::parse("HIGH"), Some(FindingSeverity::High));
        assert_eq!(FindingSeverity::parse("low"), Some(FindingSeverity::Low));
    }

    #[test]
    fn severity_parse_aliases() {
        // DefectDojo accepts ERROR/WARNING from many SAST tools.
        assert_eq!(FindingSeverity::parse("ERROR"), Some(FindingSeverity::High));
        assert_eq!(FindingSeverity::parse("warning"), Some(FindingSeverity::Medium));
        assert_eq!(FindingSeverity::parse("Moderate"), Some(FindingSeverity::Medium));
        assert_eq!(FindingSeverity::parse("Unknown"), Some(FindingSeverity::Info));
    }

    #[test]
    fn severity_parse_rejects_garbage() {
        assert_eq!(FindingSeverity::parse("garbage"), None);
        assert_eq!(FindingSeverity::parse(""), None);
    }

    #[test]
    fn severity_as_str_titlecase() {
        assert_eq!(FindingSeverity::Critical.as_str(), "Critical");
        assert_eq!(FindingSeverity::Info.as_str(), "Info");
    }

    #[test]
    fn finding_new_defaults_open_state() {
        let f = Finding::new("XSS in login form", FindingSeverity::High);
        assert_eq!(f.title, "XSS in login form");
        assert_eq!(f.severity, FindingSeverity::High);
        assert!(f.state.active);
        assert!(!f.state.verified);
        assert!(!f.state.duplicate);
        assert!(!f.state.false_p);
        assert!(!f.state.risk_accepted);
        assert!(!f.state.is_mitigated);
        assert_eq!(f.nb_occurences, 1);
    }

    #[test]
    fn finding_transition_to_mitigated_sets_timestamp() {
        let mut f = Finding::new("Hardcoded secret", FindingSeverity::Critical);
        f.transition(StateTransition::Verify, "alice").unwrap();
        f.transition(StateTransition::Mitigate, "alice").unwrap();
        assert!(f.state.is_mitigated);
        assert!(f.mitigated.is_some());
        assert!(!f.state.active);
    }

    #[test]
    fn finding_roundtrip_serde() {
        let mut f = Finding::new("Test", FindingSeverity::Medium);
        f.cve = Some("CVE-2024-99999".into());
        f.cwe = Some(79);
        f.cvssv3 = Some("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H".into());
        let j = serde_json::to_string(&f).unwrap();
        let back: Finding = serde_json::from_str(&j).unwrap();
        assert_eq!(f, back);
    }
}
