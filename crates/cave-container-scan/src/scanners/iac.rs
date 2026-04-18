use crate::engine::{ScanError, Scanner};
use crate::models::{Finding, FindingCategory, Confidence, ScanKind, ScanRequest, ScanTarget, Severity, IacKind};
use async_trait::async_trait;
use regex::Regex;

pub struct IacScanner;

impl IacScanner {
    fn scan_dockerfile(&self, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // DOCK-001: FROM with :latest
        if Regex::new(r"(?i)FROM\s+.+:latest").unwrap().is_match(content) {
            let mut f = Finding::new(
                "DOCK-001".to_string(),
                "Base image uses :latest tag".to_string(),
                FindingCategory::Misconfig,
                Severity::Medium,
                "Dockerfile uses :latest tag for base image".to_string(),
                "Using :latest tag can lead to unpredictable updates and supply chain risks".to_string(),
            );
            f.remediation = Some("Pin base image to a specific version digest".to_string());
            f.confidence = Confidence::High;
            f.location.file = Some("Dockerfile".to_string());
            findings.push(f);
        }

        // DOCK-002: Missing USER directive
        if !Regex::new(r"(?i)^USER\s+\S+").unwrap().is_match(content) {
            let mut f = Finding::new(
                "DOCK-002".to_string(),
                "Missing USER directive".to_string(),
                FindingCategory::Misconfig,
                Severity::High,
                "Dockerfile does not specify a USER directive".to_string(),
                "Images should explicitly specify a non-root user".to_string(),
            );
            f.remediation = Some("Add USER directive to Dockerfile".to_string());
            f.confidence = Confidence::Confirmed;
            f.location.file = Some("Dockerfile".to_string());
            findings.push(f);
        }

        // DOCK-003: ADD with network URL
        if Regex::new(r"(?i)ADD\s+https?://").unwrap().is_match(content) {
            let mut f = Finding::new(
                "DOCK-003".to_string(),
                "ADD used with network URL".to_string(),
                FindingCategory::Misconfig,
                Severity::Medium,
                "Dockerfile uses ADD with HTTP(S) URL".to_string(),
                "ADD with network URLs can lead to unpredictable caching and supply chain risks".to_string(),
            );
            f.remediation = Some("Use RUN with curl/wget instead of ADD for remote resources".to_string());
            f.confidence = Confidence::High;
            f.location.file = Some("Dockerfile".to_string());
            findings.push(f);
        }

        findings
    }

    fn scan_kubernetes(&self, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // K8S-001: privileged: true
        if content.contains("privileged: true") {
            let mut f = Finding::new(
                "K8S-001".to_string(),
                "Privileged container detected".to_string(),
                FindingCategory::Misconfig,
                Severity::Critical,
                "Kubernetes pod allows privileged containers".to_string(),
                "Privileged containers have unrestricted access to host resources".to_string(),
            );
            f.remediation = Some("Remove privileged: true unless absolutely required".to_string());
            f.confidence = Confidence::Confirmed;
            f.location.file = Some("kubernetes.yaml".to_string());
            findings.push(f);
        }

        // K8S-002: Missing securityContext
        if !content.contains("securityContext") {
            let mut f = Finding::new(
                "K8S-002".to_string(),
                "Missing securityContext".to_string(),
                FindingCategory::Misconfig,
                Severity::Medium,
                "Kubernetes pod lacks securityContext configuration".to_string(),
                "securityContext should be defined for all pods".to_string(),
            );
            f.remediation = Some("Add securityContext with runAsNonRoot: true and other restrictions".to_string());
            f.confidence = Confidence::High;
            f.location.file = Some("kubernetes.yaml".to_string());
            findings.push(f);
        }

        // K8S-003: hostNetwork: true
        if content.contains("hostNetwork: true") {
            let mut f = Finding::new(
                "K8S-003".to_string(),
                "Host network access enabled".to_string(),
                FindingCategory::Misconfig,
                Severity::High,
                "Pod uses host network namespace".to_string(),
                "hostNetwork: true allows access to the host's network stack".to_string(),
            );
            f.remediation = Some("Remove hostNetwork: true unless required".to_string());
            f.confidence = Confidence::Confirmed;
            f.location.file = Some("kubernetes.yaml".to_string());
            findings.push(f);
        }

        // K8S-004: imagePullPolicy: Always with :latest
        if content.contains("imagePullPolicy: Always") && content.contains(":latest") {
            let mut f = Finding::new(
                "K8S-004".to_string(),
                "Insecure image pull policy".to_string(),
                FindingCategory::Misconfig,
                Severity::Medium,
                "Pod uses Always pull policy with :latest tag".to_string(),
                "This can lead to unpredictable image updates".to_string(),
            );
            f.remediation = Some("Use immutable image digests or specific version tags".to_string());
            f.confidence = Confidence::High;
            f.location.file = Some("kubernetes.yaml".to_string());
            findings.push(f);
        }

        findings
    }

