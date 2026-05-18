// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::*;
use chrono::Utc;
use uuid::Uuid;

pub fn builtin_frameworks() -> Vec<ComplianceFramework> {
    vec![cis_kubernetes_framework(), soc2_framework(), pci_dss_framework(), hipaa_framework()]
}

pub fn cis_kubernetes_framework() -> ComplianceFramework {
    let fw_id = Uuid::new_v4();
    let controls = vec![
        make_control(fw_id, "CIS-1.1.1", "Ensure API server config file permissions", "Master Node Security Configuration", ControlSeverity::Critical, true, "check_apiserver_file_perms", "Set permissions to 600 on kube-apiserver.yaml"),
        make_control(fw_id, "CIS-1.2.1", "Ensure anonymous auth is disabled", "API Server", ControlSeverity::High, true, "check_anonymous_auth", "Set --anonymous-auth=false on kube-apiserver"),
        make_control(fw_id, "CIS-1.2.2", "Ensure basic auth is not used", "API Server", ControlSeverity::High, true, "check_basic_auth", "Remove --basic-auth-file from kube-apiserver"),
        make_control(fw_id, "CIS-1.2.6", "Ensure AlwaysAdmit admission plugin is not set", "API Server", ControlSeverity::High, true, "check_always_admit", "Remove AlwaysAdmit from --enable-admission-plugins"),
        make_control(fw_id, "CIS-1.2.9", "Ensure EventRateLimit admission plugin is set", "API Server", ControlSeverity::Medium, true, "check_event_rate_limit", "Add EventRateLimit to --enable-admission-plugins"),
        make_control(fw_id, "CIS-2.1", "Ensure etcd is configured with TLS", "etcd", ControlSeverity::Critical, true, "check_etcd_tls", "Configure --cert-file and --key-file for etcd"),
        make_control(fw_id, "CIS-3.1.1", "Ensure client cert auth not used for users", "Authentication and Authorization", ControlSeverity::High, false, "", "Use OIDC or similar for user authentication"),
        make_control(fw_id, "CIS-4.1.1", "Ensure worker service account permissions", "Worker Node Configuration", ControlSeverity::High, true, "check_worker_sa", "Ensure service accounts are not over-privileged"),
        make_control(fw_id, "CIS-5.1.1", "Ensure RBAC is enabled", "RBAC and Service Accounts", ControlSeverity::Critical, true, "check_rbac_enabled", "Set --authorization-mode=RBAC on kube-apiserver"),
        make_control(fw_id, "CIS-5.2.1", "Minimize privileged containers", "Pod Security Standards", ControlSeverity::High, true, "check_privileged_pods", "Use Pod Security Admission to restrict privileged pods"),
        make_control(fw_id, "CIS-5.2.2", "Minimize hostPID containers", "Pod Security Standards", ControlSeverity::High, true, "check_host_pid", "Restrict hostPID via admission policy"),
        make_control(fw_id, "CIS-5.3.1", "Ensure network policies are in place", "Network Policies and CNI", ControlSeverity::Medium, true, "check_network_policies", "Apply default-deny NetworkPolicy in all namespaces"),
        make_control(fw_id, "CIS-5.4.1", "Prefer Secrets as files over env vars", "Secrets Management", ControlSeverity::Medium, false, "", "Mount secrets as volumes, not environment variables"),
        make_control(fw_id, "CIS-5.7.1", "Create administrative boundaries with namespaces", "General Policies", ControlSeverity::Low, false, "", "Use namespaces to segment workloads by team/environment"),
    ];
    ComplianceFramework {
        id: fw_id,
        name: "CIS Kubernetes Benchmark".to_string(),
        kind: FrameworkKind::CisKubernetes,
        version: "1.8.0".to_string(),
        description: "CIS Kubernetes Benchmark security configuration guide".to_string(),
        controls,
        created_at: Utc::now(),
    }
}

pub fn soc2_framework() -> ComplianceFramework {
    let fw_id = Uuid::new_v4();
    let controls = vec![
        make_control(fw_id, "SOC2-CC6.1", "Logical and Physical Access Controls", "Common Criteria", ControlSeverity::High, true, "check_access_controls", "Implement RBAC and MFA for all privileged access"),
        make_control(fw_id, "SOC2-CC6.2", "Prior to issuing access credentials", "Common Criteria", ControlSeverity::High, false, "", "Document access request and approval process"),
        make_control(fw_id, "SOC2-CC6.6", "Logical access security measures", "Common Criteria", ControlSeverity::High, true, "check_network_restrictions", "Restrict network access to authorized IPs"),
        make_control(fw_id, "SOC2-CC7.1", "Vulnerability scanning", "System Operations", ControlSeverity::Medium, true, "check_vuln_scanning", "Run regular vulnerability scans on all containers"),
        make_control(fw_id, "SOC2-CC8.1", "Change management process", "Change Management", ControlSeverity::Medium, false, "", "Document all infrastructure changes with approval"),
        make_control(fw_id, "SOC2-A1.2", "Availability performance monitoring", "Availability", ControlSeverity::Medium, true, "check_monitoring", "Monitor system availability and set SLOs"),
    ];
    ComplianceFramework {
        id: fw_id,
        name: "SOC 2 Type II".to_string(),
        kind: FrameworkKind::Soc2,
        version: "2017".to_string(),
        description: "SOC 2 Trust Services Criteria for security, availability, and confidentiality".to_string(),
        controls,
        created_at: Utc::now(),
    }
}

