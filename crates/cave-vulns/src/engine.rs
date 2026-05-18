// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{ComponentVersion, Severity, VulnScanResult, Vulnerability, VulnState};
use chrono::Utc;
use uuid::Uuid;

/// Map CVSS score to severity
pub fn cvss_to_severity(score: f32) -> Severity {
    match score {
        s if s >= 9.0 => Severity::Critical,
        s if s >= 7.0 => Severity::High,
        s if s >= 4.0 => Severity::Medium,
        s if s >= 0.1 => Severity::Low,
        _ => Severity::Info,
    }
}

/// Check if a component version is in the affected versions list
pub fn is_affected(vuln: &Vulnerability, component: &str, version: &str) -> bool {
    vuln.affected_component == component && vuln.affected_versions.iter().any(|v| v == version)
}

/// Filter vulnerabilities matching a component
pub fn find_for_component<'a>(vulns: &'a [Vulnerability], component: &ComponentVersion) -> Vec<&'a Vulnerability> {
    vulns.iter()
        .filter(|v| is_affected(v, &component.name, &component.version))
        .collect()
}

/// Count findings by severity
pub fn count_by_severity(findings: &[Vulnerability]) -> (usize, usize, usize, usize) {
    let critical = findings.iter().filter(|v| v.severity == Severity::Critical).count();
    let high = findings.iter().filter(|v| v.severity == Severity::High).count();
    let medium = findings.iter().filter(|v| v.severity == Severity::Medium).count();
    let low = findings.iter().filter(|v| v.severity == Severity::Low).count();
    (critical, high, medium, low)
}

/// Build a scan result from a list of findings
pub fn build_scan_result(target: &str, findings: Vec<Vulnerability>) -> VulnScanResult {
    let (critical, high, medium, low) = count_by_severity(&findings);
    VulnScanResult {
        scan_id: Uuid::new_v4(),
        target: target.to_string(),
        findings,
        scanned_at: Utc::now(),
        total_critical: critical,
        total_high: high,
        total_medium: medium,
        total_low: low,
    }
}

/// Simple version comparison (handles semver like "1.2.3")
/// Returns true if `current` is less than `fixed_in`
pub fn version_lt(current: &str, fixed_in: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.').filter_map(|p| p.parse().ok()).collect()
    };
    let a = parse(current);
    let b = parse(fixed_in);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x < y { return true; }
        if x > y { return false; }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_vuln(component: &str, version: &str, severity: Severity) -> Vulnerability {
        Vulnerability {
            id: Uuid::new_v4(),
            cve_id: "CVE-2024-0001".to_string(),
            title: "Test Vuln".to_string(),
            description: "A test vulnerability".to_string(),
            severity,
            cvss_score: 7.5,
            affected_component: component.to_string(),
            affected_versions: vec![version.to_string()],
            fixed_in: Some("2.0.0".to_string()),
            published_at: Utc::now(),
            state: VulnState::Open,
        }
    }

    #[test]
    fn test_cvss_critical() {
        assert_eq!(cvss_to_severity(9.5), Severity::Critical);
        assert_eq!(cvss_to_severity(9.0), Severity::Critical);
    }

    #[test]
    fn test_cvss_high() {
        assert_eq!(cvss_to_severity(7.5), Severity::High);
        assert_eq!(cvss_to_severity(7.0), Severity::High);
    }

    #[test]
    fn test_cvss_medium() {
        assert_eq!(cvss_to_severity(5.0), Severity::Medium);
        assert_eq!(cvss_to_severity(4.0), Severity::Medium);
    }

    #[test]
    fn test_cvss_low() {
        assert_eq!(cvss_to_severity(2.0), Severity::Low);
        assert_eq!(cvss_to_severity(0.1), Severity::Low);
    }

    #[test]
    fn test_cvss_info() {
        assert_eq!(cvss_to_severity(0.0), Severity::Info);
    }

    #[test]
    fn test_is_affected_true() {
        let vuln = make_vuln("openssl", "1.0.1", Severity::High);
        assert!(is_affected(&vuln, "openssl", "1.0.1"));
    }

    #[test]
    fn test_is_affected_wrong_version() {
        let vuln = make_vuln("openssl", "1.0.1", Severity::High);
        assert!(!is_affected(&vuln, "openssl", "1.0.2"));
    }

    #[test]
    fn test_is_affected_wrong_component() {
        let vuln = make_vuln("openssl", "1.0.1", Severity::High);
        assert!(!is_affected(&vuln, "libcurl", "1.0.1"));
    }

    #[test]
    fn test_count_by_severity() {
        let vulns = vec![
            make_vuln("a", "1.0", Severity::Critical),
            make_vuln("b", "1.0", Severity::Critical),
            make_vuln("c", "1.0", Severity::High),
            make_vuln("d", "1.0", Severity::Medium),
            make_vuln("e", "1.0", Severity::Low),
            make_vuln("f", "1.0", Severity::Low),
            make_vuln("g", "1.0", Severity::Info),
        ];
        let (critical, high, medium, low) = count_by_severity(&vulns);
        assert_eq!(critical, 2);
        assert_eq!(high, 1);
        assert_eq!(medium, 1);
        assert_eq!(low, 2);
    }

    #[test]
    fn test_version_lt() {
        assert!(version_lt("1.0.1", "1.0.2"));
        assert!(version_lt("1.0.0", "2.0.0"));
        assert!(version_lt("0.9.9", "1.0.0"));
        assert!(!version_lt("2.0.0", "1.9.9"));
        assert!(!version_lt("1.0.2", "1.0.1"));
        assert!(!version_lt("1.0.0", "1.0.0")); // equal is not less than
    }

    #[test]
    fn test_build_scan_result_counts() {
        let findings = vec![
            make_vuln("a", "1.0", Severity::Critical),
            make_vuln("b", "1.0", Severity::High),
            make_vuln("c", "1.0", Severity::High),
            make_vuln("d", "1.0", Severity::Medium),
        ];
        let result = build_scan_result("my-target", findings);
        assert_eq!(result.target, "my-target");
        assert_eq!(result.total_critical, 1);
        assert_eq!(result.total_high, 2);
        assert_eq!(result.total_medium, 1);
        assert_eq!(result.total_low, 0);
        assert_eq!(result.findings.len(), 4);
    }
}
