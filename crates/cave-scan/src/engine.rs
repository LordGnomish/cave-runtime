use crate::models::{Finding, FindingSeverity, RuleType, ScanResult, ScanRule};
use chrono::Utc;
use uuid::Uuid;

/// Return a numeric rank for a severity level (higher = more severe)
pub fn severity_rank(s: &FindingSeverity) -> u8 {
    match s {
        FindingSeverity::Info => 0,
        FindingSeverity::Minor => 1,
        FindingSeverity::Major => 2,
        FindingSeverity::Critical => 3,
    }
}

/// Match a keyword rule against content lines, returning findings
pub fn match_keyword(rule: &ScanRule, content: &str, file_path: &str) -> Vec<Finding> {
    if rule.rule_type != RuleType::Keyword || !rule.enabled {
        return vec![];
    }
    let pattern = rule.pattern.to_lowercase();
    content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            if line.to_lowercase().contains(&pattern) {
                Some(Finding {
                    id: Uuid::new_v4(),
                    rule_id: rule.id,
                    rule_name: rule.name.clone(),
                    file_path: file_path.to_string(),
                    line_number: i + 1,
                    matched_text: line.to_string(),
                    severity: rule.severity.clone(),
                    message: format!("Pattern '{}' found", rule.pattern),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Apply all enabled rules to content
pub fn scan_content(rules: &[ScanRule], content: &str, file_path: &str) -> Vec<Finding> {
    rules.iter()
        .filter(|r| r.enabled)
        .flat_map(|rule| match rule.rule_type {
            RuleType::Keyword => match_keyword(rule, content, file_path),
            _ => vec![], // Other types not yet implemented
        })
        .collect()
}

/// Build a ScanResult from a list of file findings
pub fn build_result(target: &str, findings: Vec<Finding>, rules_count: usize, files_count: usize) -> ScanResult {
    ScanResult {
        scan_id: Uuid::new_v4(),
        target: target.to_string(),
        findings,
        scanned_at: Utc::now(),
        rules_applied: rules_count,
        files_scanned: files_count,
    }
}

/// Filter findings at or above a minimum severity level
pub fn filter_by_min_severity<'a>(findings: &'a [Finding], min_severity: &FindingSeverity) -> Vec<&'a Finding> {
    findings.iter()
        .filter(|f| severity_rank(&f.severity) >= severity_rank(min_severity))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_keyword_rule(pattern: &str, severity: FindingSeverity, enabled: bool) -> ScanRule {
        ScanRule {
            id: Uuid::new_v4(),
            name: format!("rule-{}", pattern),
            description: "Test keyword rule".to_string(),
            pattern: pattern.to_string(),
            rule_type: RuleType::Keyword,
            severity,
            enabled,
        }
    }

    fn make_semgrep_rule(pattern: &str) -> ScanRule {
        ScanRule {
            id: Uuid::new_v4(),
            name: "semgrep-rule".to_string(),
            description: "Test semgrep rule".to_string(),
            pattern: pattern.to_string(),
            rule_type: RuleType::Semgrep,
            severity: FindingSeverity::Major,
            enabled: true,
        }
    }

    fn make_finding(severity: FindingSeverity) -> Finding {
        Finding {
            id: Uuid::new_v4(),
            rule_id: Uuid::new_v4(),
            rule_name: "test-rule".to_string(),
            file_path: "src/lib.rs".to_string(),
            line_number: 1,
            matched_text: "some line".to_string(),
            severity,
            message: "Test finding".to_string(),
        }
    }

    #[test]
    fn test_keyword_match_found() {
        let rule = make_keyword_rule("password", FindingSeverity::Critical, true);
        let content = "let password = \"hunter2\";";
        let findings = match_keyword(&rule, content, "src/auth.rs");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line_number, 1);
        assert_eq!(findings[0].file_path, "src/auth.rs");
    }

    #[test]
    fn test_keyword_match_not_found() {
        let rule = make_keyword_rule("password", FindingSeverity::Critical, true);
        let content = "let username = \"admin\";";
        let findings = match_keyword(&rule, content, "src/auth.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_keyword_match_case_insensitive() {
        let rule = make_keyword_rule("PASSWORD", FindingSeverity::Critical, true);
        let content = "let password = \"secret\";";
        let findings = match_keyword(&rule, content, "src/config.rs");
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_keyword_disabled_rule_skipped() {
        let rule = make_keyword_rule("password", FindingSeverity::Critical, false);
        let content = "let password = \"secret\";";
        let findings = match_keyword(&rule, content, "src/auth.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_keyword_match_multiple_lines() {
        let rule = make_keyword_rule("secret", FindingSeverity::Major, true);
        let content = "let secret_key = \"abc\";\nprintln!(\"no match\");\nlet api_secret = \"xyz\";";
        let findings = match_keyword(&rule, content, "src/main.rs");
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].line_number, 1);
        assert_eq!(findings[1].line_number, 3);
    }

    #[test]
    fn test_scan_content_empty_rules() {
        let content = "let password = \"secret\";";
        let findings = scan_content(&[], content, "src/main.rs");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_scan_content_skips_non_keyword() {
        let rule = make_semgrep_rule("some.pattern");
        let content = "let password = \"secret\";\nsome.pattern here";
        let findings = scan_content(&[rule], content, "src/main.rs");
        // Semgrep rules are not implemented, should return empty
        assert!(findings.is_empty());
    }

    #[test]
    fn test_severity_rank_ordering() {
        assert!(severity_rank(&FindingSeverity::Critical) > severity_rank(&FindingSeverity::Major));
        assert!(severity_rank(&FindingSeverity::Major) > severity_rank(&FindingSeverity::Minor));
        assert!(severity_rank(&FindingSeverity::Minor) > severity_rank(&FindingSeverity::Info));
        assert_eq!(severity_rank(&FindingSeverity::Critical), 3);
        assert_eq!(severity_rank(&FindingSeverity::Major), 2);
        assert_eq!(severity_rank(&FindingSeverity::Minor), 1);
        assert_eq!(severity_rank(&FindingSeverity::Info), 0);
    }

    #[test]
    fn test_filter_by_min_severity() {
        let findings = vec![
            make_finding(FindingSeverity::Critical),
            make_finding(FindingSeverity::Major),
            make_finding(FindingSeverity::Minor),
            make_finding(FindingSeverity::Info),
        ];
        // Filter keeping Major and above
        let filtered = filter_by_min_severity(&findings, &FindingSeverity::Major);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|f| {
            f.severity == FindingSeverity::Critical || f.severity == FindingSeverity::Major
        }));
    }

    #[test]
    fn test_build_result() {
        let findings = vec![
            make_finding(FindingSeverity::Critical),
            make_finding(FindingSeverity::Major),
        ];
        let result = build_result("my-project", findings, 5, 10);
        assert_eq!(result.target, "my-project");
        assert_eq!(result.rules_applied, 5);
        assert_eq!(result.files_scanned, 10);
        assert_eq!(result.findings.len(), 2);
    }
}
