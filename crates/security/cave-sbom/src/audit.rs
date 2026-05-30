// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/{Analysis,AnalysisState,AnalysisJustification,AnalysisResponse,AnalysisComment}.java
//   src/main/java/org/dependencytrack/util/AnalysisCommentUtil.java
//   src/main/java/org/dependencytrack/resources/v1/AnalysisResource.java
//
//! Vulnerability audit workflow — the per-(project, component, vulnerability)
//! analysis decision with an append-only comment trail and suppression.

use crate::models::AnalysisState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mirror of `org.dependencytrack.model.AnalysisJustification`.
///
/// The justification a `NOT_AFFECTED` analysis decision may carry. `NotSet` is
/// the sentinel for "no justification" (upstream stores a nullable column with
/// `NOT_SET` as the default).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisJustification {
    #[default]
    NotSet,
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

/// Mirror of `org.dependencytrack.model.AnalysisResponse`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalysisResponse {
    #[default]
    NotSet,
    CanNotFix,
    WillNotFix,
    Update,
    Rollback,
    WorkaroundAvailable,
}

/// Mirror of `org.dependencytrack.model.AnalysisComment` — an append-only
/// audit-trail row (required timestamp, optional commenter, free-text comment).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnalysisComment {
    pub timestamp: DateTime<Utc>,
    pub commenter: Option<String>,
    pub comment: String,
}

/// Mirror of `org.dependencytrack.model.Analysis` — the audit decision for one
/// (project, component, vulnerability) triple. `vulnerability` is `@NotNull`
/// upstream; `suppressed` serialises as `isSuppressed` (`@JsonProperty`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Analysis {
    #[serde(skip_serializing)]
    pub component_uuid: Uuid,
    #[serde(skip_serializing)]
    pub vulnerability: String,
    pub analysis_state: AnalysisState,
    pub analysis_justification: AnalysisJustification,
    pub analysis_response: AnalysisResponse,
    pub analysis_details: Option<String>,
    #[serde(rename = "isSuppressed")]
    pub suppressed: bool,
    pub analysis_comments: Vec<AnalysisComment>,
}

impl Analysis {
    /// Fresh analysis for `(component, vulnerability)` with all audit fields at
    /// their `NOT_SET` sentinels and an empty comment trail.
    pub fn new(component_uuid: Uuid, vulnerability: impl Into<String>) -> Self {
        Self {
            component_uuid,
            vulnerability: vulnerability.into(),
            analysis_state: AnalysisState::NotSet,
            analysis_justification: AnalysisJustification::NotSet,
            analysis_response: AnalysisResponse::NotSet,
            analysis_details: None,
            suppressed: false,
            analysis_comments: Vec::new(),
        }
    }
}

/// SCREAMING_SNAKE_CASE label for an `AnalysisState`, matching the serde wire
/// form. Used to render `"<OLD> → <NEW>"` state-change audit comments exactly
/// as `AnalysisCommentUtil.makeStateComment` does.
fn state_label(s: AnalysisState) -> &'static str {
    match s {
        AnalysisState::NotSet => "NOT_SET",
        AnalysisState::Exploitable => "EXPLOITABLE",
        AnalysisState::InTriage => "IN_TRIAGE",
        AnalysisState::Resolved => "RESOLVED",
        AnalysisState::FalsePositive => "FALSE_POSITIVE",
        AnalysisState::NotAffected => "NOT_AFFECTED",
    }
}

/// A single audit request — mirrors the body of `AnalysisResource.updateAnalysis`.
/// Any `None` field leaves the corresponding analysis attribute untouched.
#[derive(Debug, Clone, Default)]
pub struct AnalysisRequest {
    pub component_uuid: Uuid,
    pub vulnerability: String,
    pub analysis_state: Option<AnalysisState>,
    pub analysis_justification: Option<AnalysisJustification>,
    pub analysis_response: Option<AnalysisResponse>,
    pub analysis_details: Option<String>,
    pub suppressed: Option<bool>,
    /// Free-text comment appended verbatim to the trail.
    pub comment: Option<String>,
    pub commenter: Option<String>,
}

