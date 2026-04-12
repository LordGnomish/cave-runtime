//! CAVE Security — Runtime security and vulnerability scanning.
//!
//! Replaces: Falco (runtime threat detection) + Trivy (vulnerability scanning)
//!
//! Features:
//! - Rule engine with Falco-compatible condition/priority model
//! - eBPF syscall monitoring integration points
//! - Container image vulnerability scanning against CVE database
//! - SBOM generation in SPDX / CycloneDX format
//! - Scan policies: fail on severity, CVE allowlist, signature requirement
//! - Image signing verification hooks (cosign/notation compatible)
//! - REST API: /api/v1/rules, /api/v1/alerts, /api/v1/scans,
//!             /api/v1/vulnerabilities, /api/v1/sbom

pub mod models;
pub mod routes;
pub mod rules;
pub mod scanner;

use axum::Router;
use models::{ScanPolicy, ScanResult, SecurityAlert, SecurityRule};
use rules::builtin_rules;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Module-wide shared state.
pub struct SecurityState {
    pub rules: RwLock<Vec<SecurityRule>>,
    pub alerts: RwLock<Vec<SecurityAlert>>,
    pub scans: RwLock<Vec<ScanResult>>,
    pub policy: RwLock<ScanPolicy>,
}

impl Default for SecurityState {
    fn default() -> Self {
        Self {
            rules: RwLock::new(builtin_rules()),
            alerts: RwLock::new(Vec::new()),
            scans: RwLock::new(Vec::new()),
            policy: RwLock::new(ScanPolicy::default()),
        }
    }
}

