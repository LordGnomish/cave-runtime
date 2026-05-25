// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/cloudformation/scanner.go
//! AWS CloudFormation template scanner.
//!
//! Walks the `Resources` map and dispatches per `Type`. Rules:
//!
//! | Rule          | Issue                                | Severity |
//! |---------------|--------------------------------------|----------|
//! | AVD-CFN-0001  | S3 bucket AccessControl Public*      | High     |
//! | AVD-CFN-0002  | SecurityGroup ingress 0.0.0.0/0      | Critical |
//! | AVD-CFN-0003  | RDS DBInstance PubliclyAccessible    | High     |
//! | AVD-CFN-0004  | KMS Key with no key rotation         | Medium   |
//! | AVD-CFN-0005  | CloudTrail without log file validation | Medium |
//!
//! Only YAML CFN templates are accepted (the JSON dialect would parse via the
//! same `serde_yaml` since YAML is a JSON superset).

use super::{IacError, IacFinding, IacScanner, Severity};
use serde_yaml::Value;

#[derive(Default, Clone)]
pub struct CloudFormationScanner;

impl CloudFormationScanner {
    pub fn new() -> Self {
        Self
    }
}

impl IacScanner for CloudFormationScanner {
    fn provider(&self) -> &'static str {
        "cloudformation"
    }

    fn scan_str(&self, content: &str, path: &str) -> Result<Vec<IacFinding>, IacError> {
        let v: Value = serde_yaml::from_str(content).map_err(|e| IacError::Parse(e.to_string()))?;
        let mut out = Vec::new();
        let Some(resources) = v.get("Resources").and_then(Value::as_mapping) else {
            return Ok(out);
        };
        for (_name, res) in resources {
            let kind = res.get("Type").and_then(Value::as_str).unwrap_or("");
            let props = res.get("Properties");
            match kind {
                "AWS::S3::Bucket" => scan_s3(props, path, &mut out),
                "AWS::EC2::SecurityGroup" => scan_sg(props, path, &mut out),
                "AWS::RDS::DBInstance" => scan_rds(props, path, &mut out),
                "AWS::KMS::Key" => scan_kms(props, path, &mut out),
                "AWS::CloudTrail::Trail" => scan_trail(props, path, &mut out),
                _ => {}
            }
        }
        Ok(out)
    }
}

fn scan_s3(props: Option<&Value>, path: &str, out: &mut Vec<IacFinding>) {
    let Some(p) = props else { return };
    if let Some(acl) = p.get("AccessControl").and_then(Value::as_str) {
        if acl.starts_with("Public") {
            out.push(IacFinding {
                rule_id: "AVD-CFN-0001".into(),
                severity: Severity::High,
                message: format!("S3 bucket AccessControl is `{acl}`"),
                file: path.to_string(),
                line: 0,
            });
        }
    }
}

fn scan_sg(props: Option<&Value>, path: &str, out: &mut Vec<IacFinding>) {
    let Some(p) = props else { return };
    let Some(ingress) = p.get("SecurityGroupIngress").and_then(Value::as_sequence) else {
        return;
    };
    for rule in ingress {
        let cidr = rule.get("CidrIp").and_then(Value::as_str).unwrap_or("");
        let cidr6 = rule.get("CidrIpv6").and_then(Value::as_str).unwrap_or("");
        if cidr == "0.0.0.0/0" || cidr6 == "::/0" {
            out.push(IacFinding {
                rule_id: "AVD-CFN-0002".into(),
                severity: Severity::Critical,
                message: "SecurityGroup ingress permits 0.0.0.0/0".into(),
                file: path.to_string(),
                line: 0,
            });
            return;
        }
    }
}

fn scan_rds(props: Option<&Value>, path: &str, out: &mut Vec<IacFinding>) {
    let Some(p) = props else { return };
    if p.get("PubliclyAccessible").and_then(Value::as_bool) == Some(true) {
        out.push(IacFinding {
            rule_id: "AVD-CFN-0003".into(),
            severity: Severity::High,
            message: "RDS DBInstance is publicly accessible".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}

fn scan_kms(props: Option<&Value>, path: &str, out: &mut Vec<IacFinding>) {
    let Some(p) = props else { return };
    if p.get("EnableKeyRotation").and_then(Value::as_bool) != Some(true) {
        out.push(IacFinding {
            rule_id: "AVD-CFN-0004".into(),
            severity: Severity::Medium,
            message: "KMS key does not have automatic rotation enabled".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}

fn scan_trail(props: Option<&Value>, path: &str, out: &mut Vec<IacFinding>) {
    let Some(p) = props else { return };
    if p.get("EnableLogFileValidation").and_then(Value::as_bool) != Some(true) {
        out.push(IacFinding {
            rule_id: "AVD-CFN-0005".into(),
            severity: Severity::Medium,
            message: "CloudTrail trail has no log file validation".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}
