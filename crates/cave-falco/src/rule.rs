// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Falco rule DSL model.
//!
//! NOTICE: upstream is falcosecurity/falco (Apache-2.0). The rule
//! grammar mirrors `userspace/engine/rule_loader.cpp` and the
//! `userspace/engine/falco_rules.cpp` AST.

use crate::event::Priority;
use serde::{Deserialize, Serialize};

/// Action a rule fires on match. Falco's `output` text + tags + extra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleAction {
    pub output: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub extra: Vec<OutputField>,
}

/// One `output` field as in `proc.name=%proc.name container.id=%container.id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputField {
    pub key: String,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    /// Free-text description.
    #[serde(default)]
    pub desc: String,
    /// libsinsp filter expression — modeled as a string; engine
    /// evaluates against a `FalcoEvent`.
    pub condition: String,
    /// `output` text — typically references `%proc.name` etc.
    #[serde(default)]
    pub output: String,
    pub priority: Priority,
    /// Optional `source` (defaults to "syscall").
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// `enabled: false` disables the rule without removing it.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `exceptions` block — list of exception entries; each gates the
    /// rule from firing on a specific field-value match.
    #[serde(default)]
    pub exceptions: Vec<Exception>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exception {
    pub name: String,
    pub fields: Vec<String>,
    #[serde(default)]
    pub values: Vec<Vec<String>>,
    #[serde(default = "default_comp")]
    pub comps: Vec<String>,
}

/// A macro is a reusable named condition fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroDef {
    pub name: String,
    pub condition: String,
}

/// A list is a named collection of literal values referenced inside
/// conditions (e.g. `proc.name in (allowed_procs)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListDef {
    pub name: String,
    pub items: Vec<String>,
}

fn default_source() -> String { "syscall".into() }
fn default_true() -> bool { true }
fn default_comp() -> Vec<String> { vec!["=".into()] }

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rule() -> Rule {
        Rule {
            name: "Suspicious shell".into(),
            desc: "A shell ran in a container".into(),
            condition: "spawned_process and container and proc.name in (shell_binaries)".into(),
            output: "Shell in container %container.id".into(),
            priority: Priority::Warning,
            source: "syscall".into(),
            tags: vec!["container".into(), "shell".into()],
            enabled: true,
            exceptions: vec![],
        }
    }

    #[test]
    fn rule_serde_round_trip() {
        let r = sample_rule();
        let j = serde_json::to_string(&r).unwrap();
        let r2: Rule = serde_json::from_str(&j).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn rule_default_source_is_syscall() {
        let y = "name: t\ndesc: t\ncondition: 1=1\npriority: WARNING\noutput: t\n";
        let r: Rule = serde_yaml::from_str(y).unwrap();
        assert_eq!(r.source, "syscall");
    }

    #[test]
    fn rule_default_enabled_true() {
        let y = "name: t\ndesc: t\ncondition: 1=1\npriority: WARNING\noutput: t\n";
        let r: Rule = serde_yaml::from_str(y).unwrap();
        assert!(r.enabled);
    }

    #[test]
    fn macro_def_holds_condition_fragment() {
        let m = MacroDef { name: "container".into(), condition: "container.id != host".into() };
        assert_eq!(m.condition, "container.id != host");
    }

    #[test]
    fn list_def_holds_named_set() {
        let l = ListDef { name: "shell_binaries".into(), items: vec!["bash".into(), "sh".into()] };
        assert_eq!(l.items.len(), 2);
    }

    #[test]
    fn exception_default_comp_is_eq() {
        let y = "name: drop_proc\nfields: [\"proc.name\"]\nvalues: [[\"sshd\"]]\n";
        let e: Exception = serde_yaml::from_str(y).unwrap();
        assert_eq!(e.comps, vec!["="]);
    }

    #[test]
    fn priority_inside_rule_is_uppercase() {
        let r = sample_rule();
        let j = serde_yaml::to_string(&r).unwrap();
        assert!(j.contains("WARNING"));
    }
}
