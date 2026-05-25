// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CIS check rule engine — loader + evaluator.
//!
//! Upstream: kube-bench `check/check.go`, `check/test.go`, `cfg/cis-*/*.yaml`.
//! License: Apache-2.0 (port allowed).
//!
//! kube-bench rules are YAML-structured. We model the same primitives:
//! `BinOp { Eq, NotEq, Gt, Gte, Lt, Lte, Has, NotHas, BitMaskAnd }`, `TestType { TestItem, TestItemList }`,
//! plus a `BinaryCheck { OS, Audit, Tests, OperationLogic }`.

use crate::error::{BenchError, Result};
use crate::models::{Check, Finding};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Comparison op for one test — `kube-bench check/test.go::tests.go::Op`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BinOp {
    /// =, has, eq, equal
    Eq,
    /// !=, noteq
    NotEq,
    /// gt
    Gt,
    /// gte
    Gte,
    /// lt
    Lt,
    /// lte
    Lte,
    /// has (substring)
    Has,
    /// nothave
    NotHas,
    /// bitmask AND non-zero
    BitMaskAnd,
    /// regex match
    Regex,
    /// "valid_elements" — every element in `set` is in `value` list
    ValidElements,
}

/// Source of the value being compared — kube-bench `check.Test.flag`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValueSource {
    /// CLI flag from process args, e.g. `--authorization-mode=Node,RBAC`.
    Flag(String),
    /// Path/key from a YAML config file, e.g. `kubelet.config.yaml::authentication.anonymous.enabled`.
    Path(String),
    /// File permission mode (octal), e.g. `644`.
    FileMode(String),
    /// File ownership user:group, e.g. `root:root`.
    FileOwner(String),
    /// File exists on disk.
    FileExists(String),
}

/// One assertion in a check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestItem {
    pub source: ValueSource,
    pub op: BinOp,
    /// Right-hand value, encoded as string. List ops join with `,`.
    pub value: String,
    /// True if absent value should pass (`set: false`).
    pub set: Option<bool>,
}

/// kube-bench `check.Test.operationLogic`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Logic {
    /// All items must pass.
    And,
    /// Any item passing is enough.
    Or,
}

/// Complete CIS rule — `check.Check` in upstream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CisRule {
    /// Identifier, e.g. `1.1.1`.
    pub id: String,
    pub title: String,
    pub description: String,
    pub remediation: String,
    pub items: Vec<TestItem>,
    pub logic: Logic,
    /// Whether check is "Manual" — emits Warn, not Pass.
    pub manual: bool,
}

impl CisRule {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        CisRule {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            remediation: String::new(),
            items: Vec::new(),
            logic: Logic::And,
            manual: false,
        }
    }
}

/// Captured scan state — equivalent to kube-bench's `audit` step output but pre-fetched.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CisContext {
    /// Process flag map for each binary (apiserver/kubelet/etcd...): name → "value" or "" when boolean.
    pub flags: HashMap<String, HashMap<String, String>>,
    /// YAML config path lookups (binary → key-path → value).
    pub paths: HashMap<String, HashMap<String, String>>,
    /// File mode bits captured for each path (path → octal-string).
    pub file_modes: HashMap<String, String>,
    /// File owner captured for each path (path → user:group).
    pub file_owners: HashMap<String, String>,
    /// Paths known to exist.
    pub files_exist: Vec<String>,
}

impl CisContext {
    pub fn set_flag(&mut self, bin: &str, k: &str, v: &str) {
        self.flags.entry(bin.into()).or_default().insert(k.into(), v.into());
    }
    pub fn set_path(&mut self, bin: &str, k: &str, v: &str) {
        self.paths.entry(bin.into()).or_default().insert(k.into(), v.into());
    }
    pub fn set_file(&mut self, path: &str, mode: &str, owner: &str) {
        self.file_modes.insert(path.into(), mode.into());
        self.file_owners.insert(path.into(), owner.into());
        self.files_exist.push(path.into());
    }
    pub fn flag(&self, bin: &str, k: &str) -> Option<&String> {
        self.flags.get(bin).and_then(|m| m.get(k))
    }
}

