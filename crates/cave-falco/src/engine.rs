// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Rule engine — evaluates compiled rules against a `FalcoEvent`.
//!
//! NOTICE: upstream is falcosecurity/falco/userspace/engine/falco_engine.cpp.
//! This Rust engine implements a *subset* of the libsinsp filter
//! expression syntax sufficient for the rules shipped by falco-rules.
//! Full grammar support is out of scope (see scope_cuts in the manifest).
//!
//! Supported operators:
//!   `=`, `!=`, `in (a,b,c)`, `contains`, `startswith`, `endswith`,
//!   `and`, `or`, `not`, parens.

use crate::error::{FalcoError, Result};
use crate::event::FalcoEvent;
use crate::rule::{ListDef, MacroDef, Rule};

#[derive(Debug, Clone)]
pub struct EngineMatch {
    pub rule_name: String,
    pub priority: crate::event::Priority,
    pub output_template: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Engine {
    rules: Vec<Rule>,
    macros: Vec<MacroDef>,
    lists: Vec<ListDef>,
}

impl Engine {
    pub fn new() -> Self { Self::default() }

    pub fn load_pack(&mut self, pack: crate::rule_loader::RulePack) -> Result<()> {
        for r in &pack.rules {
            if r.condition.trim().is_empty() {
                return Err(FalcoError::Compile(format!("rule '{}' has empty condition", r.name)));
            }
        }
        self.rules.extend(pack.rules);
        self.macros.extend(pack.macros);
        self.lists.extend(pack.lists);
        Ok(())
    }

    pub fn rule_count(&self) -> usize { self.rules.len() }
    pub fn macro_count(&self) -> usize { self.macros.len() }
    pub fn list_count(&self) -> usize { self.lists.len() }

    /// Evaluate all enabled rules against an event; return a list of
    /// matches (one per rule that fires).
    pub fn evaluate(&self, event: &FalcoEvent) -> Vec<EngineMatch> {
        let mut out = Vec::new();
        for r in &self.rules {
            if !r.enabled { continue; }
            // Source must match (default: syscall).
            if r.source != event.source { continue; }
            if eval_expr(&r.condition, event, &self.macros, &self.lists) {
                out.push(EngineMatch {
                    rule_name: r.name.clone(),
                    priority: r.priority,
                    output_template: r.output.clone(),
                    tags: r.tags.clone(),
                });
            }
        }
        out
    }
}

// ── expression eval (subset) ────────────────────────────────────────────────

fn eval_expr(expr: &str, ev: &FalcoEvent, macros: &[MacroDef], lists: &[ListDef]) -> bool {
    let expanded = expand_macros(expr, macros, 8);
    eval_or(&expanded, ev, lists)
}

fn expand_macros(expr: &str, macros: &[MacroDef], depth: u8) -> String {
    if depth == 0 { return expr.to_string(); }
    let mut out = expr.to_string();
    let mut changed = true;
    let mut steps = 0;
    while changed && steps < depth {
        changed = false;
        for m in macros {
            // Match macro name as a whole token (preceded/followed by
            // whitespace, paren, or string boundary).
            let needle_word = format!(" {} ", m.name);
            if out.contains(&needle_word) {
                out = out.replace(&needle_word, &format!(" ({}) ", m.condition));
                changed = true;
            }
            // Also tokens at start/end.
            if let Some(stripped) = out.strip_prefix(&format!("{} ", m.name)) {
                out = format!("({}) {}", m.condition, stripped);
                changed = true;
            }
            if let Some(stripped) = out.strip_suffix(&format!(" {}", m.name)) {
                out = format!("{} ({})", stripped, m.condition);
                changed = true;
            }
        }
        steps += 1;
    }
    out
}

/// Split on top-level ` or ` boundaries (respecting parens).
fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let sep_bytes = sep.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '(' { depth += 1; }
        else if c == ')' { depth -= 1; }
        else if depth == 0 && i + sep_bytes.len() <= bytes.len() && &bytes[i..i + sep_bytes.len()] == sep_bytes {
            parts.push(&s[start..i]);
            start = i + sep_bytes.len();
            i += sep_bytes.len();
            continue;
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

fn eval_or(expr: &str, ev: &FalcoEvent, lists: &[ListDef]) -> bool {
    split_top(expr, " or ").iter().any(|p| eval_and(p.trim(), ev, lists))
}

fn eval_and(expr: &str, ev: &FalcoEvent, lists: &[ListDef]) -> bool {
    split_top(expr, " and ").iter().all(|p| eval_atom(p.trim(), ev, lists))
}

fn eval_atom(expr: &str, ev: &FalcoEvent, lists: &[ListDef]) -> bool {
    let e = expr.trim();
    if e.starts_with('(') && e.ends_with(')') {
        return eval_or(&e[1..e.len()-1], ev, lists);
    }
    if let Some(rest) = e.strip_prefix("not ") {
        return !eval_atom(rest, ev, lists);
    }
    if e == "true" || e == "1=1" { return true; }
    if e == "false" { return false; }
    if let Some((field, rest)) = e.split_once(" in ") {
        let key = field.trim();
        let actual = ev.fields.get(key).map(|s| s.as_str()).unwrap_or("");
        let inner = rest.trim().trim_start_matches('(').trim_end_matches(')');
        // List reference (`in (shell_binaries)`) or literal list.
        let items: Vec<String> = if let Some(l) = lists.iter().find(|l| l.name == inner) {
            l.items.clone()
        } else {
            inner.split(',').map(|s| s.trim().trim_matches('\'').trim_matches('"').to_string()).collect()
        };
        return items.iter().any(|i| i == actual);
    }
    if let Some((field, val)) = e.split_once(" contains ") {
        let actual = ev.fields.get(field.trim()).map(|s| s.as_str()).unwrap_or("");
        return actual.contains(val.trim().trim_matches('\'').trim_matches('"'));
    }
    if let Some((field, val)) = e.split_once(" startswith ") {
        let actual = ev.fields.get(field.trim()).map(|s| s.as_str()).unwrap_or("");
        return actual.starts_with(val.trim().trim_matches('\'').trim_matches('"'));
    }
    if let Some((field, val)) = e.split_once(" endswith ") {
        let actual = ev.fields.get(field.trim()).map(|s| s.as_str()).unwrap_or("");
        return actual.ends_with(val.trim().trim_matches('\'').trim_matches('"'));
    }
    if let Some((field, val)) = e.split_once("!=") {
        let actual = ev.fields.get(field.trim()).map(|s| s.as_str()).unwrap_or("");
        return actual != val.trim().trim_matches('\'').trim_matches('"');
    }
    if let Some((field, val)) = e.split_once('=') {
        let actual = ev.fields.get(field.trim()).map(|s| s.as_str()).unwrap_or("");
        return actual == val.trim().trim_matches('\'').trim_matches('"');
    }
    // Bare field reference treated as "field exists".
    ev.fields.contains_key(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{FalcoEvent, Priority};
    use crate::rule::Rule;
    use crate::rule_loader::RulePack;

    fn rule(name: &str, cond: &str) -> Rule {
        Rule {
            name: name.into(),
            desc: name.into(),
            condition: cond.into(),
            output: format!("matched {name}"),
            priority: Priority::Warning,
            source: "syscall".into(),
            tags: vec![],
            enabled: true,
            exceptions: vec![],
        }
    }

    fn pack(rs: Vec<Rule>) -> RulePack {
        RulePack { rules: rs, macros: vec![], lists: vec![] }
    }

    #[test]
    fn rejects_empty_condition_on_load() {
        let mut e = Engine::new();
        let mut r = rule("x", "1=1"); r.condition.clear();
        let res = e.load_pack(pack(vec![r]));
        assert!(res.is_err());
    }

    #[test]
    fn eq_match_fires() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r1", "proc.name=bash")])).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "bash");
        assert_eq!(e.evaluate(&ev).len(), 1);
    }

    #[test]
    fn ne_match_does_not_fire_for_match() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r1", "proc.name!=bash")])).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "bash");
        assert!(e.evaluate(&ev).is_empty());
    }

    #[test]
    fn and_requires_all_clauses_true() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r", "evt.type=execve and proc.name=bash")])).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "bash");
        assert_eq!(e.evaluate(&ev).len(), 1);
        let ev2 = FalcoEvent::syscall("execve").with("proc.name", "sh");
        assert!(e.evaluate(&ev2).is_empty());
    }