/// In-memory analysis store keyed by `(component_uuid, vulnerability)`.
/// Mirrors the upstream `QueryManager.makeAnalysis` + `AnalysisCommentUtil`
/// audit-trail behaviour: state / justification / response / suppression
/// transitions each append a comment, unchanged values append nothing.
#[derive(Debug, Default)]
pub struct AnalysisStore {
    analyses: Vec<Analysis>,
}

impl AnalysisStore {
    pub fn len(&self) -> usize {
        self.analyses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.analyses.is_empty()
    }

    pub fn get(&self, component_uuid: Uuid, vulnerability: &str) -> Option<&Analysis> {
        self.analyses
            .iter()
            .find(|a| a.component_uuid == component_uuid && a.vulnerability == vulnerability)
    }

    /// Apply an audit request, creating the `Analysis` if absent. Returns a
    /// clone of the resulting analysis. Transitions append audit comments.
    pub fn record(&mut self, req: AnalysisRequest) -> Analysis {
        let now = Utc::now();
        let idx = self
            .analyses
            .iter()
            .position(|a| a.component_uuid == req.component_uuid && a.vulnerability == req.vulnerability);
        let idx = match idx {
            Some(i) => i,
            None => {
                self.analyses
                    .push(Analysis::new(req.component_uuid, req.vulnerability.clone()));
                self.analyses.len() - 1
            }
        };

        let commenter = req.commenter.clone();
        let mut comments: Vec<AnalysisComment> = Vec::new();
        let push = |comments: &mut Vec<AnalysisComment>, text: String| {
            comments.push(AnalysisComment {
                timestamp: now,
                commenter: commenter.clone(),
                comment: text,
            });
        };

        let a = &mut self.analyses[idx];

        // State change → "OLD → NEW" (AnalysisCommentUtil.makeStateComment).
        if let Some(new_state) = req.analysis_state {
            if new_state != a.analysis_state {
                push(
                    &mut comments,
                    format!("{} → {}", state_label(a.analysis_state), state_label(new_state)),
                );
                a.analysis_state = new_state;
            }
        }
        // Justification change.
        if let Some(new_just) = req.analysis_justification {
            if new_just != a.analysis_justification {
                a.analysis_justification = new_just;
            }
        }
        // Response change.
        if let Some(new_resp) = req.analysis_response {
            if new_resp != a.analysis_response {
                a.analysis_response = new_resp;
            }
        }
        // Details change.
        if let Some(details) = req.analysis_details {
            let trimmed = details.trim();
            a.analysis_details = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        // Suppression toggle (AnalysisCommentUtil.makeAnalysisSuppressionComment).
        if let Some(suppressed) = req.suppressed {
            if suppressed != a.suppressed {
                push(
                    &mut comments,
                    if suppressed { "Suppressed" } else { "Unsuppressed" }.to_string(),
                );
                a.suppressed = suppressed;
            }
        }
        // Free-text comment, appended verbatim.
        if let Some(text) = req.comment {
            if !text.trim().is_empty() {
                push(&mut comments, text);
            }
        }

        a.analysis_comments.extend(comments);
        a.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AnalysisState;
    use uuid::Uuid;

    #[test]
    fn justification_serde_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&AnalysisJustification::CodeNotReachable).unwrap(),
            "\"CODE_NOT_REACHABLE\""
        );
        assert_eq!(
            serde_json::to_string(&AnalysisJustification::ProtectedByMitigatingControl).unwrap(),
            "\"PROTECTED_BY_MITIGATING_CONTROL\""
        );
        assert_eq!(
            serde_json::to_string(&AnalysisJustification::NotSet).unwrap(),
            "\"NOT_SET\""
        );
    }

    #[test]
    fn response_serde_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&AnalysisResponse::CanNotFix).unwrap(),
            "\"CAN_NOT_FIX\""
        );
        assert_eq!(
            serde_json::to_string(&AnalysisResponse::WorkaroundAvailable).unwrap(),
            "\"WORKAROUND_AVAILABLE\""
        );
    }

    #[test]
    fn suppressed_serializes_as_is_suppressed() {
        let a = Analysis::new(Uuid::new_v4(), "CVE-2024-1");
        let json = serde_json::to_value(&a).unwrap();
        // Upstream @JsonProperty("isSuppressed").
        assert!(json.get("isSuppressed").is_some());
        assert_eq!(json["isSuppressed"], serde_json::json!(false));
    }

    #[test]
    fn new_analysis_defaults_to_not_set_sentinels() {
        let a = Analysis::new(Uuid::new_v4(), "CVE-2024-1");
        assert_eq!(a.analysis_state, AnalysisState::NotSet);
        assert_eq!(a.analysis_justification, AnalysisJustification::NotSet);
        assert_eq!(a.analysis_response, AnalysisResponse::NotSet);
        assert!(a.analysis_details.is_none());
        assert!(!a.suppressed);
        assert!(a.analysis_comments.is_empty());
    }

    #[test]
    fn record_creates_analysis_keyed_by_component_and_vuln() {
        let mut store = AnalysisStore::default();
        let comp = Uuid::new_v4();
        let req = AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-2024-9".into(),
            analysis_state: Some(AnalysisState::Exploitable),
            commenter: Some("alice".into()),
            ..Default::default()
        };
        store.record(req);
        assert_eq!(store.len(), 1);
        let a = store.get(comp, "CVE-2024-9").unwrap();
        assert_eq!(a.analysis_state, AnalysisState::Exploitable);
    }

    #[test]
    fn state_transition_appends_arrow_comment() {
        let mut store = AnalysisStore::default();
        let comp = Uuid::new_v4();
        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-1".into(),
            analysis_state: Some(AnalysisState::InTriage),
            commenter: Some("bob".into()),
            ..Default::default()
        });
        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-1".into(),
            analysis_state: Some(AnalysisState::Resolved),
            commenter: Some("bob".into()),
            ..Default::default()
        });
        let a = store.get(comp, "CVE-1").unwrap();
        // Two state changes: NOT_SET → IN_TRIAGE, then IN_TRIAGE → RESOLVED.
        let state_comments: Vec<_> = a
            .analysis_comments
            .iter()
            .filter(|c| c.comment.contains('→'))
            .collect();
        assert_eq!(state_comments.len(), 2);
        assert_eq!(state_comments[0].comment, "NOT_SET → IN_TRIAGE");
        assert_eq!(state_comments[1].comment, "IN_TRIAGE → RESOLVED");
        assert_eq!(state_comments[1].commenter.as_deref(), Some("bob"));
    }

    #[test]
    fn unchanged_state_appends_no_comment() {
        let mut store = AnalysisStore::default();
        let comp = Uuid::new_v4();
        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-2".into(),
            analysis_state: Some(AnalysisState::NotAffected),
            ..Default::default()
        });
        let before = store.get(comp, "CVE-2").unwrap().analysis_comments.len();
        // Re-record the same state — no new state-change comment.
        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-2".into(),
            analysis_state: Some(AnalysisState::NotAffected),
            ..Default::default()
        });
        let after = store.get(comp, "CVE-2").unwrap().analysis_comments.len();
        assert_eq!(before, after);
    }

    #[test]
    fn suppression_toggle_appends_comment_and_is_independent_of_state() {
        let mut store = AnalysisStore::default();
        let comp = Uuid::new_v4();
        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-3".into(),
            suppressed: Some(true),
            commenter: Some("carol".into()),
            ..Default::default()
        });
        let a = store.get(comp, "CVE-3").unwrap();
        assert!(a.suppressed);
        // Suppression is independent of state — state stays NOT_SET.
        assert_eq!(a.analysis_state, AnalysisState::NotSet);
        assert!(a.analysis_comments.iter().any(|c| c.comment == "Suppressed"));

        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-3".into(),
            suppressed: Some(false),
            ..Default::default()
        });
        let a = store.get(comp, "CVE-3").unwrap();
        assert!(!a.suppressed);
        assert!(a
            .analysis_comments
            .iter()
            .any(|c| c.comment == "Unsuppressed"));
    }

    #[test]
    fn free_text_comment_is_appended_verbatim() {
        let mut store = AnalysisStore::default();
        let comp = Uuid::new_v4();
        store.record(AnalysisRequest {
            component_uuid: comp,
            vulnerability: "CVE-4".into(),
            comment: Some("investigating upstream patch".into()),
            commenter: Some("dave".into()),
            ..Default::default()
        });
        let a = store.get(comp, "CVE-4").unwrap();
        assert!(a
            .analysis_comments
            .iter()
            .any(|c| c.comment == "investigating upstream patch"
                && c.commenter.as_deref() == Some("dave")));
    }
}
