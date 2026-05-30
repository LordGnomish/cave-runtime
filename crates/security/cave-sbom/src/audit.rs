// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: DependencyTrack/dependency-track@128fd0fa01bed9fcb57abffa3b30047c45941415
//   src/main/java/org/dependencytrack/model/{Analysis,AnalysisState,AnalysisJustification,AnalysisResponse,AnalysisComment}.java
//   src/main/java/org/dependencytrack/util/AnalysisCommentUtil.java
//   src/main/java/org/dependencytrack/resources/v1/AnalysisResource.java
//
//! Vulnerability audit workflow — the per-(project, component, vulnerability)
//! analysis decision with an append-only comment trail and suppression.

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
