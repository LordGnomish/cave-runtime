// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{Evidence, EvidenceType};
use uuid::Uuid;

pub fn create_manual_evidence(control_id: Uuid, finding_id: Option<Uuid>, description: &str, data: serde_json::Value, collected_by: &str) -> Evidence {
    Evidence {
        id: Uuid::new_v4(),
        finding_id,
        control_id,
        evidence_type: EvidenceType::Manual,
        description: description.to_string(),
        data,
        collected_at: chrono::Utc::now(),
        collected_by: collected_by.to_string(),
    }
}

pub fn create_snapshot_evidence(control_id: Uuid, finding_id: Option<Uuid>, snapshot: serde_json::Value) -> Evidence {
    Evidence {
        id: Uuid::new_v4(),
        finding_id,
        control_id,
        evidence_type: EvidenceType::ConfigSnapshot,
        description: "Configuration snapshot collected automatically".to_string(),
        data: snapshot,
        collected_at: chrono::Utc::now(),
        collected_by: "cave-compliance/auto".to_string(),
    }
}

/// Check if evidence is still valid (not stale).
pub fn is_fresh(evidence: &Evidence, max_age_hours: i64) -> bool {
    let age = chrono::Utc::now() - evidence.collected_at;
    age.num_hours() < max_age_hours
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_create_manual_evidence() {
        let id = Uuid::new_v4();
        let ev = create_manual_evidence(id, None, "Test evidence", serde_json::json!({"ok": true}), "admin");
        assert_eq!(ev.control_id, id);
        assert!(matches!(ev.evidence_type, EvidenceType::Manual));
    }
    #[test]
    fn test_is_fresh() {
        let id = Uuid::new_v4();
        let ev = create_snapshot_evidence(id, None, serde_json::json!({}));
        assert!(is_fresh(&ev, 24));
    }
}
