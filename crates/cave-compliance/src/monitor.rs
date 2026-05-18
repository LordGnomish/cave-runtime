// SPDX-License-Identifier: AGPL-3.0-or-later
//! Continuous compliance monitoring and gap analysis.

use crate::models::{
    ComplianceSummary, Control, ControlAssessment, ControlStatus, Framework,
};

// ─── Gap ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum GapPriority {
    Critical,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ComplianceGap {
    pub control_id: String,
    pub control_title: String,
    pub framework: Framework,
    pub current_status: ControlStatus,
    pub effectiveness_score: f32,
    pub gaps: Vec<String>,
    pub priority: GapPriority,
}

// ─── Monitor ─────────────────────────────────────────────────────────────────

pub struct ComplianceMonitor;

impl ComplianceMonitor {
    /// Calculate overall effectiveness score for the given framework.
    /// Returns average of all assessed controls; controls with no assessment
    /// count as 0.0.
    pub fn effectiveness_score(assessments: &[ControlAssessment], framework: Framework) -> f32 {
        // Only assessments that can be tied to a framework control are used.
        // Here we receive assessments that have already been filtered by the
        // caller to the requested framework (or we use all of them and trust
        // the caller).  We compute a simple mean of effectiveness_score values.
        if assessments.is_empty() {
            return 0.0;
        }
        let sum: f32 = assessments
            .iter()
            .filter(|a| {
                // Keep only assessments for the right framework by looking up
                // the control id in the catalog.
                crate::frameworks::get_control(&a.control_id)
                    .map(|c| c.framework == framework)
                    .unwrap_or(false)
            })
            .map(|a| a.effectiveness_score)
            .sum();
        let count = assessments
            .iter()
            .filter(|a| {
                crate::frameworks::get_control(&a.control_id)
                    .map(|c| c.framework == framework)
                    .unwrap_or(false)
            })
            .count();
        if count == 0 { 0.0 } else { sum / count as f32 }
    }

    /// Identify gaps: controls that are not implemented or have effectiveness
    /// below the threshold (0.7 by default).
    pub fn identify_gaps(
        controls: &[Control],
        assessments: &[ControlAssessment],
    ) -> Vec<ComplianceGap> {
        const THRESHOLD: f32 = 0.7;
        let mut gaps = Vec::new();

        for control in controls {
            let assessment = assessments.iter().find(|a| a.control_id == control.id);
            let (status, score, gap_list) = match assessment {
                Some(a) => (a.status, a.effectiveness_score, a.gaps.clone()),
                None => (ControlStatus::NotImplemented, 0.0, vec!["No assessment found".into()]),
            };

            let needs_attention = matches!(
                status,
                ControlStatus::NotImplemented | ControlStatus::Planned
            ) || score < THRESHOLD;

            if needs_attention {
                let priority = match status {
                    ControlStatus::NotImplemented => GapPriority::Critical,
                    ControlStatus::Planned if score == 0.0 => GapPriority::High,
                    _ if score < 0.4 => GapPriority::High,
                    _ if score < 0.6 => GapPriority::Medium,
                    _ => GapPriority::Low,
                };
                gaps.push(ComplianceGap {
                    control_id: control.id.clone(),
                    control_title: control.title.clone(),
                    framework: control.framework,
                    current_status: status,
                    effectiveness_score: score,
                    gaps: gap_list,
                    priority,
                });
            }
        }
        gaps
    }

    /// Generate a high-level compliance summary for a given framework.
    pub fn compliance_summary(
        controls: &[Control],
        assessments: &[ControlAssessment],
        framework: Framework,
    ) -> ComplianceSummary {
        let framework_controls: Vec<&Control> =
            controls.iter().filter(|c| c.framework == framework).collect();
        let total = framework_controls.len();
        let mut implemented = 0usize;
        let mut tested = 0usize;
        let mut gap_count = 0usize;

        for control in &framework_controls {
            match assessments.iter().find(|a| a.control_id == control.id) {
                Some(a) => {
                    match a.status {
                        ControlStatus::Implemented | ControlStatus::Tested | ControlStatus::Audited => {
                            implemented += 1;
                        }
                        _ => {
                            gap_count += 1;
                        }
                    }
                    if matches!(a.status, ControlStatus::Tested | ControlStatus::Audited) {
                        tested += 1;
                    }
                }
                None => {
                    gap_count += 1;
                }
            }
        }

        let effectiveness =
            Self::effectiveness_score(assessments, framework);

        ComplianceSummary {
            framework,
            total_controls: total,
            implemented,
            tested,
            gaps: gap_count,
            effectiveness_score: effectiveness,
        }
    }
}
