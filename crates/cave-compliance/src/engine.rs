//! Compliance assessment engine.
//!
//! All functions are pure (take immutable or mutable store references) so they
//! are easy to unit-test and compose without async overhead.

use crate::mapping;
use crate::models::{
    Assessment, AssessmentStatus, ComplianceReport, Control, ControlGap, ControlResult, Evidence,
    EvidenceType, Framework, GapsResponse,
};
use crate::ComplianceStore;
use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// assess_control
// ────────────────────────────────────────────────────────────────────────────

/// Assess a single control against the current store state.
///
/// Checks:
/// 1. Whether any module mappings are defined for the control.
/// 2. Whether evidence has been collected from each expected module.
///
/// Returns a new `Assessment` (not yet pushed to the store — caller decides).
pub fn assess_control(store: &ComplianceStore, control: &Control) -> Assessment {
    let mappings = mapping::get_mappings_for_control(control);

    let evidence_for_control: Vec<&Evidence> = store
        .evidences
        .iter()
        .filter(|e| e.control_id == control.id)
        .collect();

    let evidence_ids: Vec<Uuid> = evidence_for_control.iter().map(|e| e.id).collect();

    let mut findings: Vec<String> = Vec::new();

    let status = if mappings.is_empty() {
        findings.push("No module mappings defined for this control — manual evidence required.".into());
        AssessmentStatus::NonCompliant
    } else if evidence_for_control.is_empty() {
        findings.push("No evidence collected for this control.".into());
        for m in &mappings {
            findings.push(format!("Expected evidence from {}", m.cave_module));
        }
        AssessmentStatus::NonCompliant
    } else {
        let covered_modules: Vec<&str> = evidence_for_control
            .iter()
            .map(|e| e.source_module.as_str())
            .collect();

        let missing: Vec<&str> = mappings
            .iter()
            .map(|m| m.cave_module.as_str())
            .filter(|m| !covered_modules.contains(m))
            .collect();

        if missing.is_empty() {
            AssessmentStatus::Compliant
        } else {
            for m in &missing {
                findings.push(format!("Missing evidence from {m}"));
            }
            AssessmentStatus::Partial
        }
    };

    let score = match &status {
        AssessmentStatus::Compliant => Some(1.0_f32),
        AssessmentStatus::Partial => {
            let total = mapping::get_mappings_for_control(control).len() as f32;
            let covered = (mapping::get_mappings_for_control(control).len()
                - findings.len().min(mapping::get_mappings_for_control(control).len()))
                as f32;
            Some(if total > 0.0 { covered / total } else { 0.0 })
        }
        AssessmentStatus::NonCompliant => Some(0.0_f32),
        AssessmentStatus::NotApplicable => None,
    };

    Assessment {
        id: Uuid::new_v4(),
        control_id: control.id,
        status,
        score,
        findings,
        evidence_ids,
        assessed_at: Utc::now(),
        assessed_by: None,
        next_review_at: None,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// collect_evidence
// ────────────────────────────────────────────────────────────────────────────

/// Auto-gather evidence for a control from the CAVE module graph.
///
/// Currently produces synthetic config-type evidence for each mapped module.
/// Future: call into cave-auth, cave-vault, cave-scan etc. via their APIs.
pub fn collect_evidence(control: &Control) -> Vec<Evidence> {
    let mappings = mapping::get_mappings_for_control(control);
    let now = Utc::now();

    mappings
        .iter()
        .map(|m| Evidence {
            id: Uuid::new_v4(),
            control_id: control.id,
            evidence_type: EvidenceType::Config,
            title: format!("{} — module active", m.cave_module),
            content: format!(
                "Auto-collected: {} is active and satisfies \"{}\" (control {}).",
                m.cave_module, m.description, control.identifier
            ),
            source_module: m.cave_module.clone(),
            collected_at: now,
            collected_by: None,
            expires_at: None,
        })
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// generate_report
// ────────────────────────────────────────────────────────────────────────────

/// Build a full `ComplianceReport` for the given framework.
pub fn generate_report(
    store: &ComplianceStore,
    framework: &Framework,
    period_start: chrono::DateTime<Utc>,
    period_end: chrono::DateTime<Utc>,
) -> ComplianceReport {
    let controls: Vec<&Control> = store
        .controls
        .iter()
        .filter(|c| &c.framework == framework)
        .collect();

    let total = controls.len();
    let mut compliant_count = 0usize;
    let mut non_compliant_count = 0usize;
    let mut partial_count = 0usize;
    let mut not_applicable_count = 0usize;
    let mut control_results: Vec<ControlResult> = Vec::new();

    for control in &controls {
        let latest_assessment = store
            .assessments
            .iter()
            .filter(|a| a.control_id == control.id)
            .max_by_key(|a| a.assessed_at);

        let status = latest_assessment
            .map(|a| a.status.clone())
            .unwrap_or(AssessmentStatus::NonCompliant);

        match &status {
            AssessmentStatus::Compliant => compliant_count += 1,
            AssessmentStatus::NonCompliant => non_compliant_count += 1,
            AssessmentStatus::Partial => partial_count += 1,
            AssessmentStatus::NotApplicable => not_applicable_count += 1,
        }

        let evidence_count = store
            .evidences
            .iter()
            .filter(|e| e.control_id == control.id)
            .count();

        let gaps: Vec<String> = if evidence_count == 0 {
            vec!["No evidence collected".into()]
        } else {
            vec![]
        };

        control_results.push(ControlResult {
            control: (*control).clone(),
            status,
            evidence_count,
            gaps,
        });
    }

    let overall_score = if total > 0 {
        (compliant_count as f32 + partial_count as f32 * 0.5) / total as f32
    } else {
        0.0
    };

    ComplianceReport {
        id: Uuid::new_v4(),
        framework: framework.clone(),
        title: format!("{framework} Compliance Report"),
        generated_at: Utc::now(),
        period_start,
        period_end,
        total_controls: total,
        compliant: compliant_count,
        non_compliant: non_compliant_count,
        partial: partial_count,
        not_applicable: not_applicable_count,
        overall_score,
        control_results,
        summary: format!(
            "{compliant_count}/{total} controls compliant ({:.0}% overall score)",
            overall_score * 100.0
        ),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// detect_gaps
// ────────────────────────────────────────────────────────────────────────────

/// Find controls that have never been assessed or whose latest assessment is
/// non-compliant/partial.
pub fn detect_gaps(store: &ComplianceStore) -> GapsResponse {
    let gaps: Vec<ControlGap> = store
        .controls
        .iter()
        .filter_map(|control| {
            let latest_assessment = store
                .assessments
                .iter()
                .filter(|a| a.control_id == control.id)
                .max_by_key(|a| a.assessed_at);

            let evidence_count = store
                .evidences
                .iter()
                .filter(|e| e.control_id == control.id)
                .count();

            let is_gap = match &latest_assessment {
                None => true,
                Some(a) => matches!(
                    a.status,
                    AssessmentStatus::NonCompliant | AssessmentStatus::Partial
                ),
            };

            if is_gap {
                let gap_reason = match &latest_assessment {
                    None => "Never assessed".into(),
                    Some(a) => format!("Assessment status: {:?}", a.status),
                };

                Some(ControlGap {
                    control: control.clone(),
                    gap_reason,
                    evidence_count,
                    last_assessed: latest_assessment.map(|a| a.assessed_at),
                })
            } else {
                None
            }
        })
        .collect();

    let total = gaps.len();
    GapsResponse { gaps, total }
}

// ────────────────────────────────────────────────────────────────────────────
// continuous_monitoring
// ────────────────────────────────────────────────────────────────────────────

/// Run a full automated sweep: collect evidence for every control then
/// re-assess. Skips evidence that already exists for a (control, module) pair.
///
/// Intended to be called on a schedule (e.g. via a tokio interval task).
pub fn continuous_monitoring(store: &mut ComplianceStore) {
    let controls: Vec<Control> = store.controls.clone();

    info!(
        controls = controls.len(),
        "Starting continuous compliance monitoring sweep"
    );

    for control in &controls {
        // Collect new evidence — skip modules already represented
        let new_evidence = collect_evidence(control);
        for ev in new_evidence {
            let already_present = store.evidences.iter().any(|e| {
                e.control_id == ev.control_id && e.source_module == ev.source_module
            });
            if !already_present {
                store.evidences.push(ev);
            }
        }

        // Re-assess (immutable borrow of store ends before the push below)
        let assessment = assess_control(store, control);
        store.assessments.push(assessment);
    }

    warn!(
        controls = controls.len(),
        "Continuous compliance monitoring sweep complete"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::seed_controls;

    fn make_store() -> ComplianceStore {
        ComplianceStore {
            controls: seed_controls(),
            evidences: vec![],
            assessments: vec![],
            audit_trail: vec![],
            risks: vec![],
            policy_documents: vec![],
            remediations: vec![],
        }
    }

    #[test]
    fn test_assess_no_evidence_is_non_compliant() {
        let store = make_store();
        let control = store
            .controls
            .iter()
            .find(|c| c.identifier == "CC6.1")
            .unwrap()
            .clone();
        let assessment = assess_control(&store, &control);
        assert_eq!(assessment.status, AssessmentStatus::NonCompliant);
    }

    #[test]
    fn test_collect_evidence_matches_mappings() {
        let store = make_store();
        let control = store
            .controls
            .iter()
            .find(|c| c.identifier == "CC6.1")
            .unwrap()
            .clone();
        let evidence = collect_evidence(&control);
        assert_eq!(evidence.len(), 2); // cave-auth + cave-pam
        assert!(evidence.iter().any(|e| e.source_module == "cave-auth"));
        assert!(evidence.iter().any(|e| e.source_module == "cave-pam"));
    }

    #[test]
    fn test_assess_compliant_when_all_modules_covered() {
        let mut store = make_store();
        let control = store
            .controls
            .iter()
            .find(|c| c.identifier == "CC6.1")
            .unwrap()
            .clone();

        // Inject evidence for both expected modules
        for ev in collect_evidence(&control) {
            store.evidences.push(ev);
        }

        let assessment = assess_control(&store, &control);
        assert_eq!(assessment.status, AssessmentStatus::Compliant);
    }

    #[test]
    fn test_detect_gaps_includes_unassessed_controls() {
        let store = make_store();
        let gaps = detect_gaps(&store);
        assert_eq!(gaps.total, store.controls.len());
    }

    #[test]
    fn test_continuous_monitoring_creates_assessments() {
        let mut store = make_store();
        continuous_monitoring(&mut store);
        // Every control should have at least one assessment now
        assert!(!store.assessments.is_empty());
        assert_eq!(store.assessments.len(), store.controls.len());
    }
}
