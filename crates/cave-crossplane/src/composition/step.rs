// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition pipeline `Step` — name + FunctionRef + input JSON + credentials.
//!
//! Upstream: apis/apiextensions/v1/composition_types.go::PipelineStep.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepCredentials {
    pub name: String,
    pub source: CredentialSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CredentialSource {
    Secret { namespace: String, name: String },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub step: String,
    pub function_ref: String,
    pub input: Option<serde_json::Value>,
    pub credentials: Vec<StepCredentials>,
}

impl Step {
    pub fn new(step: impl Into<String>, function_ref: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            function_ref: function_ref.into(),
            input: None,
            credentials: Vec::new(),
        }
    }

    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = Some(input);
        self
    }

    pub fn with_credential(mut self, c: StepCredentials) -> Self {
        self.credentials.push(c);
        self
    }

    /// Estimate the step's serialized footprint (bytes) — used for telemetry.
    pub fn estimated_size(&self) -> usize {
        let i = self.input.as_ref().map(|v| v.to_string().len()).unwrap_or(0);
        self.step.len() + self.function_ref.len() + i
    }
}

/// A step's classified result — `Normal`, `Warning`, `Fatal` — matches upstream's
/// `apiextensions.fn.proto.v1.Result.Severity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepSeverity {
    Normal,
    Warning,
    Fatal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step: String,
    pub severity: StepSeverity,
    pub message: String,
}

impl StepResult {
    pub fn ok(step: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            severity: StepSeverity::Normal,
            message: "ok".into(),
        }
    }
    pub fn warn(step: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            severity: StepSeverity::Warning,
            message: msg.into(),
        }
    }
    pub fn fatal(step: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            severity: StepSeverity::Fatal,
            message: msg.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn step_new_defaults() {
        let s = Step::new("a", "fn");
        assert!(s.input.is_none());
        assert!(s.credentials.is_empty());
        assert_eq!(s.function_ref, "fn");
    }

    #[test]
    fn step_with_input() {
        let s = Step::new("a", "fn").with_input(json!({"k":"v"}));
        assert!(s.input.is_some());
    }

    #[test]
    fn step_with_credential_appends() {
        let s = Step::new("a", "fn").with_credential(StepCredentials {
            name: "creds".into(),
            source: CredentialSource::None,
        });
        assert_eq!(s.credentials.len(), 1);
    }

    #[test]
    fn step_size_nonzero() {
        let s = Step::new("xx", "fn").with_input(json!({"k":"v"}));
        assert!(s.estimated_size() > 0);
    }

    #[test]
    fn step_secret_credential() {
        let c = StepCredentials {
            name: "k".into(),
            source: CredentialSource::Secret {
                namespace: "ns".into(),
                name: "s".into(),
            },
        };
        assert!(matches!(c.source, CredentialSource::Secret { .. }));
    }

    #[test]
    fn severity_ok_warn_fatal() {
        assert_eq!(StepResult::ok("a").severity, StepSeverity::Normal);
        assert_eq!(StepResult::warn("a", "w").severity, StepSeverity::Warning);
        assert_eq!(StepResult::fatal("a", "f").severity, StepSeverity::Fatal);
    }

    #[test]
    fn step_serializes() {
        let s = Step::new("a", "fn");
        let v = serde_json::to_string(&s).unwrap();
        assert!(v.contains("\"step\":\"a\""));
    }
}
