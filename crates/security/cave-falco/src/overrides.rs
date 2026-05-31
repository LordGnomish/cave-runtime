// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Rule append / override semantics.
//!
//! NOTICE: upstream is falcosecurity/falco
//! `userspace/engine/rule_loader_collector.cpp` (Apache-2.0), the
//! `collector::append` / `collector::selective_replace` paths. A Falco rule
//! file may re-declare an existing rule with `append: true` (legacy) or an
//! `override:` block (`override.condition: append|replace`, etc.) to extend
//! or replace fields of a previously loaded rule.
//!
//! Append rules (1:1 with upstream):
//!   - `condition` / `output` / `desc` : space-joined onto the previous value
//!   - `tags`                          : set-union (no duplicates)
//!   - `exceptions`                    : per-name — a new name is pushed
//!     (requires `fields` + `values`); an existing name may append **values
//!     only** (specifying `fields` or `comps` is an error).
//!
//! Replace rules overwrite the named field wholesale.

use crate::error::{FalcoError, Result};
use crate::rule::{Exception, Rule};

/// The fields an append/replace entry may carry. `None` means "not present
/// in this update" (so untouched); for `tags`/`exceptions` an empty Vec is
/// treated as "not present".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RuleUpdate {
    pub name: String,
    pub condition: Option<String>,
    pub output: Option<String>,
    pub desc: Option<String>,
    pub tags: Vec<String>,
    pub exceptions: Vec<Exception>,
}

impl RuleUpdate {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }
    pub fn with_condition(mut self, c: impl Into<String>) -> Self {
        self.condition = Some(c.into());
        self
    }
}

/// `collector::append` — merge `info` into `prev`.
pub fn append(prev: &mut Rule, info: &RuleUpdate) -> Result<()> {
    if let Some(c) = info.condition.as_ref().filter(|c| !c.is_empty()) {
        prev.condition.push(' ');
        prev.condition.push_str(c);
    }
    if let Some(o) = info.output.as_ref().filter(|o| !o.is_empty()) {
        prev.output.push(' ');
        prev.output.push_str(o);
    }
    if let Some(d) = info.desc.as_ref().filter(|d| !d.is_empty()) {
        prev.desc.push(' ');
        prev.desc.push_str(d);
    }
    for tag in &info.tags {
        if !prev.tags.contains(tag) {
            prev.tags.push(tag.clone());
        }
    }
    for ex in &info.exceptions {
        match prev.exceptions.iter_mut().find(|e| e.name == ex.name) {
            None => {
                if ex.fields.is_empty() {
                    return Err(FalcoError::RuleParse(format!(
                        "rule exception '{}' must have a fields property", ex.name
                    )));
                }
                if ex.values.is_empty() {
                    return Err(FalcoError::RuleParse(format!(
                        "rule exception '{}' must have a values property", ex.name
                    )));
                }
                prev.exceptions.push(ex.clone());
            }
            Some(prev_ex) => {
                if !ex.fields.is_empty() {
                    return Err(FalcoError::RuleParse(format!(
                        "can not append exception fields to existing exception '{}', only values",
                        ex.name
                    )));
                }
                prev_ex.values.extend(ex.values.iter().cloned());
            }
        }
    }
    Ok(())
}

/// `collector::selective_replace` — overwrite the named fields of `prev`.
pub fn replace(prev: &mut Rule, info: &RuleUpdate) -> Result<()> {
    if let Some(c) = &info.condition {
        prev.condition = c.clone();
    }
    if let Some(o) = &info.output {
        prev.output = o.clone();
    }
    if let Some(d) = &info.desc {
        prev.desc = d.clone();
    }
    if !info.tags.is_empty() {
        prev.tags = info.tags.clone();
    }
    if !info.exceptions.is_empty() {
        prev.exceptions = info.exceptions.clone();
    }
    Ok(())
}

