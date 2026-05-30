// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Custom benchmark rule authoring.
//!
//! Upstream:
//! - kube-bench custom checks — operators drop `cfg/<version>/*.yaml` files
//!   (groups[].checks[] with text/audit/tests) and load them via
//!   `--config-dir`; `check/controls.go::NewControls` validates ids.
//! - kubescape custom frameworks — `kubescape scan framework <custom.json>`
//!   loads a JSON framework and validates control ids / rules
//!   (`core/pkg/policyhandler`).
//!
//! cave-bench unifies both: a custom control embeds a [`CustomRule`] that
//! reuses the CIS engine ([`crate::cis_engine`] TestItem/BinOp/ValueSource),
//! so authored rules are evaluated by the *same* evaluator as the built-in
//! catalogues — no second rule interpreter, no stub.

use crate::cis_engine::{CisContext, CisRule, Logic, TestItem, evaluate_rule};
use crate::error::{BenchError, Result};
use crate::models::{Check, CisLevel, Finding, Framework, NodeType, Profile, Severity};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;

/// A CIS-engine rule body, authored by an operator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomRule {
    #[serde(default)]
    pub items: Vec<TestItem>,
    #[serde(default = "default_logic")]
    pub logic: Logic,
    #[serde(default)]
    pub manual: bool,
}

fn default_logic() -> Logic {
    Logic::And
}

impl Default for CustomRule {
    fn default() -> Self {
        CustomRule { items: Vec::new(), logic: Logic::And, manual: false }
    }
}

/// One operator-authored control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomControlSpec {
    pub control_id: String,
    pub name: String,
    #[serde(with = "severity_str")]
    pub severity: Severity,
    #[serde(with = "node_type_str")]
    pub node_type: NodeType,
    #[serde(default)]
    pub remediation: String,
    #[serde(default)]
    pub rule: CustomRule,
}

/// A full operator-authored framework — a runnable overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomFrameworkSpec {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub controls: Vec<CustomControlSpec>,
}

impl CustomFrameworkSpec {
    /// Parse a kubescape-style custom-framework JSON document.
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(BenchError::from)
    }

    /// Serialize back to JSON (round-trip / export).
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(BenchError::from)
    }

    /// Validate authored controls before they may be registered/run.
    ///
    /// Mirrors `NewControls` (kube-bench) + kubescape framework validation:
    /// non-empty id, ≥1 control, unique control ids, valid id charset, and
    /// every non-manual control must carry ≥1 test item.
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(BenchError::ControlInvalid("framework id is empty".into()));
        }
        if !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
            return Err(BenchError::ControlInvalid(format!("framework id '{}' has invalid chars", self.id)));
        }
        if self.controls.is_empty() {
            return Err(BenchError::ControlInvalid(format!("framework '{}' has no controls", self.id)));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for c in &self.controls {
            if c.control_id.trim().is_empty() {
                return Err(BenchError::ControlInvalid("control_id is empty".into()));
            }
            if !seen.insert(c.control_id.as_str()) {
                return Err(BenchError::ControlInvalid(format!("duplicate control_id '{}'", c.control_id)));
            }
            if c.name.trim().is_empty() {
                return Err(BenchError::ControlInvalid(format!("control '{}' has empty name", c.control_id)));
            }
            if !c.rule.manual && c.rule.items.is_empty() {
                return Err(BenchError::ControlInvalid(format!(
                    "non-manual control '{}' must declare ≥1 test item",
                    c.control_id
                )));
            }
        }
        Ok(())
    }

    /// All control ids in authored order.
    pub fn control_ids(&self) -> Vec<String> {
        self.controls.iter().map(|c| c.control_id.clone()).collect()
    }

    /// Build a runnable [`Profile`] referencing this framework's controls.
    pub fn to_profile(&self) -> Profile {
        Profile {
            id: self.id.clone(),
            framework: Framework::Custom,
            name: self.name.clone(),
            description: self.description.clone(),
            check_ids: self.control_ids(),
        }
    }

    /// One control → ([`Check`], [`CisRule`]) suitable for the CIS engine.
    fn lower(&self, c: &CustomControlSpec) -> (Check, CisRule) {
        let mut check = Check::new(&c.control_id, Framework::Custom, c.node_type.clone(), &c.name);
        check.severity = c.severity;
        check.remediation = c.remediation.clone();
        check.level = if matches!(c.severity, Severity::Critical | Severity::High) {
            CisLevel::L1
        } else {
            CisLevel::L2
        };
        check.tags = vec!["custom".into(), self.id.clone()];

        let mut rule = CisRule::new(&c.control_id, &c.name);
        rule.remediation = c.remediation.clone();
        rule.items = c.rule.items.clone();
        rule.logic = c.rule.logic;
        rule.manual = c.rule.manual;
        (check, rule)
    }

    /// Evaluate every authored control against a CIS context. Flags are keyed
    /// by the control's `node_type` label (e.g. "master"/"node"/"etcd").
    pub fn evaluate(&self, ctx: &CisContext, host: &str) -> Vec<Finding> {
        self.controls
            .iter()
            .map(|c| {
                let (check, rule) = self.lower(c);
                let bin = c.node_type.as_str();
                evaluate_rule(&rule, &check, ctx, bin, host)
            })
            .collect()
    }
}

