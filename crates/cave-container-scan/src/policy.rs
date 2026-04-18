use crate::engine::aggregate_verdict;
use crate::models::{Finding, ScanVerdict, Severity};

/// Evaluate findings against a policy to produce a ScanVerdict.
/// This is a thin wrapper that applies cave-policy-style rules in pure Rust.
pub fn evaluate_policy(findings: &[Finding], floor: Option<Severity>) -> ScanVerdict {
    aggregate_verdict(findings, floor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FindingCategory, Confidence};
    use uuid::Uuid;

    fn make_finding(severity: Severity) -> Finding {
        let mut f = Finding::new(
            "TEST".to_string(),
            "Test".to_string(),
            FindingCategory::Misconfig,
            severity,
            "Test".to_string(),
            "Test".to_string(),
        );
        f.confidence = Confidence::High;
        f
    }

    #[test]
    fn test_policy_evaluation_critical() {
        let findings = vec![make_finding(Severity::Critical)];
        let verdict = evaluate_policy(&findings, None);
        assert_eq!(verdict.decision.to_string(), "fail");
    }

    #[test]
    fn test_policy_evaluation_pass() {
        let findings = vec![make_finding(Severity::Info)];
        let verdict = evaluate_policy(&findings, None);
        assert_eq!(verdict.decision.to_string(), "pass");
    }
}
