// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CycloneDX VEX (Vulnerability Exploitability eXchange) export.
//!
//! Spec: <https://cyclonedx.org/capabilities/vex/>.
//! Upstream: `resources/v1/VexResource` + `parser.cyclonedx.CycloneDxVexParser`.

use crate::audit::Analysis;
use crate::models::{AnalysisJustification, AnalysisResponse, AnalysisState, Vulnerability};
use chrono::Utc;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct VexDocument {
    pub bom_format: String,
    pub spec_version: String,
    pub serial_number: String,
    pub vulnerabilities: Vec<Value>,
}

impl VexDocument {
    pub fn new() -> Self {
        Self {
            bom_format: "CycloneDX".into(),
            spec_version: "1.6".into(),
            serial_number: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            vulnerabilities: Vec::new(),
        }
    }

    pub fn push_analysis(&mut self, v: &Vulnerability, a: &Analysis) {
        let state = analysis_state_to_cdx(a.state);
        let justification = justification_to_cdx(a.justification);
        let response = response_to_cdx(a.response);
        let entry = json!({
            "id": v.vuln_id,
            "source": { "name": format!("{:?}", v.source).to_uppercase() },
            "analysis": {
                "state": state,
                "justification": justification,
                "response": response,
                "detail": a.details.clone().unwrap_or_default(),
            },
            "affects": [{"ref": a.component.to_string()}],
        });
        self.vulnerabilities.push(entry);
    }

    pub fn to_json(&self) -> Value {
        json!({
            "bomFormat": self.bom_format,
            "specVersion": self.spec_version,
            "serialNumber": self.serial_number,
            "version": 1,
            "metadata": {
                "timestamp": Utc::now().to_rfc3339(),
                "tools": [{"vendor":"Cave","name":"cave-dependency-track","version": env!("CARGO_PKG_VERSION")}]
            },
            "vulnerabilities": self.vulnerabilities,
        })
    }
}

impl Default for VexDocument {
    fn default() -> Self {
        Self::new()
    }
}

fn analysis_state_to_cdx(s: AnalysisState) -> &'static str {
    match s {
        AnalysisState::NotSet => "in_triage",
        AnalysisState::Exploitable => "exploitable",
        AnalysisState::InTriage => "in_triage",
        AnalysisState::Resolved => "resolved",
        AnalysisState::FalsePositive => "false_positive",
        AnalysisState::NotAffected => "not_affected",
    }
}

fn justification_to_cdx(j: AnalysisJustification) -> &'static str {
    match j {
        AnalysisJustification::NotSet => "",
        AnalysisJustification::CodeNotPresent => "code_not_present",
        AnalysisJustification::CodeNotReachable => "code_not_reachable",
        AnalysisJustification::RequiresConfiguration => "requires_configuration",
        AnalysisJustification::RequiresDependency => "requires_dependency",
        AnalysisJustification::RequiresEnvironment => "requires_environment",
        AnalysisJustification::ProtectedByCompiler => "protected_by_compiler",
        AnalysisJustification::ProtectedAtRuntime => "protected_at_runtime",
        AnalysisJustification::ProtectedAtPerimeter => "protected_at_perimeter",
        AnalysisJustification::ProtectedByMitigatingControl => "protected_by_mitigating_control",
    }
}

fn response_to_cdx(r: AnalysisResponse) -> &'static str {
    match r {
        AnalysisResponse::NotSet => "",
        AnalysisResponse::CanNotFix => "can_not_fix",
        AnalysisResponse::WillNotFix => "will_not_fix",
        AnalysisResponse::Update => "update",
        AnalysisResponse::RollBack => "rollback",
        AnalysisResponse::WorkaroundAvailable => "workaround_available",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VulnSource;
    use chrono::Utc;
    use uuid::Uuid;

    fn mk_analysis(state: AnalysisState) -> Analysis {
        Analysis {
            component: Uuid::new_v4(),
            vulnerability: Uuid::new_v4(),
            state,
            justification: AnalysisJustification::CodeNotReachable,
            response: AnalysisResponse::WillNotFix,
            details: Some("scoped to admin".into()),
            suppressed: false,
            last_changed: Utc::now(),
            comments: Vec::new(),
        }
    }

    #[test]
    fn vex_doc_has_required_top_level_keys() {
        let mut d = VexDocument::new();
        let v = Vulnerability::new("CVE-2026-1", VulnSource::Nvd);
        let a = mk_analysis(AnalysisState::NotAffected);
        d.push_analysis(&v, &a);
        let json = d.to_json();
        for key in ["bomFormat", "specVersion", "serialNumber", "vulnerabilities", "metadata"] {
            assert!(json.get(key).is_some(), "missing {}", key);
        }
        assert_eq!(json["bomFormat"], "CycloneDX");
    }

    #[test]
    fn analysis_state_maps_lowercase() {
        assert_eq!(analysis_state_to_cdx(AnalysisState::FalsePositive), "false_positive");
        assert_eq!(analysis_state_to_cdx(AnalysisState::NotAffected), "not_affected");
    }

    #[test]
    fn vex_entry_includes_affects_ref() {
        let mut d = VexDocument::new();
        let v = Vulnerability::new("CVE-2026-1", VulnSource::Nvd);
        let a = mk_analysis(AnalysisState::NotAffected);
        let comp_uuid = a.component;
        d.push_analysis(&v, &a);
        assert_eq!(
            d.vulnerabilities[0]["affects"][0]["ref"],
            comp_uuid.to_string()
        );
        assert_eq!(d.vulnerabilities[0]["analysis"]["state"], "not_affected");
    }

    #[test]
    fn justification_to_cdx_full_table() {
        assert_eq!(justification_to_cdx(AnalysisJustification::CodeNotPresent), "code_not_present");
        assert_eq!(justification_to_cdx(AnalysisJustification::ProtectedAtRuntime), "protected_at_runtime");
        assert_eq!(justification_to_cdx(AnalysisJustification::NotSet), "");
    }

    #[test]
    fn response_to_cdx_full_table() {
        assert_eq!(response_to_cdx(AnalysisResponse::WillNotFix), "will_not_fix");
        assert_eq!(response_to_cdx(AnalysisResponse::RollBack), "rollback");
        assert_eq!(response_to_cdx(AnalysisResponse::NotSet), "");
    }
}