    fn scan_terraform(&self, content: &str) -> Vec<Finding> {
        let mut findings = vec![];

        // TF-001: S3 bucket with public read
        if (content.contains("s3") || content.contains("aws_s3")) && content.contains("public-read") {
            let mut f = Finding::new(
                "TF-001".to_string(),
                "S3 bucket allows public read".to_string(),
                FindingCategory::Misconfig,
                Severity::Critical,
                "S3 bucket has public read ACL".to_string(),
                "This exposes bucket contents to the internet".to_string(),
            );
            f.remediation = Some("Change ACL to private or use IAM policies".to_string());
            f.confidence = Confidence::High;
            f.location.file = Some("main.tf".to_string());
            findings.push(f);
        }

        // TF-002: Unrestricted security group ingress
        if content.contains("0.0.0.0/0") && (content.contains("ingress") || content.contains("from_port")) {
            let mut f = Finding::new(
                "TF-002".to_string(),
                "Unrestricted security group ingress".to_string(),
                FindingCategory::Misconfig,
                Severity::High,
                "Security group allows traffic from 0.0.0.0/0".to_string(),
                "This allows inbound traffic from any IP address".to_string(),
            );
            f.remediation = Some("Restrict ingress to specific IPs or use security group IDs".to_string());
            f.confidence = Confidence::High;
            f.location.file = Some("main.tf".to_string());
            findings.push(f);
        }

        findings
    }
}

#[async_trait::async_trait]
impl Scanner for IacScanner {
    fn kind(&self) -> ScanKind {
        ScanKind::Iac
    }

    async fn scan(&self, req: &ScanRequest) -> Result<Vec<Finding>, ScanError> {
        match &req.target {
            ScanTarget::IacBundle { kind, content } => {
                let findings = match kind {
                    IacKind::Dockerfile => self.scan_dockerfile(content),
                    IacKind::Kubernetes => self.scan_kubernetes(content),
                    IacKind::Terraform => self.scan_terraform(content),
                    _ => vec![],
                };
                Ok(findings)
            }
            _ => Err(ScanError::InvalidRequest("Expected IacBundle target".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dockerfile_latest_tag() {
        let scanner = IacScanner;
        let content = "FROM ubuntu:latest\nRUN apt-get install -y curl";
        let req = ScanRequest {
            kind: ScanKind::Iac,
            target: ScanTarget::IacBundle {
                kind: IacKind::Dockerfile,
                content: content.to_string(),
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "DOCK-001"));
    }

    #[tokio::test]
    async fn test_dockerfile_missing_user() {
        let scanner = IacScanner;
        let content = "FROM ubuntu:20.04\nRUN apt-get install -y curl\nCMD [\"bash\"]";
        let req = ScanRequest {
            kind: ScanKind::Iac,
            target: ScanTarget::IacBundle {
                kind: IacKind::Dockerfile,
                content: content.to_string(),
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "DOCK-002"));
    }

    #[tokio::test]
    async fn test_kubernetes_privileged() {
        let scanner = IacScanner;
        let content = r#"
kind: Pod
metadata:
  name: test
spec:
  containers:
  - name: app
    image: myapp:1.0
    securityContext:
      privileged: true
"#;
        let req = ScanRequest {
            kind: ScanKind::Iac,
            target: ScanTarget::IacBundle {
                kind: IacKind::Kubernetes,
                content: content.to_string(),
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "K8S-001"));
    }

    #[tokio::test]
    async fn test_terraform_s3_public() {
        let scanner = IacScanner;
        let content = r#"
resource "aws_s3_bucket_acl" "example" {
  bucket = aws_s3_bucket.example.id
  acl    = "public-read"
}
"#;
        let req = ScanRequest {
            kind: ScanKind::Iac,
            target: ScanTarget::IacBundle {
                kind: IacKind::Terraform,
                content: content.to_string(),
            },
            options: Default::default(),
        };

        let findings = scanner.scan(&req).await.unwrap();
        assert!(findings.iter().any(|f| f.rule_id == "TF-001"));
    }
}