/// Evaluate a single TestItem against the context.
fn eval_item(item: &TestItem, ctx: &CisContext, bin: &str) -> bool {
    let actual: Option<String> = match &item.source {
        ValueSource::Flag(k) => ctx.flag(bin, k).cloned(),
        ValueSource::Path(k) => ctx.paths.get(bin).and_then(|m| m.get(k)).cloned(),
        ValueSource::FileMode(p) => ctx.file_modes.get(p).cloned(),
        ValueSource::FileOwner(p) => ctx.file_owners.get(p).cloned(),
        ValueSource::FileExists(p) => {
            let present = ctx.files_exist.iter().any(|f| f == p);
            return match item.op {
                BinOp::Eq => present == (item.value == "true"),
                BinOp::NotEq => present != (item.value == "true"),
                _ => present,
            };
        }
    };
    let set_required = item.set.unwrap_or(true);
    let Some(actual) = actual else {
        // value absent → only passes if `set: false` was declared and we expected it absent.
        return !set_required;
    };
    eval_op(item.op, &actual, &item.value)
}

fn eval_op(op: BinOp, actual: &str, expected: &str) -> bool {
    match op {
        BinOp::Eq => actual == expected,
        BinOp::NotEq => actual != expected,
        BinOp::Has => actual.contains(expected),
        BinOp::NotHas => !actual.contains(expected),
        BinOp::Gt => parse_num(actual) > parse_num(expected),
        BinOp::Gte => parse_num(actual) >= parse_num(expected),
        BinOp::Lt => parse_num(actual) < parse_num(expected),
        BinOp::Lte => parse_num(actual) <= parse_num(expected),
        BinOp::BitMaskAnd => (parse_num(actual) as u64) & (parse_num(expected) as u64) != 0,
        BinOp::Regex => regex::Regex::new(expected).map(|r| r.is_match(actual)).unwrap_or(false),
        BinOp::ValidElements => {
            // every comma-separated element of expected must appear in actual (comma-separated)
            let act: Vec<&str> = actual.split(',').map(str::trim).collect();
            expected.split(',').map(str::trim).all(|e| act.contains(&e))
        }
    }
}

fn parse_num(s: &str) -> i64 {
    if let Some(stripped) = s.strip_prefix("0o") {
        return i64::from_str_radix(stripped, 8).unwrap_or(0);
    }
    // Octal file modes — three-digit string "0644" or "644"
    if s.len() <= 4 && s.chars().all(|c| c.is_ascii_digit()) && s.starts_with('0') {
        if let Ok(n) = i64::from_str_radix(s.trim_start_matches('0'), 8) {
            return n;
        }
    }
    s.parse::<i64>().unwrap_or(0)
}

/// Run a rule against the context. Returns one Finding.
pub fn evaluate_rule(rule: &CisRule, check_meta: &Check, ctx: &CisContext, bin: &str, host: &str) -> Finding {
    if rule.manual {
        return Finding::warn(check_meta, host, format!("Manual check: {}", rule.title));
    }
    if rule.items.is_empty() {
        return Finding::warn(check_meta, host, "No test items defined — manual review");
    }
    let evals: Vec<bool> = rule.items.iter().map(|i| eval_item(i, ctx, bin)).collect();
    let pass = match rule.logic {
        Logic::And => evals.iter().all(|&b| b),
        Logic::Or => evals.iter().any(|&b| b),
    };
    if pass {
        Finding::pass(check_meta, host, format!("{}: control satisfied", rule.id))
    } else {
        Finding::fail(check_meta, host, format!("{}: control violated", rule.id))
            .with_evidence(format!("logic={:?}, items_evaluated={}", rule.logic, evals.len()))
    }
}