    #[test]
    fn or_requires_any_clause_true() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r", "proc.name=bash or proc.name=sh")])).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "sh");
        assert_eq!(e.evaluate(&ev).len(), 1);
    }

    #[test]
    fn in_literal_list_matches() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r", "proc.name in (bash,sh,zsh)")])).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "zsh");
        assert_eq!(e.evaluate(&ev).len(), 1);
    }

    #[test]
    fn in_named_list_resolves_via_lists_section() {
        let mut e = Engine::new();
        let mut p = pack(vec![rule("r", "proc.name in (shell_binaries)")]);
        p.lists.push(ListDef { name: "shell_binaries".into(), items: vec!["bash".into(), "fish".into()] });
        e.load_pack(p).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "fish");
        assert_eq!(e.evaluate(&ev).len(), 1);
    }

    #[test]
    fn contains_startswith_endswith() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![
            rule("c", "fd.name contains '/etc/'"),
            rule("s", "fd.name startswith '/etc/'"),
            rule("z", "fd.name endswith '/passwd'"),
        ])).unwrap();
        let ev = FalcoEvent::syscall("openat").with("fd.name", "/etc/passwd");
        let m = e.evaluate(&ev);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn not_operator_inverts() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r", "not proc.name=bash")])).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "sh");
        assert_eq!(e.evaluate(&ev).len(), 1);
    }

    #[test]
    fn disabled_rule_skipped() {
        let mut e = Engine::new();
        let mut r = rule("r", "1=1"); r.enabled = false;
        e.load_pack(pack(vec![r])).unwrap();
        let ev = FalcoEvent::syscall("execve");
        assert!(e.evaluate(&ev).is_empty());
    }

    #[test]
    fn source_filter_skips_wrong_source() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r", "1=1")])).unwrap();
        let ev = FalcoEvent::k8s_audit("ResponseComplete", "create");
        assert!(e.evaluate(&ev).is_empty());
    }

    #[test]
    fn macros_expand_inside_condition() {
        let mut e = Engine::new();
        let mut p = pack(vec![rule("r", "spawned and proc.name=bash")]);
        p.macros.push(MacroDef { name: "spawned".into(), condition: "evt.type=execve".into() });
        e.load_pack(p).unwrap();
        let ev = FalcoEvent::syscall("execve").with("proc.name", "bash");
        assert_eq!(e.evaluate(&ev).len(), 1);
    }

    #[test]
    fn engine_counts_pack_sizes() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("a", "1=1"), rule("b", "1=1")])).unwrap();
        assert_eq!(e.rule_count(), 2);
    }
}
