// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for cave-forensics public surface.

use cave_forensics::engine::{evidence_count, has_valid_evidence, highest_severity, open_cases, severity_rank};
use cave_forensics::models::{CaseStatus, EvidenceItem, EvidenceType, ForensicCase, ForensicSeverity};
use chrono::Utc;
use uuid::Uuid;

fn case(severity: ForensicSeverity, status: CaseStatus, evidence: Vec<EvidenceItem>) -> ForensicCase {
    ForensicCase {
        id: Uuid::new_v4(),
        title: "test".to_string(),
        description: "i".to_string(),
        severity,
        status,
        created_at: Utc::now(),
        evidence,
    }
}

fn evidence(t: EvidenceType, hash: Option<&str>) -> EvidenceItem {
    EvidenceItem {
        id: Uuid::new_v4(),
        evidence_type: t,
        description: "test evidence".to_string(),
        hash_sha256: hash.map(|s| s.to_string()),
        collected_at: Utc::now(),
        chain_of_custody: vec![],
    }
}

#[test]
fn integration_severity_rank_via_public_api() {
    assert!(severity_rank(&ForensicSeverity::Critical) > severity_rank(&ForensicSeverity::Low));
}

#[test]
fn integration_open_cases_filter() {
    let cases = vec![
        case(ForensicSeverity::High, CaseStatus::Open, vec![]),
        case(ForensicSeverity::Low, CaseStatus::Closed, vec![]),
    ];
    assert_eq!(open_cases(&cases).len(), 1);
}

#[test]
fn integration_highest_severity_picks_critical() {
    let cases = vec![
        case(ForensicSeverity::Low, CaseStatus::Open, vec![]),
        case(ForensicSeverity::Critical, CaseStatus::Open, vec![]),
        case(ForensicSeverity::Medium, CaseStatus::Open, vec![]),
    ];
    let h = highest_severity(&cases).unwrap();
    assert_eq!(h.severity, ForensicSeverity::Critical);
}

#[test]
fn integration_evidence_count_matches_pushed() {
    let mut c = case(ForensicSeverity::Low, CaseStatus::Open, vec![]);
    c.evidence.push(evidence(EvidenceType::LogFile, None));
    c.evidence.push(evidence(EvidenceType::ProcessDump, None));
    c.evidence.push(evidence(EvidenceType::FileSystem, None));
    assert_eq!(evidence_count(&c), 3);
}

#[test]
fn integration_valid_hash_64_hex_chars() {
    let ev = evidence(
        EvidenceType::MemoryImage,
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
    );
    assert!(has_valid_evidence(&ev));
}

#[test]
fn integration_default_state_constructible() {
    let _ = std::sync::Arc::new(cave_forensics::State::default());
}
