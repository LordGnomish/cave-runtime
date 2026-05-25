// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/terraform/scanner.go
//! Terraform HCL scanner.
//!
//! Built on `hcl-rs`. Walks the parsed `Body` once, dispatching per-resource-type
//! checks. Currently implements:
//!
//! | Rule          | Resource                      | CIS / control |
//! |---------------|-------------------------------|---------------|
//! | AVD-AWS-0001  | aws_s3_bucket / acl=public-*  | CIS 1.20      |
//! | AVD-AWS-0088  | aws_s3_bucket / no SSE config | CIS 2.1.1     |
//! | AVD-AWS-0107  | aws_security_group / 0.0.0.0/0| CIS 5.2       |
//! | AVD-AWS-0057  | aws_iam_policy / wildcard "*" | CIS 1.16      |
//! | AVD-AWS-0017  | aws_db_instance / public      | CIS 2.3       |
//!
//! Implementation is pure structural pattern matching — no Rego, no
//! data-flow analysis (upstream uses tfvars resolution + interpolation).

use super::{IacError, IacFinding, IacScanner, Severity};
use hcl::{Block, Body, Expression};

/// Terraform scanner with a fixed rule set.
#[derive(Default, Clone)]
pub struct TerraformScanner;

impl TerraformScanner {
    pub fn new() -> Self {
        Self
    }
}

impl IacScanner for TerraformScanner {
    fn provider(&self) -> &'static str {
        "terraform"
    }

    fn scan_str(&self, content: &str, path: &str) -> Result<Vec<IacFinding>, IacError> {
        let body: Body = hcl::from_str(content).map_err(|e| IacError::Parse(e.to_string()))?;
        let mut out = Vec::new();
        for s in body.into_iter() {
            if let hcl::Structure::Block(b) = s {
                if b.identifier.as_str() == "resource" {
                    scan_resource(&b, path, &mut out);
                }
            }
        }
        Ok(out)
    }
}

fn scan_resource(b: &Block, path: &str, out: &mut Vec<IacFinding>) {
    let kind = b
        .labels
        .first()
        .map(|l| l.as_str().to_string())
        .unwrap_or_default();
    match kind.as_str() {
        "aws_s3_bucket" => scan_s3(b, path, out),
        "aws_security_group" => scan_sg(b, path, out),
        "aws_iam_policy" => scan_iam_policy(b, path, out),
        "aws_db_instance" => scan_db_instance(b, path, out),
        _ => {}
    }
}

fn attr_value<'a>(b: &'a Block, name: &str) -> Option<&'a Expression> {
    b.body
        .attributes()
        .find(|a| a.key.as_str() == name)
        .map(|a| &a.expr)
}

fn nested_block<'a>(b: &'a Block, ident: &str) -> Option<&'a Block> {
    b.body.blocks().find(|nb| nb.identifier.as_str() == ident)
}

fn expr_as_str(e: &Expression) -> Option<String> {
    if let Expression::String(s) = e {
        Some(s.clone())
    } else {
        None
    }
}

fn scan_s3(b: &Block, path: &str, out: &mut Vec<IacFinding>) {
    // AVD-AWS-0001: ACL is public-*
    if let Some(acl_expr) = attr_value(b, "acl") {
        if let Some(s) = expr_as_str(acl_expr) {
            if s.starts_with("public-") {
                out.push(IacFinding {
                    rule_id: "AVD-AWS-0001".into(),
                    severity: Severity::High,
                    message: format!("S3 bucket has public ACL `{s}`"),
                    file: path.to_string(),
                    line: 0,
                });
            }
        }
    }
    // AVD-AWS-0088: missing server_side_encryption_configuration
    if nested_block(b, "server_side_encryption_configuration").is_none() {
        out.push(IacFinding {
            rule_id: "AVD-AWS-0088".into(),
            severity: Severity::High,
            message: "S3 bucket lacks server-side encryption configuration".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}

fn scan_sg(b: &Block, path: &str, out: &mut Vec<IacFinding>) {
    // AVD-AWS-0107: ingress with 0.0.0.0/0
    for nb in b.body.blocks() {
        if nb.identifier.as_str() != "ingress" {
            continue;
        }
        if let Some(cidr_expr) = attr_value(nb, "cidr_blocks") {
            if let Expression::Array(items) = cidr_expr {
                for it in items {
                    if let Some(s) = expr_as_str(it) {
                        if s == "0.0.0.0/0" || s == "::/0" {
                            out.push(IacFinding {
                                rule_id: "AVD-AWS-0107".into(),
                                severity: Severity::Critical,
                                message: "Security group ingress permits 0.0.0.0/0".into(),
                                file: path.to_string(),
                                line: 0,
                            });
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn scan_iam_policy(b: &Block, path: &str, out: &mut Vec<IacFinding>) {
    // AVD-AWS-0057: policy contains "Action": "*"
    if let Some(p) = attr_value(b, "policy") {
        if let Some(s) = expr_as_str(p) {
            if s.contains("\"Action\":\"*\"") || s.contains("\"Action\": \"*\"") {
                out.push(IacFinding {
                    rule_id: "AVD-AWS-0057".into(),
                    severity: Severity::High,
                    message: "IAM policy grants wildcard Action `*`".into(),
                    file: path.to_string(),
                    line: 0,
                });
            }
        }
    }
}

fn scan_db_instance(b: &Block, path: &str, out: &mut Vec<IacFinding>) {
    // AVD-AWS-0017: publicly_accessible = true
    if let Some(e) = attr_value(b, "publicly_accessible") {
        if let Expression::Bool(true) = e {
            out.push(IacFinding {
                rule_id: "AVD-AWS-0017".into(),
                severity: Severity::High,
                message: "RDS instance is publicly accessible".into(),
                file: path.to_string(),
                line: 0,
            });
        }
    }
}