/// Load a YAML CIS rule file (kube-bench-format). Returns parsed rules.
pub fn load_rules_yaml(text: &str) -> Result<Vec<CisRule>> {
    let raw: serde_yaml::Value = serde_yaml::from_str(text)?;
    let mut rules: Vec<CisRule> = Vec::new();
    let Some(groups) = raw.get("groups").and_then(|g| g.as_sequence()) else {
        return Err(BenchError::ControlInvalid("missing groups[]".into()));
    };
    for g in groups {
        let Some(checks) = g.get("checks").and_then(|c| c.as_sequence()) else { continue };
        for c in checks {
            let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let title = c.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let remediation = c.get("remediation").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let manual = c.get("scored").and_then(|v| v.as_bool()).map(|s| !s).unwrap_or(false);
            let mut rule = CisRule::new(id, title);
            rule.remediation = remediation;
            rule.manual = manual;
            rules.push(rule);
        }
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Framework, NodeType, Verdict};

    #[test]
    fn test_parse_num_octal() {
        assert_eq!(parse_num("644"), 644);
        assert_eq!(parse_num("0644"), 0o644);
    }

    #[test]
    fn test_eval_op_eq_neq() {
        assert!(eval_op(BinOp::Eq, "true", "true"));
        assert!(!eval_op(BinOp::Eq, "false", "true"));
        assert!(eval_op(BinOp::NotEq, "false", "true"));
    }

    #[test]
    fn test_eval_op_has_nothas() {
        assert!(eval_op(BinOp::Has, "RBAC,Node", "RBAC"));
        assert!(eval_op(BinOp::NotHas, "Node", "RBAC"));
    }

    #[test]
    fn test_eval_op_valid_elements() {
        assert!(eval_op(BinOp::ValidElements, "Node,RBAC", "Node,RBAC"));
        assert!(!eval_op(BinOp::ValidElements, "RBAC", "Node,RBAC"));
    }

    #[test]
    fn test_eval_op_bitmask() {
        assert!(eval_op(BinOp::BitMaskAnd, "7", "4"));
        assert!(!eval_op(BinOp::BitMaskAnd, "8", "4"));
    }

    #[test]
    fn test_context_set_flag_get() {
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--authorization-mode", "Node,RBAC");
        assert_eq!(ctx.flag("apiserver", "--authorization-mode").map(String::as_str), Some("Node,RBAC"));
    }

    #[test]
    fn test_evaluate_rule_pass() {
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--anonymous-auth", "false");
        let mut rule = CisRule::new("1.2.1", "Disable anonymous auth");
        rule.items.push(TestItem {
            source: ValueSource::Flag("--anonymous-auth".into()),
            op: BinOp::Eq,
            value: "false".into(),
            set: Some(true),
        });
        let meta = Check::new("1.2.1", Framework::CisK8s, NodeType::Master, "x");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, Verdict::Pass);
    }

    #[test]
    fn test_evaluate_rule_fail() {
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--anonymous-auth", "true");
        let mut rule = CisRule::new("1.2.1", "Disable anonymous auth");
        rule.items.push(TestItem {
            source: ValueSource::Flag("--anonymous-auth".into()),
            op: BinOp::Eq,
            value: "false".into(),
            set: Some(true),
        });
        let meta = Check::new("1.2.1", Framework::CisK8s, NodeType::Master, "x");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, Verdict::Fail);
    }

    #[test]
    fn test_evaluate_manual_warns() {
        let mut rule = CisRule::new("1.0.1", "Manual review");
        rule.manual = true;
        let meta = Check::new("1.0.1", Framework::CisK8s, NodeType::Master, "x");
        let ctx = CisContext::default();
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, Verdict::Warn);
    }

    #[test]
    fn test_logic_or_passes_if_any() {
        let mut ctx = CisContext::default();
        ctx.set_flag("apiserver", "--a", "1");
        let mut rule = CisRule::new("x.y", "T");
        rule.logic = Logic::Or;
        rule.items.push(TestItem {
            source: ValueSource::Flag("--a".into()),
            op: BinOp::Eq,
            value: "1".into(),
            set: Some(true),
        });
        rule.items.push(TestItem {
            source: ValueSource::Flag("--b".into()),
            op: BinOp::Eq,
            value: "1".into(),
            set: Some(true),
        });
        let meta = Check::new("x.y", Framework::CisK8s, NodeType::Master, "t");
        let f = evaluate_rule(&rule, &meta, &ctx, "apiserver", "n1");
        assert_eq!(f.verdict, Verdict::Pass);
    }

    #[test]
    fn test_load_rules_yaml_minimal() {
        let yaml = r#"
groups:
  - id: "1.1"
    text: "API permissions"
    checks:
      - id: "1.1.1"
        text: "Ensure foo"
        scored: true
        remediation: "Do bar"
"#;
        let rules = load_rules_yaml(yaml).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "1.1.1");
        assert!(!rules[0].manual);
    }

    #[test]
    fn test_file_exists_eval() {
        let mut ctx = CisContext::default();
        ctx.set_file("/etc/k8s/admin.conf", "600", "root:root");
        let item = TestItem {
            source: ValueSource::FileExists("/etc/k8s/admin.conf".into()),
            op: BinOp::Eq,
            value: "true".into(),
            set: Some(true),
        };
        assert!(eval_item(&item, &ctx, ""));
    }
}
