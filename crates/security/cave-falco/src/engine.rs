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
use std::cmp::Ordering;
use std::net::IpAddr;

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

    /// Falco `-T <tags>`: disable every rule carrying at least one of the
    /// given tags (`falco_engine::enable_rule_by_tag(tags, false)`).
    /// Additive — repeated calls accumulate. Returns the number of rules
    /// newly disabled by this call.
    pub fn disable_by_tags(&mut self, tags: &[&str]) -> usize {
        let mut n = 0;
        for r in &mut self.rules {
            if r.enabled && rule_has_any_tag(r, tags) {
                r.enabled = false;
                n += 1;
            }
        }
        n
    }

    /// Falco `-t <tags>`: run **only** rules carrying at least one of the
    /// given tags; every other rule is disabled. Returns the number of
    /// rules kept enabled.
    pub fn run_only_tags(&mut self, tags: &[&str]) -> usize {
        let mut kept = 0;
        for r in &mut self.rules {
            if rule_has_any_tag(r, tags) {
                r.enabled = true;
                kept += 1;
            } else {
                r.enabled = false;
            }
        }
        kept
    }

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

/// True if `r` carries at least one of `tags` (Falco tag intersection).
fn rule_has_any_tag(r: &Rule, tags: &[&str]) -> bool {
    r.tags.iter().any(|t| tags.contains(&t.as_str()))
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

    // Unary: `field exists` (libsinsp s_unary_ops).
    if let Some(field) = e.strip_suffix(" exists") {
        return ev.fields.contains_key(field.trim());
    }

    let field_val = |field: &str| ev.fields.get(field.trim()).map(|s| s.as_str()).unwrap_or("");

    // List operators (s_binary_list_ops): in, intersects, pmatch.
    if let Some((field, rest)) = e.split_once(" in ") {
        let actual = field_val(field);
        return resolve_list(rest, lists).iter().any(|i| matches_value(actual, i));
    }
    if let Some((field, rest)) = e.split_once(" intersects ") {
        let actual_set: Vec<&str> = field_val(field).split(',').map(|s| s.trim()).collect();
        let items = resolve_list(rest, lists);
        return items.iter().any(|i| actual_set.contains(&i.as_str()));
    }
    if let Some((field, rest)) = e.split_once(" pmatch ") {
        let actual = field_val(field);
        return resolve_list(rest, lists).iter().any(|p| path_prefix_match(p, actual));
    }

    // String operators (s_binary_str_ops). `icontains`/`iglob` checked before
    // their case-sensitive counterparts.
    if let Some((field, val)) = e.split_once(" icontains ") {
        return field_val(field).to_lowercase().contains(&unquote(val).to_lowercase());
    }
    if let Some((field, val)) = e.split_once(" contains ") {
        return field_val(field).contains(unquote(val));
    }
    if let Some((field, val)) = e.split_once(" iglob ") {
        return glob_match(unquote(val).to_lowercase().as_bytes(), field_val(field).to_lowercase().as_bytes());
    }
    if let Some((field, val)) = e.split_once(" glob ") {
        return glob_match(unquote(val).as_bytes(), field_val(field).as_bytes());
    }
    if let Some((field, val)) = e.split_once(" regex ") {
        return regex::Regex::new(unquote(val)).map(|re| re.is_match(field_val(field))).unwrap_or(false);
    }
    if let Some((field, val)) = e.split_once(" startswith ") {
        return field_val(field).starts_with(unquote(val));
    }
    if let Some((field, val)) = e.split_once(" endswith ") {
        return field_val(field).ends_with(unquote(val));
    }

    // Numeric comparisons (s_binary_num_ops). Two-char ops first.
    for (op, cmp) in [(" >= ", Ordering::Greater), (" <= ", Ordering::Less)] {
        if let Some((field, val)) = e.split_once(op) {
            return numeric_cmp(field_val(field), unquote(val))
                .map(|o| o == cmp || o == Ordering::Equal).unwrap_or(false);
        }
    }
    for (op, cmp) in [(" > ", Ordering::Greater), (" < ", Ordering::Less)] {
        if let Some((field, val)) = e.split_once(op) {
            return numeric_cmp(field_val(field), unquote(val)).map(|o| o == cmp).unwrap_or(false);
        }
    }

    // Equality (`!=`, `==`, `=`) with CIDR-aware comparison.
    if let Some((field, val)) = e.split_once("!=") {
        return !matches_value(field_val(field), unquote(val));
    }
    if let Some((field, val)) = e.split_once("==") {
        return matches_value(field_val(field), unquote(val));
    }
    if let Some((field, val)) = e.split_once('=') {
        return matches_value(field_val(field), unquote(val));
    }
    // Bare field reference treated as "field exists".
    ev.fields.contains_key(e)
}

