// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store for cave-compliance.

use crate::frameworks;
use crate::models::*;
use crate::monitor::ComplianceMonitor;
use chrono::Utc;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct ComplianceStore {
    assessments: Arc<Mutex<Vec<ControlAssessment>>>,
    evidence: Arc<Mutex<Vec<Evidence>>>,
    policies: Arc<Mutex<Vec<Policy>>>,
    risks: Arc<Mutex<Vec<RiskEntry>>>,
    vendors: Arc<Mutex<Vec<VendorAssessment>>>,
    audit_log: Arc<Mutex<Vec<AuditEvent>>>,
}

impl Default for ComplianceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ComplianceStore {
    pub fn new() -> Self {
        Self {
            assessments: Arc::new(Mutex::new(Vec::new())),
            evidence: Arc::new(Mutex::new(Vec::new())),
            policies: Arc::new(Mutex::new(Vec::new())),
            risks: Arc::new(Mutex::new(Vec::new())),
            vendors: Arc::new(Mutex::new(Vec::new())),
            audit_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    // ─── Controls ────────────────────────────────────────────────────────────

    pub fn list_controls(&self, framework: Option<Framework>) -> Vec<Control> {
        match framework {
            Some(Framework::Soc2TypeII) => frameworks::soc2_controls(),
            Some(Framework::Iso27001) => frameworks::iso27001_controls(),
            None => frameworks::all_controls(),
        }
    }

    pub fn get_control(&self, id: &str) -> Option<Control> {
        frameworks::get_control(id)
    }

    // ─── Assessments ─────────────────────────────────────────────────────────

    pub fn list_assessments(&self) -> Vec<ControlAssessment> {
        self.assessments.lock().unwrap().clone()
    }

    pub fn get_assessment_for_control(&self, control_id: &str) -> Option<ControlAssessment> {
        self.assessments
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.control_id == control_id)
            .cloned()
    }

    /// Insert or replace the assessment for the given control_id.
    pub fn upsert_assessment(&self, assessment: ControlAssessment) {
        let mut guard = self.assessments.lock().unwrap();
        if let Some(pos) = guard.iter().position(|a| a.control_id == assessment.control_id) {
            guard[pos] = assessment;
        } else {
            guard.push(assessment);
        }
    }

    // ─── Evidence ────────────────────────────────────────────────────────────

    pub fn list_evidence(&self, control_id: Option<&str>) -> Vec<Evidence> {
        let guard = self.evidence.lock().unwrap();
        match control_id {
            Some(id) => guard.iter().filter(|e| e.control_id == id).cloned().collect(),
            None => guard.clone(),
        }
    }

    pub fn add_evidence(&self, evidence: Evidence) {
        self.evidence.lock().unwrap().push(evidence);
    }

    // ─── Policies ────────────────────────────────────────────────────────────

    pub fn list_policies(&self) -> Vec<Policy> {
        self.policies.lock().unwrap().clone()
    }

    pub fn create_policy(&self, req: CreatePolicyRequest) -> Policy {
        let now = Utc::now();
        let policy = Policy {
            id: Uuid::new_v4(),
            title: req.title,
            version: req.version,
            content: req.content,
            status: PolicyStatus::Draft,
            owner: req.owner,
            effective_date: req.effective_date,
            review_date: req.review_date,
            acknowledgments: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.policies.lock().unwrap().push(policy.clone());
        policy
    }

    pub fn acknowledge_policy(
        &self,
        policy_id: Uuid,
        user_id: Uuid,
    ) -> Option<Policy> {
        let mut guard = self.policies.lock().unwrap();
        if let Some(policy) = guard.iter_mut().find(|p| p.id == policy_id) {
            let ack = PolicyAcknowledgment {
                user_id,
                acknowledged_at: Utc::now(),
                version: policy.version.clone(),
            };
            policy.acknowledgments.push(ack);
            policy.updated_at = Utc::now();
            Some(policy.clone())
        } else {
            None
        }
    }

    // ─── Risks ────────────────────────────────────────────────────────────────

    pub fn list_risks(&self) -> Vec<RiskEntry> {
        self.risks.lock().unwrap().clone()
    }

    pub fn create_risk(&self, req: CreateRiskRequest) -> RiskEntry {
        let now = Utc::now();
        let risk_score = req.likelihood as f32 * req.impact as f32;
        let risk = RiskEntry {
            id: Uuid::new_v4(),
            title: req.title,
            description: req.description,
            category: req.category,
            likelihood: req.likelihood,
            impact: req.impact,
            risk_score,
            treatment: req.treatment,
            treatment_plan: req.treatment_plan,
            owner: req.owner,
            control_ids: req.control_ids.unwrap_or_default(),
            status: RiskStatus::Open,
            created_at: now,
            updated_at: now,
        };
        self.risks.lock().unwrap().push(risk.clone());
        risk
    }

    pub fn update_risk(&self, id: Uuid, req: UpdateRiskRequest) -> Option<RiskEntry> {
        let mut guard = self.risks.lock().unwrap();
        if let Some(risk) = guard.iter_mut().find(|r| r.id == id) {
            if let Some(s) = req.status {
                risk.status = s;
            }
            if let Some(t) = req.treatment {
                risk.treatment = t;
            }
            if req.treatment_plan.is_some() {
                risk.treatment_plan = req.treatment_plan;
            }
            if let Some(l) = req.likelihood {
                risk.likelihood = l;
                risk.risk_score = l as f32 * risk.impact as f32;
            }
            if let Some(i) = req.impact {
                risk.impact = i;
                risk.risk_score = risk.likelihood as f32 * i as f32;
            }
            risk.updated_at = Utc::now();
            Some(risk.clone())
        } else {
            None
        }
    }

    // ─── Vendors ─────────────────────────────────────────────────────────────

    pub fn list_vendors(&self) -> Vec<VendorAssessment> {
        self.vendors.lock().unwrap().clone()
    }

    pub fn create_vendor_assessment(&self, req: VendorQuestionnaireRequest) -> VendorAssessment {
        let va = VendorAssessment {
            id: Uuid::new_v4(),
            vendor_name: req.vendor_name,
            vendor_url: req.vendor_url,
            risk_tier: req.risk_tier,
            questionnaire_responses: req.questionnaire_responses,
            score: None,
            reviewed_by: req.reviewed_by,
            last_assessed: Utc::now(),
            next_review: req.next_review,
        };
        self.vendors.lock().unwrap().push(va.clone());
        va
    }

    // ─── Audit Log ───────────────────────────────────────────────────────────

    pub fn append_audit_event(&self, event: AuditEvent) {
        self.audit_log.lock().unwrap().push(event);
    }

    pub fn get_audit_log(&self, limit: usize) -> Vec<AuditEvent> {
        let guard = self.audit_log.lock().unwrap();
        let len = guard.len();
        if limit == 0 || limit >= len {
            guard.clone()
        } else {
            guard[len - limit..].to_vec()
        }
    }

    // ─── Summary ─────────────────────────────────────────────────────────────

    pub fn compliance_summary(&self, framework: Framework) -> ComplianceSummary {
        let controls = self.list_controls(Some(framework));
        let assessments = self.list_assessments();
        ComplianceMonitor::compliance_summary(&controls, &assessments, framework)
    }
}
