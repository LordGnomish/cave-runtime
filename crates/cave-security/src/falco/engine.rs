// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Falco rule engine — evaluates events against the loaded rule set.

use crate::falco::{
    condition::{EvalContext, Expr, eval, parse_condition},
    fields::EventContext,
    rule::{BUILTIN_RULES_YAML, FalcoList, FalcoMacro, FalcoRule, Priority, RuleSet},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Alert
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: Uuid,
    pub rule_name: String,
    pub priority: Priority,
    pub output: String,
    pub source: String,
    pub tags: Vec<String>,
    pub fields: HashMap<String, String>,
    pub timestamp: DateTime<Utc>,
}

impl Alert {
    pub fn new(rule: &FalcoRule, output: String, fields: HashMap<String, String>) -> Self {
        Alert {
            id: Uuid::new_v4(),
            rule_name: rule.name.clone(),
            priority: rule.priority,
            output,
            source: format!("{:?}", rule.source),
            tags: rule.tags.clone(),
            fields,
            timestamp: Utc::now(),
        }
    }
}

// ---------------------------------------------------------------------------
// RuleStore
// ---------------------------------------------------------------------------

/// Compiled rule store — holds parsed rules + pre-compiled condition ASTs.
pub struct RuleStore {
    pub rule_set: RuleSet,
    /// Compiled macro expressions (macro name → AST).
    pub compiled_macros: HashMap<String, Expr>,
    /// Named lists (list name → items).
    pub list_map: HashMap<String, Vec<String>>,
    /// Compiled rule conditions (rule name → AST).
    pub compiled_rules: HashMap<String, Expr>,
}

impl Default for RuleStore {
    fn default() -> Self {
        let rule_set = RuleSet::from_yaml(BUILTIN_RULES_YAML).expect("builtin rules must parse");
        Self::from_ruleset(rule_set)
    }
}

impl RuleStore {
    pub fn from_ruleset(rule_set: RuleSet) -> Self {
        let list_map: HashMap<String, Vec<String>> = rule_set
            .lists
            .iter()
            .map(|l| (l.name.clone(), l.items.clone()))
            .collect();

        // Compile macros first (macros can reference other macros — one pass is
        // sufficient for non-recursive Falco macros).
        let compiled_macros: HashMap<String, Expr> = rule_set
            .macros
            .iter()
            .map(|m| (m.name.clone(), parse_condition(&m.condition)))
            .collect();

        let compiled_rules: HashMap<String, Expr> = rule_set
            .rules
            .iter()
            .filter(|r| r.enabled)
            .map(|r| (r.name.clone(), parse_condition(&r.condition)))
            .collect();

        RuleStore {
            rule_set,
            compiled_macros,
            list_map,
            compiled_rules,
        }
    }

    /// Load additional rules from YAML, merging into the store.
    pub fn load_yaml(&mut self, yaml: &str) -> anyhow::Result<usize> {
        let new_rs = RuleSet::from_yaml(yaml)?;
        let count = new_rs.rules.len() + new_rs.macros.len() + new_rs.lists.len();
        self.merge(new_rs);
        Ok(count)
    }

    fn merge(&mut self, new_rs: RuleSet) {
        for list in &new_rs.lists {
            self.list_map.insert(list.name.clone(), list.items.clone());
        }
        for mac in &new_rs.macros {
            self.compiled_macros
                .insert(mac.name.clone(), parse_condition(&mac.condition));
        }
        for rule in new_rs.rules.iter().filter(|r| r.enabled) {
            self.compiled_rules
                .insert(rule.name.clone(), parse_condition(&rule.condition));
        }
        self.rule_set.merge(new_rs);
    }

    pub fn rules(&self) -> &[FalcoRule] {
        &self.rule_set.rules
    }

    pub fn macros(&self) -> &[FalcoMacro] {
        &self.rule_set.macros
    }

    pub fn lists(&self) -> &[FalcoList] {
        &self.rule_set.lists
    }

    pub fn rule_count(&self) -> usize {
        self.rule_set.rules.len()
    }
}

// ---------------------------------------------------------------------------
// Rule engine
// ---------------------------------------------------------------------------

pub struct RuleEngine<'a> {
    store: &'a RuleStore,
}

impl<'a> RuleEngine<'a> {
    pub fn new(store: &'a RuleStore) -> Self {
        RuleEngine { store }
    }

    /// Evaluate an event context against all enabled rules.
    /// Returns one Alert per matching rule.
    pub fn evaluate(&self, event: &EventContext) -> Vec<Alert> {
        let eval_ctx = EvalContext {
            fields: &event.fields,
            lists: &self.store.list_map,
            macros: &self.store.compiled_macros,
        };

        let mut alerts = Vec::new();
        for rule in self.store.rule_set.rules.iter().filter(|r| r.enabled) {
            if let Some(expr) = self.store.compiled_rules.get(&rule.name) {
                if eval(expr, &eval_ctx) {
                    let output = format_output(&rule.output, &event.fields);
                    alerts.push(Alert::new(rule, output, event.fields.clone()));
                }
            }
        }
        alerts
    }
}

/// Substitute `%field.name` tokens in Falco output templates.
pub fn format_output(template: &str, fields: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    // Replace %field.name with the field value
    for (key, val) in fields {
        let placeholder = format!("%{key}");
        result = result.replace(&placeholder, val);
    }
    // Remove any un-substituted placeholders
    let mut out = String::new();
    let mut chars = result.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            // skip to next space or end
            while chars.peek().map(|&ch| ch != ' ').unwrap_or(false) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_store_has_builtin_rules() {
        let store = RuleStore::default();
        assert!(store.rule_count() > 0);
    }

    #[test]
    fn evaluate_terminal_shell_in_container() {
        let store = RuleStore::default();
        let engine = RuleEngine::new(&store);

        let ctx = EventContext::syscall_execve("bash", "bash -i", "root", 0, "abc123");
        let alerts = engine.evaluate(&ctx);
        // Should fire "Terminal shell in container"
        assert!(
            alerts
                .iter()
                .any(|a| a.rule_name.contains("shell") || a.rule_name.contains("Shell"))
        );
    }

    #[test]
    fn no_alert_for_benign_event() {
        let store = RuleStore::default();
        let engine = RuleEngine::new(&store);
        let mut ctx = EventContext::new();
        ctx.set("evt.type", "read");
        ctx.set("proc.name", "cat");
        ctx.set("user.name", "alice");
        ctx.set("container.id", "host");
        // "cat" reading without /etc or sensitive patterns should not fire
        let alerts = engine.evaluate(&ctx);
        assert!(alerts.iter().all(|a| !a.rule_name.contains("sensitive")));
    }

    #[test]
    fn load_custom_yaml_rules() {
        let mut store = RuleStore::default();
        let yaml = r#"
- rule: Test custom rule
  condition: evt.type = "open" and fd.name = "/tmp/test"
  output: "Test output (file=%fd.name)"
  priority: DEBUG
  enabled: true
"#;
        let before = store.rule_count();
        store.load_yaml(yaml).expect("load yaml");
        assert_eq!(store.rule_count(), before + 1);
    }

    #[test]
    fn format_output_substitution() {
        let mut fields = HashMap::new();
        fields.insert("proc.name".into(), "bash".into());
        fields.insert("user.name".into(), "alice".into());
        let tmpl = "Shell run by %user.name process=%proc.name";
        let out = format_output(tmpl, &fields);
        assert!(out.contains("alice"));
        assert!(out.contains("bash"));
    }
}