pub fn router(state: Arc<SecurityState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "security";

// ── eBPF integration points ───────────────────────────────────────────────────
//
// These stubs represent hooks that an Aya-based eBPF program would call.
// In production each function would receive events from a perf-event ring
// buffer and dispatch them through `rules::evaluate_rules`.

/// Hook called by eBPF when a process exec syscall is captured.
pub fn on_process_exec(_pid: u32, _comm: &str) {}

/// Hook called by eBPF when a file open syscall is captured.
pub fn on_file_open(_pid: u32, _path: &str) {}

/// Hook called by eBPF when a network connect syscall is captured.
pub fn on_network_connect(_pid: u32, _dst_port: u16) {}

#[cfg(test)]
mod tests {
    use crate::models::*;
    use crate::rules::*;
    use crate::scanner::*;
    use std::collections::HashMap;

    fn make_event(event_type: EventType) -> SecurityEvent {
        SecurityEvent {
            id: uuid::Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            event_type,
            process_name: None,
            file_path: None,
            network_port: None,
            is_root: false,
            syscall: None,
            container_image: None,
            metadata: HashMap::new(),
        }
    }

    // 1
    #[test]
    fn test_priority_ordering() {
        assert!(Priority::Emergency > Priority::Alert);
        assert!(Priority::Alert > Priority::Critical);
        assert!(Priority::Critical > Priority::Error);
        assert!(Priority::Error > Priority::Warning);
        assert!(Priority::Debug < Priority::Warning);
    }

    // 2
    #[test]
    fn test_condition_process_name_exact_match() {
        let cond = Condition::ProcessName { value: "bash".to_string(), exact: true };
        let mut event = make_event(EventType::ProcessExec);
        event.process_name = Some("bash".to_string());
        assert!(evaluate_condition(&cond, &event));
    }

    // 3
    #[test]
    fn test_condition_process_name_exact_no_match() {
        let cond = Condition::ProcessName { value: "bash".to_string(), exact: true };
        let mut event = make_event(EventType::ProcessExec);
        event.process_name = Some("bash_wrapper".to_string());
        assert!(!evaluate_condition(&cond, &event));
    }

    // 4
    #[test]
    fn test_condition_process_name_contains() {
        let cond = Condition::ProcessName { value: "sh".to_string(), exact: false };
        let mut event = make_event(EventType::ProcessExec);
        event.process_name = Some("bash".to_string());
        assert!(evaluate_condition(&cond, &event));
    }

    // 5
    #[test]
    fn test_condition_file_path() {
        let cond = Condition::FilePath { prefix: "/etc/shadow".to_string() };
        let mut event = make_event(EventType::FileAccess);
        event.file_path = Some("/etc/shadow".to_string());
        assert!(evaluate_condition(&cond, &event));
        event.file_path = Some("/tmp/safe".to_string());
        assert!(!evaluate_condition(&cond, &event));
    }

    // 6
    #[test]
    fn test_condition_network_port() {
        let cond = Condition::NetworkPort { port: 4444 };
        let mut event = make_event(EventType::NetworkConnect);
        event.network_port = Some(4444);
        assert!(evaluate_condition(&cond, &event));
        event.network_port = Some(80);
        assert!(!evaluate_condition(&cond, &event));
    }

    // 7
    #[test]
    fn test_condition_is_root() {
        let cond = Condition::IsRoot;
        let mut event = make_event(EventType::ProcessExec);
        event.is_root = true;
        assert!(evaluate_condition(&cond, &event));
        event.is_root = false;
        assert!(!evaluate_condition(&cond, &event));
    }

    // 8
    #[test]
    fn test_condition_syscall() {
        let cond = Condition::Syscall { name: "ptrace".to_string() };
        let mut event = make_event(EventType::SyscallDetected);
        event.syscall = Some("ptrace".to_string());
        assert!(evaluate_condition(&cond, &event));
        event.syscall = Some("read".to_string());
        assert!(!evaluate_condition(&cond, &event));
    }

    // 9
    #[test]
    fn test_condition_and() {
        let cond = Condition::And {
            conditions: vec![
                Condition::IsRoot,
                Condition::NetworkPort { port: 22 },
            ],
        };
        let mut event = make_event(EventType::NetworkConnect);
        event.is_root = true;
        event.network_port = Some(22);
        assert!(evaluate_condition(&cond, &event));
        event.is_root = false;
        assert!(!evaluate_condition(&cond, &event));
    }

    // 10
    #[test]
    fn test_condition_or() {
        let cond = Condition::Or {
            conditions: vec![
                Condition::NetworkPort { port: 4444 },
                Condition::NetworkPort { port: 1337 },
            ],
        };
        let mut event = make_event(EventType::NetworkConnect);
        event.network_port = Some(1337);
        assert!(evaluate_condition(&cond, &event));
        event.network_port = Some(80);
        assert!(!evaluate_condition(&cond, &event));
    }

    // 11
    #[test]
    fn test_condition_not() {
        let cond = Condition::Not { condition: Box::new(Condition::IsRoot) };
        let mut event = make_event(EventType::ProcessExec);
        event.is_root = false;
        assert!(evaluate_condition(&cond, &event));
        event.is_root = true;
        assert!(!evaluate_condition(&cond, &event));
    }

    // 12
    #[test]
    fn test_evaluate_rules_generates_alert_on_match() {
        let rules = builtin_rules();
        let mut event = make_event(EventType::FileAccess);
        event.file_path = Some("/etc/shadow".to_string());
        let alerts = evaluate_rules(&rules, &event);
        assert!(!alerts.is_empty());
        assert!(alerts.iter().any(|a| a.rule_name == "sensitive_file_access"));
    }

    // 13
    #[test]
    fn test_evaluate_rules_no_match_produces_no_alerts() {
        let rules = builtin_rules();
        // Safe event: ordinary file path, not root, no suspicious port
        let mut event = make_event(EventType::FileAccess);
        event.file_path = Some("/tmp/safe.txt".to_string());
        let alerts = evaluate_rules(&rules, &event);
        // None of the builtin rules should match this
        assert!(alerts.is_empty());
    }

    // 14
    #[test]
    fn test_scanner_finds_critical_vulnerability() {
        let db = sample_cve_db();
        let packages = vec![InstalledPackage {
            name: "openssl".to_string(),
            version: "1.1.1".to_string(),
            layer_digest: None,
        }];
        let vulns = find_vulnerabilities(&packages, &db);
        assert_eq!(vulns.len(), 1);
        assert_eq!(vulns[0].cve_id, "CVE-2023-1234");
        assert_eq!(vulns[0].severity, CvssSeverity::Critical);
    }

    // 15
    #[test]
    fn test_scanner_unaffected_version_has_no_vulns() {
        let db = sample_cve_db();
        let packages = vec![InstalledPackage {
            name: "openssl".to_string(),
            version: "3.0.0".to_string(),
            layer_digest: None,
        }];
        assert!(find_vulnerabilities(&packages, &db).is_empty());
    }

    // 16
    #[test]
    fn test_scan_policy_fails_on_critical() {
        let db = sample_cve_db();
        let packages = vec![InstalledPackage {
            name: "openssl".to_string(),
            version: "1.1.1".to_string(),
            layer_digest: None,
        }];
        let policy = ScanPolicy { fail_on_severity: CvssSeverity::High, ..ScanPolicy::default() };
        let result = scan_image("example/image:latest", "sha256:abc", &db, &packages, &policy);
        assert!(matches!(result.policy_result, PolicyResult::Fail { .. }));
    }

    // 17
    #[test]
    fn test_scan_policy_allowlist_bypasses_cve() {
        let db = sample_cve_db();
        let packages = vec![InstalledPackage {
            name: "openssl".to_string(),
            version: "1.1.1".to_string(),
            layer_digest: None,
        }];
        let policy = ScanPolicy {
            allowed_cves: vec!["CVE-2023-1234".to_string()],
            fail_on_severity: CvssSeverity::Critical,
            ..ScanPolicy::default()
        };
        let result = scan_image("example/image:latest", "sha256:abc", &db, &packages, &policy);
        assert!(matches!(result.policy_result, PolicyResult::Pass));
    }

    // 18
    #[test]
    fn test_sbom_generation_spdx() {
        let packages = vec![
            InstalledPackage {
                name: "curl".to_string(),
                version: "7.84.0".to_string(),
                layer_digest: None,
            },
            InstalledPackage {
                name: "openssl".to_string(),
                version: "1.1.1".to_string(),
                layer_digest: None,
            },
        ];
        let sbom = generate_sbom("example/image:latest", &packages, SbomFormat::Spdx);
        assert_eq!(sbom.components.len(), 2);
        assert!(sbom.components.iter().any(|c| c.name == "curl"));
        assert!(sbom.components[0].purl.starts_with("pkg:generic/"));
    }

    // 19 (bonus)
    #[test]
    fn test_cvss_severity_ordering() {
        assert!(CvssSeverity::Critical > CvssSeverity::High);
        assert!(CvssSeverity::High > CvssSeverity::Medium);
        assert!(CvssSeverity::Medium > CvssSeverity::Low);
        assert!(CvssSeverity::Low > CvssSeverity::None);
    }

    // 20 (bonus)
    #[test]
    fn test_builtin_rules_are_all_enabled() {
        let rules = builtin_rules();
        assert!(rules.len() >= 5);
        assert!(rules.iter().all(|r| r.enabled));
    }
}
