// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{CaseStatus, EvidenceItem, ForensicCase, ForensicSeverity};

pub fn severity_rank(s: &ForensicSeverity) -> u8 {
    match s {
        ForensicSeverity::Low => 0,
        ForensicSeverity::Medium => 1,
        ForensicSeverity::High => 2,
        ForensicSeverity::Critical => 3,
    }
}

pub fn open_cases(cases: &[ForensicCase]) -> Vec<&ForensicCase> {
    cases
        .iter()
        .filter(|c| c.status == CaseStatus::Open || c.status == CaseStatus::InProgress)
        .collect()
}

pub fn highest_severity<'a>(cases: &'a [ForensicCase]) -> Option<&'a ForensicCase> {
    cases.iter().max_by_key(|c| severity_rank(&c.severity))
}

pub fn evidence_count(case: &ForensicCase) -> usize {
    case.evidence.len()
}

pub fn has_valid_evidence(item: &EvidenceItem) -> bool {
    item.hash_sha256.as_ref().map_or(false, |h| {
        h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EvidenceType, ForensicCase, ForensicSeverity, CaseStatus, EvidenceItem};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_case(severity: ForensicSeverity, status: CaseStatus) -> ForensicCase {
        ForensicCase {
            id: Uuid::new_v4(),
            title: "Test Case".to_string(),
            description: "desc".to_string(),
            severity,
            status,
            created_at: Utc::now(),
            evidence: vec![],
        }
    }

    fn make_evidence(hash: Option<&str>) -> EvidenceItem {
        EvidenceItem {
            id: Uuid::new_v4(),
            evidence_type: EvidenceType::LogFile,
            description: "test evidence".to_string(),
            hash_sha256: hash.map(|s| s.to_string()),
            collected_at: Utc::now(),
            chain_of_custody: vec![],
        }
    }

    #[test]
    fn test_severity_rank_ordering() {
        assert!(severity_rank(&ForensicSeverity::Critical) > severity_rank(&ForensicSeverity::High));
        assert!(severity_rank(&ForensicSeverity::High) > severity_rank(&ForensicSeverity::Medium));
        assert!(severity_rank(&ForensicSeverity::Medium) > severity_rank(&ForensicSeverity::Low));
    }

    #[test]
    fn test_open_cases_filter() {
        let cases = vec![
            make_case(ForensicSeverity::High, CaseStatus::Open),
            make_case(ForensicSeverity::Medium, CaseStatus::InProgress),
            make_case(ForensicSeverity::Low, CaseStatus::Closed),
            make_case(ForensicSeverity::Critical, CaseStatus::Archived),
        ];
        let open = open_cases(&cases);
        assert_eq!(open.len(), 2);
    }

    #[test]
    fn test_highest_severity() {
        let cases = vec![
            make_case(ForensicSeverity::Low, CaseStatus::Open),
            make_case(ForensicSeverity::Critical, CaseStatus::Open),
            make_case(ForensicSeverity::High, CaseStatus::Open),
        ];
        let highest = highest_severity(&cases).unwrap();
        assert_eq!(highest.severity, ForensicSeverity::Critical);
    }

    #[test]
    fn test_evidence_count() {
        let mut case = make_case(ForensicSeverity::Low, CaseStatus::Open);
        case.evidence.push(make_evidence(None));
        case.evidence.push(make_evidence(None));
        assert_eq!(evidence_count(&case), 2);
    }

    #[test]
    fn test_has_valid_evidence_valid_hash() {
        let ev = make_evidence(Some(
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
        ));
        assert!(has_valid_evidence(&ev));
    }

    #[test]
    fn test_has_valid_evidence_no_hash() {
        let ev = make_evidence(None);
        assert!(!has_valid_evidence(&ev));
    }

    // ─── extended coverage ───────────────────────────────────────────────

    #[test]
    fn test_severity_rank_values() {
        assert_eq!(severity_rank(&ForensicSeverity::Low), 0);
        assert_eq!(severity_rank(&ForensicSeverity::Medium), 1);
        assert_eq!(severity_rank(&ForensicSeverity::High), 2);
        assert_eq!(severity_rank(&ForensicSeverity::Critical), 3);
    }

    #[test]
    fn test_open_cases_excludes_closed_and_archived() {
        let cases = vec![
            make_case(ForensicSeverity::Low, CaseStatus::Closed),
            make_case(ForensicSeverity::Low, CaseStatus::Archived),
        ];
        assert!(open_cases(&cases).is_empty());
    }

    #[test]
    fn test_open_cases_empty_input() {
        let cases: Vec<ForensicCase> = vec![];
        assert!(open_cases(&cases).is_empty());
    }

    #[test]
    fn test_highest_severity_empty_input_none() {
        let cases: Vec<ForensicCase> = vec![];
        assert!(highest_severity(&cases).is_none());
    }

    #[test]
    fn test_highest_severity_single_case() {
        let cases = vec![make_case(ForensicSeverity::Medium, CaseStatus::Open)];
        let h = highest_severity(&cases).unwrap();
        assert_eq!(h.severity, ForensicSeverity::Medium);
    }

    #[test]
    fn test_evidence_count_zero_for_new_case() {
        let case = make_case(ForensicSeverity::Low, CaseStatus::Open);
        assert_eq!(evidence_count(&case), 0);
    }

    #[test]
    fn test_has_valid_evidence_invalid_hex_chars() {
        let ev = make_evidence(Some(
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
        ));
        assert!(!has_valid_evidence(&ev));
    }

    #[test]
    fn test_has_valid_evidence_too_short() {
        let ev = make_evidence(Some("abcdef")); // 6 chars, not 64
        assert!(!has_valid_evidence(&ev));
    }

    #[test]
    fn test_has_valid_evidence_too_long() {
        let ev = make_evidence(Some(&"a".repeat(65)));
        assert!(!has_valid_evidence(&ev));
    }

    #[test]
    fn test_has_valid_evidence_uppercase_hex_accepted() {
        let ev = make_evidence(Some(
            "ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890",
        ));
        assert!(has_valid_evidence(&ev));
    }
}
