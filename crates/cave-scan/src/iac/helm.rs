// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/helm/scanner.go
//! Helm chart scanner.
//!
//! Helm scanning in trivy actually shells out to `helm template` to render the
//! chart then re-feeds the YAML through the Kubernetes scanner. We can't do
//! that here without bringing in a Go runtime, so the scanner walks the
//! Chart.yaml + values.yaml directly:
//!
//! | Rule          | Issue                              | Severity |
//! |---------------|------------------------------------|----------|
//! | AVD-HELM-0001 | Chart.yaml missing appVersion      | Low      |
//! | AVD-HELM-0002 | values.yaml defaults privileged    | High     |
//! | AVD-HELM-0003 | values.yaml mounts hostPath        | Medium   |
//! | AVD-HELM-0004 | values.yaml empty image.tag        | Medium   |
//! | AVD-HELM-0005 | Chart.yaml type != application|library | Low |
//!
//! Live `helm template` rendering deferred — see parity.manifest.toml
//! `status="missing"` entries.

use super::{IacError, IacFinding, IacScanner, Severity};
use serde_yaml::Value;

#[derive(Default, Clone)]
pub struct HelmScanner;

impl HelmScanner {
    pub fn new() -> Self {
        Self
    }
}

impl IacScanner for HelmScanner {
    fn provider(&self) -> &'static str {
        "helm"
    }

    fn scan_str(&self, content: &str, path: &str) -> Result<Vec<IacFinding>, IacError> {
        let v: Value = serde_yaml::from_str(content).map_err(|e| IacError::Parse(e.to_string()))?;
        let mut out = Vec::new();
        if path.ends_with("Chart.yaml") || path.ends_with("chart.yaml") {
            scan_chart(&v, path, &mut out);
        } else {
            scan_values(&v, path, &mut out);
        }
        Ok(out)
    }
}

fn scan_chart(v: &Value, path: &str, out: &mut Vec<IacFinding>) {
    if v.get("appVersion").is_none() {
        out.push(IacFinding {
            rule_id: "AVD-HELM-0001".into(),
            severity: Severity::Low,
            message: "Chart.yaml lacks appVersion — consumers can't pin upstream".into(),
            file: path.to_string(),
            line: 0,
        });
    }
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("application");
    if kind != "application" && kind != "library" {
        out.push(IacFinding {
            rule_id: "AVD-HELM-0005".into(),
            severity: Severity::Low,
            message: format!("Chart.yaml has unknown type `{kind}`"),
            file: path.to_string(),
            line: 0,
        });
    }
}

fn scan_values(v: &Value, path: &str, out: &mut Vec<IacFinding>) {
    // Recursively look for securityContext.privileged: true
    if has_privileged(v) {
        out.push(IacFinding {
            rule_id: "AVD-HELM-0002".into(),
            severity: Severity::High,
            message: "values.yaml default makes container privileged".into(),
            file: path.to_string(),
            line: 0,
        });
    }
    if has_hostpath(v) {
        out.push(IacFinding {
            rule_id: "AVD-HELM-0003".into(),
            severity: Severity::Medium,
            message: "values.yaml mounts a hostPath by default".into(),
            file: path.to_string(),
            line: 0,
        });
    }
    if has_empty_image_tag(v) {
        out.push(IacFinding {
            rule_id: "AVD-HELM-0004".into(),
            severity: Severity::Medium,
            message: "values.yaml leaves image.tag empty (resolves to :latest)".into(),
            file: path.to_string(),
            line: 0,
        });
    }
}

fn has_privileged(v: &Value) -> bool {
    if let Some(sc) = v.get("securityContext") {
        if sc.get("privileged").and_then(Value::as_bool) == Some(true) {
            return true;
        }
    }
    if let Some(map) = v.as_mapping() {
        for (_, child) in map {
            if has_privileged(child) {
                return true;
            }
        }
    }
    false
}

fn has_hostpath(v: &Value) -> bool {
    if let Some(seq) = v.get("volumes").and_then(Value::as_sequence) {
        for vol in seq {
            if vol.get("hostPath").is_some() {
                return true;
            }
        }
    }
    if let Some(map) = v.as_mapping() {
        for (_, child) in map {
            if has_hostpath(child) {
                return true;
            }
        }
    }
    false
}

fn has_empty_image_tag(v: &Value) -> bool {
    if let Some(img) = v.get("image") {
        let tag = img.get("tag").and_then(Value::as_str);
        if tag == Some("") {
            return true;
        }
    }
    false
}