pub fn pci_dss_framework() -> ComplianceFramework {
    let fw_id = Uuid::new_v4();
    let controls = vec![
        make_control(fw_id, "PCI-1.1", "Network security controls", "Build and Maintain a Secure Network", ControlSeverity::Critical, true, "check_network_segmentation", "Implement network segmentation for cardholder data"),
        make_control(fw_id, "PCI-2.1", "Change default passwords", "Secure Configurations", ControlSeverity::Critical, true, "check_default_credentials", "Change all vendor-supplied default credentials"),
        make_control(fw_id, "PCI-6.3", "Security vulnerabilities identified", "Develop and Maintain Secure Systems", ControlSeverity::High, true, "check_vuln_management", "Maintain a vulnerability management program"),
        make_control(fw_id, "PCI-7.1", "Limit access to cardholder data", "Restrict Access", ControlSeverity::Critical, false, "", "Implement need-to-know access policy for cardholder data"),
        make_control(fw_id, "PCI-8.2", "User identification and authentication", "Identify and Authenticate", ControlSeverity::High, true, "check_user_auth", "Assign unique IDs to all users"),
        make_control(fw_id, "PCI-10.1", "Audit logs for all system components", "Track and Monitor Access", ControlSeverity::High, true, "check_audit_logging", "Enable audit logging for all access to cardholder data"),
        make_control(fw_id, "PCI-11.3", "Regular penetration testing", "Test Security Systems", ControlSeverity::High, false, "", "Perform penetration testing annually"),
    ];
    ComplianceFramework {
        id: fw_id,
        name: "PCI DSS".to_string(),
        kind: FrameworkKind::PciDss,
        version: "4.0".to_string(),
        description: "Payment Card Industry Data Security Standard".to_string(),
        controls,
        created_at: Utc::now(),
    }
}

pub fn hipaa_framework() -> ComplianceFramework {
    let fw_id = Uuid::new_v4();
    let controls = vec![
        make_control(fw_id, "HIPAA-164.308(a)(1)", "Security Management Process", "Administrative Safeguards", ControlSeverity::Critical, false, "", "Implement policies and procedures to prevent security violations"),
        make_control(fw_id, "HIPAA-164.308(a)(5)", "Security Awareness Training", "Administrative Safeguards", ControlSeverity::Medium, false, "", "Implement security awareness and training program"),
        make_control(fw_id, "HIPAA-164.312(a)(1)", "Access Control", "Technical Safeguards", ControlSeverity::Critical, true, "check_access_control", "Implement technical policies for ePHI access"),
        make_control(fw_id, "HIPAA-164.312(b)", "Audit Controls", "Technical Safeguards", ControlSeverity::High, true, "check_hipaa_audit", "Implement audit controls for ePHI access"),
        make_control(fw_id, "HIPAA-164.312(e)(1)", "Transmission Security", "Technical Safeguards", ControlSeverity::High, true, "check_encryption_transit", "Encrypt all ePHI in transit using TLS 1.2+"),
        make_control(fw_id, "HIPAA-164.312(a)(2)(iv)", "Encryption at Rest", "Technical Safeguards", ControlSeverity::High, true, "check_encryption_rest", "Encrypt ePHI at rest"),
    ];
    ComplianceFramework {
        id: fw_id,
        name: "HIPAA".to_string(),
        kind: FrameworkKind::Hipaa,
        version: "2013".to_string(),
        description: "Health Insurance Portability and Accountability Act Security Rule".to_string(),
        controls,
        created_at: Utc::now(),
    }
}

fn make_control(framework_id: Uuid, id: &str, title: &str, category: &str, severity: ControlSeverity, automated: bool, check_fn: &str, remediation: &str) -> Control {
    Control {
        id: Uuid::new_v4(),
        framework_id,
        control_id: id.to_string(),
        title: title.to_string(),
        description: format!("{}: {}", id, title),
        category: category.to_string(),
        severity,
        automated,
        check_fn: if check_fn.is_empty() { None } else { Some(check_fn.to_string()) },
        remediation: remediation.to_string(),
        references: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_frameworks_loaded() {
        let fws = builtin_frameworks();
        assert_eq!(fws.len(), 4);
    }

    #[test]
    fn test_cis_has_controls() {
        let cis = cis_kubernetes_framework();
        assert!(!cis.controls.is_empty());
        assert!(cis.controls.iter().any(|c| c.control_id == "CIS-5.1.1"));
    }

    #[test]
    fn test_all_controls_have_remediation() {
        for fw in builtin_frameworks() {
            for ctrl in &fw.controls {
                assert!(!ctrl.remediation.is_empty(), "Control {} has no remediation", ctrl.control_id);
            }
        }
    }
}