/// Strip surrounding quotes/whitespace from an RHS literal.
fn unquote(s: &str) -> &str {
    s.trim().trim_matches('\'').trim_matches('"')
}

/// Resolve `(name)` / `(a, b, c)` into the literal item set, expanding a
/// named list reference if one matches.
fn resolve_list(rest: &str, lists: &[ListDef]) -> Vec<String> {
    let inner = rest.trim().trim_start_matches('(').trim_end_matches(')');
    if let Some(l) = lists.iter().find(|l| l.name == inner.trim()) {
        return l.items.clone();
    }
    inner.split(',').map(|s| unquote(s).to_string()).filter(|s| !s.is_empty()).collect()
}

/// Equality with `net_compare` semantics: when the RHS is a CIDR and the LHS
/// is an IP, test containment; otherwise plain string equality.
fn matches_value(actual: &str, val: &str) -> bool {
    if val.contains('/') {
        if let Some(hit) = cidr_contains(val, actual) {
            return hit;
        }
    }
    actual == val
}

/// `pmatch`: true if `path` equals `prefix` or sits beneath it.
fn path_prefix_match(prefix: &str, path: &str) -> bool {
    let prefix = unquote(prefix);
    path == prefix || path.starts_with(&format!("{}/", prefix.trim_end_matches('/')))
}

/// Parse both operands as f64 and order them.
fn numeric_cmp(a: &str, b: &str) -> Option<Ordering> {
    let a: f64 = a.trim().parse().ok()?;
    let b: f64 = b.trim().parse().ok()?;
    a.partial_cmp(&b)
}

/// CIDR containment for v4/v6. `None` if `cidr` is not a valid CIDR or `ip`
/// is not a valid address (caller falls back to string equality).
fn cidr_contains(cidr: &str, ip: &str) -> Option<bool> {
    let (net, prefix) = cidr.split_once('/')?;
    let prefix: u32 = prefix.parse().ok()?;
    let net: IpAddr = net.parse().ok()?;
    let ip: IpAddr = ip.parse().ok()?;
    match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(a)) => {
            if prefix > 32 { return Some(false); }
            let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
            Some(u32::from(n) & mask == u32::from(a) & mask)
        }
        (IpAddr::V6(n), IpAddr::V6(a)) => {
            if prefix > 128 { return Some(false); }
            let mask = if prefix == 0 { 0 } else { u128::MAX << (128 - prefix) };
            Some(u128::from(n) & mask == u128::from(a) & mask)
        }
        _ => Some(false),
    }
}