/// Fluent builder for authoring a custom framework in Rust.
#[derive(Debug, Clone)]
pub struct CustomFrameworkBuilder {
    spec: CustomFrameworkSpec,
}

impl CustomFrameworkBuilder {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        CustomFrameworkBuilder {
            spec: CustomFrameworkSpec {
                id: id.into(),
                name: name.into(),
                version: String::new(),
                description: String::new(),
                controls: Vec::new(),
            },
        }
    }

    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.spec.version = v.into();
        self
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.spec.description = d.into();
        self
    }

    pub fn control(
        mut self,
        control_id: impl Into<String>,
        name: impl Into<String>,
        severity: Severity,
        node_type: &str,
        remediation: impl Into<String>,
        rule: CustomRule,
    ) -> Self {
        self.spec.controls.push(CustomControlSpec {
            control_id: control_id.into(),
            name: name.into(),
            severity,
            node_type: parse_node_type(node_type).unwrap_or(NodeType::Policies),
            remediation: remediation.into(),
            rule,
        });
        self
    }

    pub fn build(self) -> CustomFrameworkSpec {
        self.spec
    }
}

/// Thread-safe registry of registered custom frameworks.
#[derive(Debug, Default)]
pub struct CustomRegistry {
    inner: Mutex<Vec<CustomFrameworkSpec>>,
}

impl CustomRegistry {
    /// Validate then register a custom framework (rejects duplicate ids).
    pub fn register(&self, spec: CustomFrameworkSpec) -> Result<()> {
        spec.validate()?;
        let mut g = self.inner.lock().unwrap();
        if g.iter().any(|s| s.id == spec.id) {
            return Err(BenchError::ControlInvalid(format!("framework '{}' already registered", spec.id)));
        }
        g.push(spec);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<CustomFrameworkSpec> {
        self.inner.lock().unwrap().iter().find(|s| s.id == id).cloned()
    }

    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn list_ids(&self) -> Vec<String> {
        self.inner.lock().unwrap().iter().map(|s| s.id.clone()).collect()
    }

    pub fn remove(&self, id: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        let before = g.len();
        g.retain(|s| s.id != id);
        g.len() != before
    }
}

// ─── lowercase string ↔ enum serde helpers ──────────────────────────────────

fn parse_severity(s: &str) -> Option<Severity> {
    Some(match s.to_ascii_lowercase().as_str() {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        "info" => Severity::Info,
        _ => return None,
    })
}

fn parse_node_type(s: &str) -> Option<NodeType> {
    Some(match s.to_ascii_lowercase().as_str() {
        "master" => NodeType::Master,
        "node" => NodeType::Node,
        "etcd" => NodeType::Etcd,
        "controlplane" | "control-plane" => NodeType::ControlPlane,
        "policies" => NodeType::Policies,
        "managedservices" => NodeType::Managedservices,
        _ => return None,
    })
}

mod severity_str {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Severity, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(v.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Severity, D::Error> {
        let raw = String::deserialize(d)?;
        parse_severity(&raw).ok_or_else(|| serde::de::Error::custom(format!("unknown severity '{raw}'")))
    }
}

mod node_type_str {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &NodeType, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(v.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<NodeType, D::Error> {
        let raw = String::deserialize(d)?;
        parse_node_type(&raw).ok_or_else(|| serde::de::Error::custom(format!("unknown node_type '{raw}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cis_engine::{BinOp, ValueSource};

    fn one_item_rule() -> CustomRule {
        CustomRule {
            items: vec![TestItem {
                source: ValueSource::Flag("--x".into()),
                op: BinOp::Eq,
                value: "1".into(),
                set: Some(true),
            }],
            logic: Logic::And,
            manual: false,
        }
    }

    #[test]
    fn test_json_round_trip() {
        let spec = CustomFrameworkBuilder::new("rt", "RoundTrip")
            .version("9")
            .control("R-1", "c", Severity::Low, "node", "fix", one_item_rule())
            .build();
        let json = spec.to_json().unwrap();
        let back = CustomFrameworkSpec::from_json(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn test_validate_invalid_id_chars() {
        let spec = CustomFrameworkBuilder::new("bad id!", "x")
            .control("A", "n", Severity::Low, "node", "", one_item_rule())
            .build();
        assert!(spec.validate().is_err());
    }

    #[test]
    fn test_manual_control_needs_no_items() {
        let spec = CustomFrameworkBuilder::new("m", "M")
            .control("M-1", "manual", Severity::Medium, "policies", "", CustomRule { manual: true, ..Default::default() })
            .build();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn test_registry_remove() {
        let reg = CustomRegistry::default();
        let spec = CustomFrameworkBuilder::new("z", "Z")
            .control("Z-1", "n", Severity::Low, "node", "", one_item_rule())
            .build();
        reg.register(spec).unwrap();
        assert!(reg.remove("z"));
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_parse_severity_helper() {
        assert_eq!(parse_severity("CRITICAL"), Some(Severity::Critical));
        assert_eq!(parse_severity("bogus"), None);
    }
}
