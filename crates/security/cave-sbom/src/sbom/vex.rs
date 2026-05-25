// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/parser/cyclonedx/CycloneDXVexImporter.java
//
//! CycloneDX VEX importer — deep port of the `vulnerabilities[]` block.
//!
//! VEX (Vulnerability Exploitability eXchange) lets vendors publish a per-CVE
//! analysis on top of an SBOM. The importer reads CycloneDX 1.5/1.6
//! `vulnerabilities[]` records and projects them onto cave-sbom's
//! `AnalysisState` enum + a triage note.

use crate::models::AnalysisState;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VexState {
    Resolved,
    ResolvedWithPedigree,
    Exploitable,
    InTriage,
    FalsePositive,
    NotAffected,
}

impl VexState {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "resolved" => Self::Resolved,
            "resolved_with_pedigree" | "resolved-with-pedigree" => Self::ResolvedWithPedigree,
            "exploitable" => Self::Exploitable,
            "in_triage" | "in-triage" => Self::InTriage,
            "false_positive" | "false-positive" => Self::FalsePositive,
            "not_affected" | "not-affected" => Self::NotAffected,
            _ => return None,
        })
    }

    pub fn to_analysis_state(&self) -> AnalysisState {
        match self {
            Self::Resolved | Self::ResolvedWithPedigree => AnalysisState::Resolved,
            Self::Exploitable => AnalysisState::Exploitable,
            Self::FalsePositive => AnalysisState::FalsePositive,
            Self::NotAffected => AnalysisState::NotAffected,
            Self::InTriage => AnalysisState::InTriage,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VexJustification {
    CodeNotPresent,
    CodeNotReachable,
    RequiresConfiguration,
    RequiresDependency,
    RequiresEnvironment,
    ProtectedByCompiler,
    ProtectedAtRuntime,
    ProtectedAtPerimeter,
    ProtectedByMitigatingControl,
}

impl VexJustification {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "code_not_present" => Self::CodeNotPresent,
            "code_not_reachable" => Self::CodeNotReachable,
            "requires_configuration" => Self::RequiresConfiguration,
            "requires_dependency" => Self::RequiresDependency,
            "requires_environment" => Self::RequiresEnvironment,
            "protected_by_compiler" => Self::ProtectedByCompiler,
            "protected_at_runtime" => Self::ProtectedAtRuntime,
            "protected_at_perimeter" => Self::ProtectedAtPerimeter,
            "protected_by_mitigating_control" => Self::ProtectedByMitigatingControl,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VexAnalysis {
    pub state: VexState,
    pub justification: Option<VexJustification>,
    #[serde(default)]
    pub responses: Vec<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VexAffectedComponent {
    pub bom_ref: String,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VexStatement {
    pub bom_ref: Option<String>,
    pub cve: Option<String>,
    pub source: Option<String>,
    pub description: Option<String>,
    pub analysis: Option<VexAnalysis>,
    #[serde(default)]
    pub affects: Vec<VexAffectedComponent>,
}

#[derive(Debug, thiserror::Error)]
pub enum VexImportError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CycloneDX document missing `vulnerabilities` array")]
    MissingVulnerabilities,
}

/// Top-level entry: import the `vulnerabilities[]` block of a CycloneDX BOM.
pub fn import_vex(json: &str) -> Result<Vec<VexStatement>, VexImportError> {
    let v: Value = serde_json::from_str(json)?;
    let arr = v
        .get("vulnerabilities")
        .and_then(Value::as_array)
        .ok_or(VexImportError::MissingVulnerabilities)?;
    Ok(arr.iter().map(parse_statement).collect())
}

fn parse_statement(v: &Value) -> VexStatement {
    let bom_ref = v.get("bom-ref").and_then(Value::as_str).map(String::from);
    let cve = v.get("id").and_then(Value::as_str).map(String::from);
    let source = v
        .get("source")
        .and_then(|s| s.get("name").and_then(Value::as_str))
        .map(String::from);
    let description = v.get("description").and_then(Value::as_str).map(String::from);
    let analysis = v.get("analysis").map(parse_analysis);
    let affects = v
        .get("affects")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(parse_affect).collect())
        .unwrap_or_default();
    VexStatement {
        bom_ref,
        cve,
        source,
        description,
        analysis,
        affects,
    }
}

fn parse_analysis(v: &Value) -> VexAnalysis {
    let state = v
        .get("state")
        .and_then(Value::as_str)
        .and_then(VexState::parse)
        .unwrap_or(VexState::InTriage);
    let justification = v
        .get("justification")
        .and_then(Value::as_str)
        .and_then(VexJustification::parse);
    let responses = v
        .get("response")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let detail = v.get("detail").and_then(Value::as_str).map(String::from);
    VexAnalysis {
        state,
        justification,
        responses,
        detail,
    }
}

fn parse_affect(v: &Value) -> Option<VexAffectedComponent> {
    let bom_ref = v.get("ref").and_then(Value::as_str)?.to_string();
    let versions = v
        .get("versions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    x.get("version")
                        .or_else(|| x.get("range"))
                        .and_then(Value::as_str)
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(VexAffectedComponent { bom_ref, versions })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_complete_resolved_statement() {
        let json = r#"{
            "vulnerabilities": [{
                "bom-ref": "vex-1",
                "id": "CVE-2023-12345",
                "source": {"name": "NVD"},
                "description": "Use after free",
                "analysis": {
                    "state": "resolved",
                    "justification": "code_not_reachable",
                    "response": ["update", "rollback"],
                    "detail": "Upgraded library"
                },
                "affects": [{
                    "ref": "pkg:npm/foo@1.0.0",
                    "versions": [{"version": "1.0.0"}]
                }]
            }]
        }"#;
        let v = import_vex(json).unwrap();
        assert_eq!(v.len(), 1);
        let stmt = &v[0];
        assert_eq!(stmt.bom_ref.as_deref(), Some("vex-1"));
        assert_eq!(stmt.cve.as_deref(), Some("CVE-2023-12345"));
        assert_eq!(stmt.source.as_deref(), Some("NVD"));
        assert_eq!(stmt.description.as_deref(), Some("Use after free"));
        let an = stmt.analysis.as_ref().unwrap();
        assert_eq!(an.state, VexState::Resolved);
        assert_eq!(an.justification, Some(VexJustification::CodeNotReachable));
        assert_eq!(an.responses, vec!["update", "rollback"]);
        assert_eq!(stmt.affects.len(), 1);
        assert_eq!(stmt.affects[0].bom_ref, "pkg:npm/foo@1.0.0");
    }

    #[test]
    fn analysis_state_projection_for_each_vex_state() {
        assert_eq!(VexState::Resolved.to_analysis_state(), AnalysisState::Resolved);
        assert_eq!(
            VexState::ResolvedWithPedigree.to_analysis_state(),
            AnalysisState::Resolved
        );
        assert_eq!(
            VexState::Exploitable.to_analysis_state(),
            AnalysisState::Exploitable
        );
        assert_eq!(
            VexState::FalsePositive.to_analysis_state(),
            AnalysisState::FalsePositive
        );
        assert_eq!(
            VexState::NotAffected.to_analysis_state(),
            AnalysisState::NotAffected
        );
        assert_eq!(VexState::InTriage.to_analysis_state(), AnalysisState::InTriage);
    }

    #[test]
    fn parse_dash_and_underscore_states() {
        assert_eq!(VexState::parse("not-affected"), Some(VexState::NotAffected));
        assert_eq!(VexState::parse("not_affected"), Some(VexState::NotAffected));
        assert_eq!(VexState::parse("FALSE-POSITIVE"), Some(VexState::FalsePositive));
    }

    #[test]
    fn parse_unknown_state_returns_none() {
        assert!(VexState::parse("garbage").is_none());
    }

    #[test]
    fn parse_unknown_justification_returns_none() {
        assert!(VexJustification::parse("hand_wave").is_none());
    }

    #[test]
    fn missing_vulnerabilities_errors() {
        let err = import_vex(r#"{"bomFormat":"CycloneDX"}"#).unwrap_err();
        assert!(matches!(err, VexImportError::MissingVulnerabilities));
    }

    #[test]
    fn invalid_json_errors() {
        assert!(matches!(import_vex("not json").unwrap_err(), VexImportError::Json(_)));
    }

    #[test]
    fn empty_vulnerabilities_array() {
        let v = import_vex(r#"{"vulnerabilities":[]}"#).unwrap();
        assert!(v.is_empty());
    }

    #[test]
    fn missing_analysis_yields_none() {
        let stmt = &import_vex(r#"{"vulnerabilities":[{"id":"CVE-A"}]}"#).unwrap()[0];
        assert!(stmt.analysis.is_none());
        assert_eq!(stmt.cve.as_deref(), Some("CVE-A"));
    }

    #[test]
    fn affects_range_is_parsed() {
        let json = r#"{
            "vulnerabilities":[{
                "id":"CVE-X",
                "affects":[{"ref":"r1","versions":[{"range":">=1.0,<2.0"}]}]
            }]
        }"#;
        let stmt = &import_vex(json).unwrap()[0];
        assert_eq!(stmt.affects[0].versions, vec![">=1.0,<2.0"]);
    }

    #[test]
    fn analysis_serde_roundtrip() {
        let an = VexAnalysis {
            state: VexState::Resolved,
            justification: Some(VexJustification::CodeNotPresent),
            responses: vec!["update".into()],
            detail: Some("note".into()),
        };
        let j = serde_json::to_string(&an).unwrap();
        let back: VexAnalysis = serde_json::from_str(&j).unwrap();
        assert_eq!(back, an);
    }
}
