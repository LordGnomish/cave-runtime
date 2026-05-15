// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/kubernetes/scanner.go
//! Kubernetes YAML manifest scanner.
//!
//! Walks the parsed YAML once and emits findings for each Pod/Deployment-like
//! resource. Rules ported from upstream trivy `pkg/iac/rules/kubernetes/`:
//!
//! | Rule          | Issue                                | Severity |
//! |---------------|--------------------------------------|----------|
//! | AVD-KSV-0017  | privileged: true                     | Critical |
//! | AVD-KSV-0014  | no runAsNonRoot                      | High     |
//! | AVD-KSV-0013  | image uses :latest or no tag         | Medium   |
//! | AVD-KSV-0009  | hostNetwork: true                    | High     |
//! | AVD-KSV-0011  | no resources.limits                  | Medium   |

use super::{IacError, IacFinding, IacScanner, Severity};
use serde::Deserialize;
use serde_yaml::Value;

#[derive(Default, Clone)]
pub struct KubernetesScanner;

impl KubernetesScanner {
    pub fn new() -> Self {
        Self
    }
}

impl IacScanner for KubernetesScanner {
    fn provider(&self) -> &'static str {
        "kubernetes"
    }

    fn scan_str(&self, content: &str, path: &str) -> Result<Vec<IacFinding>, IacError> {
        let mut out = Vec::new();
        for doc in serde_yaml::Deserializer::from_str(content) {
            let val = Value::deserialize(doc).map_err(|e| IacError::Parse(e.to_string()))?;
            scan_doc(&val, path, &mut out);
        }
        Ok(out)
    }
}

fn scan_doc(v: &Value, path: &str, out: &mut Vec<IacFinding>) {
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("");
    let spec = match kind {
        "Pod" => v.get("spec"),
        "Deployment" | "StatefulSet" | "DaemonSet" | "Job" => v
            .get("spec")
            .and_then(|s| s.get("template"))
            .and_then(|t| t.get("spec")),
        _ => return,
    };
    let Some(spec) = spec else {
        return;
    };
    check_host_network(spec, path, out);
    if let Some(containers) = spec.get("containers").and_then(Value::as_sequence) {
        for c in containers {
            check_container(c, path, out);
        }
    }
}

fn check_host_network(spec: &Value, path: &str, out: &mut Vec<IacFinding>) {
    if spec.get("hostNetwork").and_then(Value::as_bool) == Some(true) {
        out.push(IacFinding {
            rule_id: "AVD-KSV-0009".into(),
            severity: Severity::High,
            message: "Pod uses the host network namespace".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}

fn check_container(c: &Value, path: &str, out: &mut Vec<IacFinding>) {
    let sc = c.get("securityContext");
    // AVD-KSV-0017: privileged
    if sc
        .and_then(|s| s.get("privileged"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        out.push(IacFinding {
            rule_id: "AVD-KSV-0017".into(),
            severity: Severity::Critical,
            message: "Container runs in privileged mode".into(),
            file: path.to_string(),
            line: 0,
        });
    }
    // AVD-KSV-0014: runAsNonRoot != true
    let non_root = sc
        .and_then(|s| s.get("runAsNonRoot"))
        .and_then(Value::as_bool);
    if non_root != Some(true) {
        out.push(IacFinding {
            rule_id: "AVD-KSV-0014".into(),
            severity: Severity::High,
            message: "Container does not enforce runAsNonRoot".into(),
            file: path.to_string(),
            line: 0,
        });
    }
    // AVD-KSV-0013: image uses :latest
    if let Some(img) = c.get("image").and_then(Value::as_str) {
        let bad_tag = match img.rsplit_once(':') {
            Some((_, tag)) => tag == "latest",
            None => true, // no tag implicitly :latest
        };
        if bad_tag {
            out.push(IacFinding {
                rule_id: "AVD-KSV-0013".into(),
                severity: Severity::Medium,
                message: format!("Container image `{img}` uses :latest or no tag"),
                file: path.to_string(),
                line: 0,
            });
        }
    }
    // AVD-KSV-0011: no resources.limits
    let has_limits = c
        .get("resources")
        .and_then(|r| r.get("limits"))
        .is_some_and(|l| l.as_mapping().is_some_and(|m| !m.is_empty()));
    if !has_limits {
        out.push(IacFinding {
            rule_id: "AVD-KSV-0011".into(),
            severity: Severity::Medium,
            message: "Container has no resource limits set".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}
