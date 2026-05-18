// SPDX-License-Identifier: AGPL-3.0-or-later
//! Data types for lint requests and results.

use serde::{Deserialize, Serialize};

use crate::rules::{Severity, Violation};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Dockerfile,
    KubernetesManifest,
    HelmChart,
    TerraformHcl,
    DockerCompose,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LintRequest {
    pub content: String,
    pub content_type: ContentType,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LintResult {
    pub violations: Vec<Violation>,
    pub content_type: ContentType,
    pub total_errors: usize,
    pub total_warnings: usize,
    pub total_info: usize,
    pub passed: bool,
    pub score: u8,
}

impl LintResult {
    pub fn from_violations(violations: Vec<Violation>, content_type: ContentType) -> Self {
        let total_errors = violations
            .iter()
            .filter(|v| matches!(v.severity, Severity::Error))
            .count();
        let total_warnings = violations
            .iter()
            .filter(|v| matches!(v.severity, Severity::Warning))
            .count();
        let total_info = violations
            .iter()
            .filter(|v| matches!(v.severity, Severity::Info))
            .count();

        let deduction = (total_errors * 10 + total_warnings * 5 + total_info * 1).min(100) as u8;
        let score = 100u8.saturating_sub(deduction);
        let passed = total_errors == 0;

        Self {
            violations,
            content_type,
            total_errors,
            total_warnings,
            total_info,
            passed,
            score,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchLintRequest {
    pub files: Vec<LintRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchLintResult {
    pub results: Vec<(String, LintResult)>,
    pub total_errors: usize,
    pub total_warnings: usize,
    pub passed: bool,
}