/// Locate the rule named `info.name` in `rules` and apply `append`. Errors
/// (`ERROR_NO_PREVIOUS_RULE_APPEND`) if no such rule exists.
pub fn append_in(rules: &mut [Rule], info: &RuleUpdate) -> Result<()> {
    let prev = rules
        .iter_mut()
        .find(|r| r.name == info.name)
        .ok_or_else(|| FalcoError::RuleParse(format!(
            "appended rule '{}' has no previous definition", info.name
        )))?;
    append(prev, info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Priority;

    fn base() -> Rule {
        Rule {
            name: "Write below etc".into(),
            desc: "write under /etc".into(),
            condition: "evt.type=open and fd.name startswith /etc".into(),
            output: "file=%fd.name".into(),
            priority: Priority::Warning,
            source: "syscall".into(),
            tags: vec!["filesystem".into()],
            enabled: true,
            exceptions: vec![],
        }
    }

    fn ex(name: &str, fields: &[&str], values: Vec<Vec<&str>>) -> Exception {
        Exception {
            name: name.into(),
            fields: fields.iter().map(|s| s.to_string()).collect(),
            values: values.into_iter().map(|v| v.into_iter().map(|s| s.to_string()).collect()).collect(),
            comps: vec![],
        }
    }

    #[test]
    fn append_condition_space_joins() {
        let mut r = base();
        append(&mut r, &RuleUpdate::new(&r.name.clone()).with_condition("and not proc.name=foo")).unwrap();
        assert_eq!(r.condition, "evt.type=open and fd.name startswith /etc and not proc.name=foo");
    }

    #[test]
    fn append_tags_unions_without_dups() {
        let mut r = base();
        let info = RuleUpdate { name: r.name.clone(), tags: vec!["filesystem".into(), "mitre".into()], ..Default::default() };
        append(&mut r, &info).unwrap();
        assert_eq!(r.tags, vec!["filesystem".to_string(), "mitre".to_string()]);
    }

    #[test]
    fn append_output_and_desc_space_join() {
        let mut r = base();
        let info = RuleUpdate { name: r.name.clone(), output: Some("user=%user.name".into()), desc: Some("extra".into()), ..Default::default() };
        append(&mut r, &info).unwrap();
        assert_eq!(r.output, "file=%fd.name user=%user.name");
        assert_eq!(r.desc, "write under /etc extra");
    }

    #[test]
    fn append_new_exception_name_pushes() {
        let mut r = base();
        let info = RuleUpdate { name: r.name.clone(), exceptions: vec![ex("proc", &["proc.name"], vec![vec!["sshd"]])], ..Default::default() };
        append(&mut r, &info).unwrap();
        assert_eq!(r.exceptions.len(), 1);
        assert_eq!(r.exceptions[0].name, "proc");
    }

    #[test]
    fn append_existing_exception_appends_values_only() {
        let mut r = base();
        r.exceptions.push(ex("proc", &["proc.name"], vec![vec!["sshd"]]));
        // values-only update (no fields)
        let info = RuleUpdate { name: r.name.clone(), exceptions: vec![ex("proc", &[], vec![vec!["nginx"]])], ..Default::default() };
        append(&mut r, &info).unwrap();
        assert_eq!(r.exceptions.len(), 1);
        assert_eq!(r.exceptions[0].values.len(), 2);
    }

    #[test]
    fn append_existing_exception_with_fields_errors() {
        let mut r = base();
        r.exceptions.push(ex("proc", &["proc.name"], vec![vec!["sshd"]]));
        let info = RuleUpdate { name: r.name.clone(), exceptions: vec![ex("proc", &["proc.name"], vec![vec!["nginx"]])], ..Default::default() };
        assert!(append(&mut r, &info).is_err());
    }

    #[test]
    fn append_new_exception_without_values_errors() {
        let mut r = base();
        let info = RuleUpdate { name: r.name.clone(), exceptions: vec![ex("proc", &["proc.name"], vec![])], ..Default::default() };
        assert!(append(&mut r, &info).is_err());
    }

    #[test]
    fn replace_condition_overwrites() {
        let mut r = base();
        replace(&mut r, &RuleUpdate::new(&r.name.clone()).with_condition("evt.type=connect")).unwrap();
        assert_eq!(r.condition, "evt.type=connect");
    }

    #[test]
    fn replace_exceptions_overwrites_list() {
        let mut r = base();
        r.exceptions.push(ex("old", &["proc.name"], vec![vec!["x"]]));
        let info = RuleUpdate { name: r.name.clone(), exceptions: vec![ex("new", &["fd.name"], vec![vec!["/tmp"]])], ..Default::default() };
        replace(&mut r, &info).unwrap();
        assert_eq!(r.exceptions.len(), 1);
        assert_eq!(r.exceptions[0].name, "new");
    }

    #[test]
    fn append_in_missing_rule_errors() {
        let mut rules = vec![base()];
        let res = append_in(&mut rules, &RuleUpdate::new("Nonexistent").with_condition("and x=1"));
        assert!(res.is_err());
    }

    #[test]
    fn append_in_finds_and_merges() {
        let mut rules = vec![base()];
        append_in(&mut rules, &RuleUpdate::new("Write below etc").with_condition("or fd.name startswith /usr")).unwrap();
        assert!(rules[0].condition.ends_with("or fd.name startswith /usr"));
    }
}