/// Classic wildcard glob (`*` any-run, `?` single-char), iterative backtrack.
fn glob_match(pat: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while t < text.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(sp) = star {
            p = sp + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
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

    fn tagged(name: &str, tags: &[&str]) -> Rule {
        let mut r = rule(name, "1=1");
        r.tags = tags.iter().map(|s| s.to_string()).collect();
        r
    }

    #[test]
    fn disable_by_tags_turns_off_matching_rules() {
        // Falco `-T network`: any rule tagged `network` is disabled.
        let mut e = Engine::new();
        e.load_pack(pack(vec![
            tagged("a", &["network", "container"]),
            tagged("b", &["filesystem"]),
        ])).unwrap();
        let n = e.disable_by_tags(&["network"]);
        assert_eq!(n, 1);
        let ev = FalcoEvent::syscall("execve");
        let fired: Vec<_> = e.evaluate(&ev).into_iter().map(|m| m.rule_name).collect();
        assert_eq!(fired, vec!["b".to_string()]);
    }

    #[test]
    fn run_only_tags_disables_everything_else() {
        // Falco `-t container`: only rules tagged `container` run.
        let mut e = Engine::new();
        e.load_pack(pack(vec![
            tagged("a", &["network"]),
            tagged("b", &["container"]),
            tagged("c", &[]),
        ])).unwrap();
        let kept = e.run_only_tags(&["container"]);
        assert_eq!(kept, 1);
        let ev = FalcoEvent::syscall("execve");
        let fired: Vec<_> = e.evaluate(&ev).into_iter().map(|m| m.rule_name).collect();
        assert_eq!(fired, vec!["b".to_string()]);
    }

    #[test]
    fn run_only_tags_matches_any_of_listed() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![
            tagged("a", &["network"]),
            tagged("b", &["container"]),
            tagged("c", &["mitre_execution"]),
        ])).unwrap();
        let kept = e.run_only_tags(&["network", "mitre_execution"]);
        assert_eq!(kept, 2);
        let ev = FalcoEvent::syscall("execve");
        let mut fired: Vec<_> = e.evaluate(&ev).into_iter().map(|m| m.rule_name).collect();
        fired.sort();
        assert_eq!(fired, vec!["a".to_string(), "c".to_string()]);
    }

    #[test]
    fn disable_by_tags_is_additive_across_calls() {
        let mut e = Engine::new();
        e.load_pack(pack(vec![
            tagged("a", &["network"]),
            tagged("b", &["filesystem"]),
            tagged("c", &["process"]),
        ])).unwrap();
        e.disable_by_tags(&["network"]);
        e.disable_by_tags(&["filesystem"]);
        let ev = FalcoEvent::syscall("execve");
        let fired: Vec<_> = e.evaluate(&ev).into_iter().map(|m| m.rule_name).collect();
        assert_eq!(fired, vec!["c".to_string()]);
    }

    // ── full filter grammar (libsinsp operator set) ────────────────────────

    fn fires(cond: &str, ev: &FalcoEvent) -> bool {
        let mut e = Engine::new();
        e.load_pack(pack(vec![rule("r", cond)])).unwrap();
        !e.evaluate(ev).is_empty()
    }

    #[test]
    fn double_equals_is_alias_for_eq() {
        let ev = FalcoEvent::syscall("execve").with("proc.name", "bash");
        assert!(fires("proc.name == bash", &ev));
        assert!(!fires("proc.name == sh", &ev));
    }

    #[test]
    fn numeric_comparisons() {
        let ev = FalcoEvent::syscall("open").with("fd.num", "1024");
        assert!(fires("fd.num > 1000", &ev));
        assert!(fires("fd.num >= 1024", &ev));
        assert!(fires("fd.num < 2000", &ev));
        assert!(fires("fd.num <= 1024", &ev));
        assert!(!fires("fd.num > 1024", &ev));
        assert!(!fires("fd.num < 1024", &ev));
    }

    #[test]
    fn icontains_is_case_insensitive() {
        let ev = FalcoEvent::syscall("open").with("fd.name", "/etc/PASSWD");
        assert!(fires("fd.name icontains passwd", &ev));
        assert!(!fires("fd.name contains passwd", &ev));
    }

    #[test]
    fn glob_matches_wildcards() {
        let ev = FalcoEvent::syscall("open").with("fd.name", "/var/log/syslog");
        assert!(fires("fd.name glob '/var/log/*'", &ev));
        assert!(fires("fd.name glob '/var/*/sys???'", &ev));
        assert!(!fires("fd.name glob '/etc/*'", &ev));
    }

    #[test]
    fn regex_matches_pattern() {
        let ev = FalcoEvent::syscall("execve").with("proc.name", "python3.11");
        assert!(fires("proc.name regex 'python[0-9.]+'", &ev));
        assert!(!fires("proc.name regex '^java'", &ev));
    }

    #[test]
    fn explicit_exists_unary() {
        let ev = FalcoEvent::syscall("execve").with("container.id", "abc");
        assert!(fires("container.id exists", &ev));
        assert!(!fires("container.image exists", &ev));
    }

    #[test]
    fn pmatch_path_prefix_set() {
        let ev = FalcoEvent::syscall("open").with("fd.name", "/etc/shadow");
        assert!(fires("fd.name pmatch (/etc, /usr)", &ev));
        assert!(!fires("fd.name pmatch (/var, /tmp)", &ev));
        // exact path also matches
        let ev2 = FalcoEvent::syscall("open").with("fd.name", "/etc");
        assert!(fires("fd.name pmatch (/etc)", &ev2));
    }

    #[test]
    fn intersects_multi_value_field() {
        let ev = FalcoEvent::syscall("execve").with("proc.aname", "bash,sudo,init");
        assert!(fires("proc.aname intersects (sudo, sshd)", &ev));
        assert!(!fires("proc.aname intersects (nginx, sshd)", &ev));
    }

    #[test]
    fn cidr_containment_via_eq_and_in() {
        let ev = FalcoEvent::syscall("connect").with("fd.cip", "10.1.2.3");
        assert!(fires("fd.cip = 10.0.0.0/8", &ev));
        assert!(!fires("fd.cip = 192.168.0.0/16", &ev));
        assert!(fires("fd.cip in (192.168.0.0/16, 10.0.0.0/8)", &ev));
    }

    #[test]
    fn cidr_ipv6_containment() {
        let ev = FalcoEvent::syscall("connect").with("fd.cip", "2001:db8::1");
        assert!(fires("fd.cip = 2001:db8::/32", &ev));
        assert!(!fires("fd.cip = 2001:dead::/32", &ev));
    }
}
