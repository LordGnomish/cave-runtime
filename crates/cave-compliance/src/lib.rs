//! CAVE Compliance — SOC2 Type II and ISO 27001 compliance management.
//!
//! Replaces: Vanta, Drata, Tugboat Logic
//! Automated compliance tracking, control mapping, evidence collection.

pub mod frameworks;
pub mod models;
pub mod monitor;
pub mod routes;
pub mod store;

use axum::Router;
use std::sync::Arc;

pub use store::ComplianceStore;

pub struct ComplianceState {
    pub store: Arc<ComplianceStore>,
}

impl Default for ComplianceState {
    fn default() -> Self {
        Self {
            store: Arc::new(ComplianceStore::new()),
        }
    }
}

pub fn router(state: Arc<ComplianceState>) -> Router {
    routes::create_router(state.store.clone())
}

pub const MODULE_NAME: &str = "compliance";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;
    use crate::monitor::ComplianceMonitor;
    use chrono::Utc;
    use uuid::Uuid;

    // ── Control catalog tests ─────────────────────────────────────────────────

    #[test]
    fn soc2_catalog_has_controls() {
        let controls = frameworks::soc2_controls();
        assert!(controls.len() >= 15, "SOC2 should have at least 15 controls");
        // All controls should be SOC2 framework
        for c in &controls {
            assert_eq!(c.framework, Framework::Soc2TypeII);
        }
    }

    #[test]
    fn iso27001_catalog_has_controls() {
        let controls = frameworks::iso27001_controls();
        assert!(controls.len() >= 15, "ISO27001 should have at least 15 controls");
        for c in &controls {
            assert_eq!(c.framework, Framework::Iso27001);
        }
    }

    #[test]
    fn all_controls_combines_both_frameworks() {
        let all = frameworks::all_controls();
        let soc2 = frameworks::soc2_controls();
        let iso = frameworks::iso27001_controls();
        assert_eq!(all.len(), soc2.len() + iso.len());
    }

    #[test]
    fn get_control_by_id() {
        let control = frameworks::get_control("CC6.1");
        assert!(control.is_some());
        assert_eq!(control.unwrap().framework, Framework::Soc2TypeII);
    }

    #[test]
    fn get_control_iso_by_id() {
        let control = frameworks::get_control("A.9.1.1");
        assert!(control.is_some());
        assert_eq!(control.unwrap().framework, Framework::Iso27001);
    }

    #[test]
    fn unknown_control_returns_none() {
        assert!(frameworks::get_control("NONEXISTENT").is_none());
    }

    // ── Cross-framework mapping tests ─────────────────────────────────────────

    #[test]
    fn cc6_1_maps_to_iso_a9_1_1() {
        let mappings = frameworks::get_mappings("CC6.1");
        assert!(!mappings.is_empty(), "CC6.1 should have mappings");
        let iso_mapping = mappings.iter().find(|m| m.control_id == "A.9.1.1");
        assert!(iso_mapping.is_some(), "CC6.1 should map to A.9.1.1");
    }

    #[test]
    fn iso_a9_1_1_maps_back_to_soc2() {
        let mappings = frameworks::get_mappings("A.9.1.1");
        assert!(!mappings.is_empty(), "A.9.1.1 should have SOC2 mappings");
        let soc2_mapping = mappings.iter().find(|m| m.framework == Framework::Soc2TypeII);
        assert!(soc2_mapping.is_some());
    }

    // ── Effectiveness score tests ─────────────────────────────────────────────

    #[test]
    fn effectiveness_score_empty_assessments() {
        let score = ComplianceMonitor::effectiveness_score(&[], Framework::Soc2TypeII);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn effectiveness_score_calculates_correctly() {
        let assessments = vec![
            make_assessment("CC6.1", ControlStatus::Implemented, 0.8),
            make_assessment("CC7.1", ControlStatus::Tested, 0.9),
        ];
        let score = ComplianceMonitor::effectiveness_score(&assessments, Framework::Soc2TypeII);
        let expected = (0.8 + 0.9) / 2.0;
        assert!((score - expected).abs() < 0.001, "score={score}, expected={expected}");
    }

    #[test]
    fn effectiveness_score_filters_by_framework() {
        let assessments = vec![
            make_assessment("CC6.1", ControlStatus::Implemented, 1.0),  // SOC2
            make_assessment("A.9.1.1", ControlStatus::Implemented, 0.0), // ISO — should be excluded
        ];
        let score = ComplianceMonitor::effectiveness_score(&assessments, Framework::Soc2TypeII);
        // Only CC6.1 (1.0) counted for SOC2
        assert!((score - 1.0).abs() < 0.001);
    }

    // ── Gap identification tests ──────────────────────────────────────────────

    #[test]
    fn gap_identified_for_unassessed_control() {
        let controls = frameworks::soc2_controls();
        let gaps = ComplianceMonitor::identify_gaps(&controls, &[]);
        assert_eq!(gaps.len(), controls.len(), "All controls should be gaps when no assessments");
    }

    #[test]
    fn fully_implemented_control_not_a_gap() {
        let controls = vec![frameworks::get_control("CC6.1").unwrap()];
        let assessments = vec![make_assessment("CC6.1", ControlStatus::Implemented, 0.95)];
        let gaps = ComplianceMonitor::identify_gaps(&controls, &assessments);
        assert!(gaps.is_empty(), "No gaps expected for fully-implemented control");
    }

    #[test]
    fn low_effectiveness_generates_gap() {
        let controls = vec![frameworks::get_control("CC6.1").unwrap()];
        let assessments = vec![make_assessment("CC6.1", ControlStatus::Implemented, 0.4)];
        let gaps = ComplianceMonitor::identify_gaps(&controls, &assessments);
        assert_eq!(gaps.len(), 1);
    }

    // ── Risk score tests ──────────────────────────────────────────────────────

    #[test]
    fn risk_score_is_likelihood_times_impact() {
        let store = ComplianceStore::new();
        let risk = store.create_risk(CreateRiskRequest {
            title: "Test Risk".into(),
            description: "A test risk".into(),
            category: RiskCategory::Technical,
            likelihood: 3,
            impact: 4,
            treatment: RiskTreatment::Mitigate,
            treatment_plan: None,
            owner: Uuid::new_v4(),
            control_ids: None,
        });
        assert_eq!(risk.risk_score, 12.0, "3 × 4 = 12");
    }

    #[test]
    fn risk_score_max_is_25() {
        let store = ComplianceStore::new();
        let risk = store.create_risk(CreateRiskRequest {
            title: "Max Risk".into(),
            description: "Maximum risk".into(),
            category: RiskCategory::Compliance,
            likelihood: 5,
            impact: 5,
            treatment: RiskTreatment::Avoid,
            treatment_plan: None,
            owner: Uuid::new_v4(),
            control_ids: None,
        });
        assert_eq!(risk.risk_score, 25.0);
    }

    // ── Policy tests ──────────────────────────────────────────────────────────

    #[test]
    fn create_and_acknowledge_policy() {
        let store = ComplianceStore::new();
        let policy = store.create_policy(CreatePolicyRequest {
            title: "Security Policy".into(),
            version: "1.0".into(),
            content: "All access must be logged.".into(),
            owner: Uuid::new_v4(),
            effective_date: None,
            review_date: None,
        });
        assert!(policy.acknowledgments.is_empty());

        let user_id = Uuid::new_v4();
        let updated = store.acknowledge_policy(policy.id, user_id);
        assert!(updated.is_some());
        let updated = updated.unwrap();
        assert_eq!(updated.acknowledgments.len(), 1);
        assert_eq!(updated.acknowledgments[0].user_id, user_id);
        assert_eq!(updated.acknowledgments[0].version, "1.0");
    }

    #[test]
    fn acknowledge_nonexistent_policy_returns_none() {
        let store = ComplianceStore::new();
        let result = store.acknowledge_policy(Uuid::new_v4(), Uuid::new_v4());
        assert!(result.is_none());
    }

    // ── Audit log tests ───────────────────────────────────────────────────────

    #[test]
    fn audit_log_always_appends() {
        let store = ComplianceStore::new();
        for i in 0..5u8 {
            store.append_audit_event(AuditEvent {
                id: Uuid::new_v4(),
                event_type: "test.event".into(),
                description: format!("event {i}"),
                actor: Uuid::new_v4(),
                resource_type: "policy".into(),
                resource_id: i.to_string(),
                old_value: None,
                new_value: None,
                ip_address: None,
                created_at: Utc::now(),
            });
        }
        assert_eq!(store.get_audit_log(0).len(), 5);
    }

    #[test]
    fn audit_log_limit_returns_last_n() {
        let store = ComplianceStore::new();
        for i in 0..10u8 {
            store.append_audit_event(AuditEvent {
                id: Uuid::new_v4(),
                event_type: "test".into(),
                description: format!("event {i}"),
                actor: Uuid::new_v4(),
                resource_type: "risk".into(),
                resource_id: i.to_string(),
                old_value: None,
                new_value: None,
                ip_address: None,
                created_at: Utc::now(),
            });
        }
        let last3 = store.get_audit_log(3);
        assert_eq!(last3.len(), 3);
    }

    // ── Compliance summary test ───────────────────────────────────────────────

    #[test]
    fn compliance_summary_totals_correctly() {
        let store = ComplianceStore::new();
        store.upsert_assessment(make_assessment("CC6.1", ControlStatus::Implemented, 0.9));
        store.upsert_assessment(make_assessment("CC7.1", ControlStatus::Tested, 1.0));

        let summary = store.compliance_summary(Framework::Soc2TypeII);
        assert_eq!(summary.framework, Framework::Soc2TypeII);
        assert!(summary.total_controls >= 15);
        assert_eq!(summary.implemented, 2);
        assert_eq!(summary.tested, 1);
        assert!(summary.effectiveness_score > 0.0);
    }

    // ── Evidence tests ────────────────────────────────────────────────────────

    #[test]
    fn evidence_associated_with_control() {
        let store = ComplianceStore::new();
        store.add_evidence(Evidence {
            id: Uuid::new_v4(),
            control_id: "CC6.1".into(),
            evidence_type: EvidenceType::AccessLog,
            title: "Q1 Access Log Export".into(),
            description: "Export of access logs for Q1 2026".into(),
            source_module: Some("cave-pam".into()),
            content: serde_json::json!({"entries": 1234}),
            collected_at: Utc::now(),
            collected_by: Uuid::new_v4(),
            valid_until: None,
            is_automated: true,
        });

        let evidence = store.list_evidence(Some("CC6.1"));
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].control_id, "CC6.1");
    }

    #[test]
    fn evidence_filtered_by_control_id() {
        let store = ComplianceStore::new();
        store.add_evidence(Evidence {
            id: Uuid::new_v4(),
            control_id: "CC6.1".into(),
            evidence_type: EvidenceType::AccessLog,
            title: "CC6.1 evidence".into(),
            description: "".into(),
            source_module: None,
            content: serde_json::json!({}),
            collected_at: Utc::now(),
            collected_by: Uuid::new_v4(),
            valid_until: None,
            is_automated: false,
        });
        store.add_evidence(Evidence {
            id: Uuid::new_v4(),
            control_id: "CC7.1".into(),
            evidence_type: EvidenceType::ScanResult,
            title: "CC7.1 evidence".into(),
            description: "".into(),
            source_module: None,
            content: serde_json::json!({}),
            collected_at: Utc::now(),
            collected_by: Uuid::new_v4(),
            valid_until: None,
            is_automated: true,
        });

        let cc6_evidence = store.list_evidence(Some("CC6.1"));
        assert_eq!(cc6_evidence.len(), 1);
        let all_evidence = store.list_evidence(None);
        assert_eq!(all_evidence.len(), 2);
    }

    // ── Vendor risk tier tests ────────────────────────────────────────────────

    #[test]
    fn vendor_risk_tier_stored_correctly() {
        let store = ComplianceStore::new();
        let va = store.create_vendor_assessment(VendorQuestionnaireRequest {
            vendor_name: "Acme Cloud".into(),
            vendor_url: Some("https://acme.example.com".into()),
            risk_tier: VendorRiskTier::Critical,
            questionnaire_responses: serde_json::json!({"soc2_report": true}),
            reviewed_by: None,
            next_review: None,
        });
        assert_eq!(va.risk_tier, VendorRiskTier::Critical);
        assert_eq!(va.vendor_name, "Acme Cloud");

        let vendors = store.list_vendors();
        assert_eq!(vendors.len(), 1);
        assert_eq!(vendors[0].risk_tier, VendorRiskTier::Critical);
    }

    // ── Helper ────────────────────────────────────────────────────────────────

    fn make_assessment(
        control_id: &str,
        status: ControlStatus,
        score: f32,
    ) -> ControlAssessment {
        ControlAssessment {
            id: Uuid::new_v4(),
            control_id: control_id.into(),
            status,
            effectiveness_score: score,
            gaps: Vec::new(),
            evidence_ids: Vec::new(),
            assessor: Uuid::new_v4(),
            assessed_at: Utc::now(),
            next_review_date: None,
        }
    }
}
