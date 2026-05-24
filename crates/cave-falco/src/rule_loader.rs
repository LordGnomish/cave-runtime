// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! YAML rule-pack loader. A Falco rule file may contain three top-level
//! lists interleaved: `rule:`, `macro:`, `list:`. We accept the standard
//! Falco YAML shape (sequence of single-key documents).
//!
//! NOTICE: upstream is falcosecurity/falco/userspace/engine/rule_loader.cpp.

use crate::error::{FalcoError, Result};
use crate::rule::{ListDef, MacroDef, Rule};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulePack {
    pub rules: Vec<Rule>,
    pub macros: Vec<MacroDef>,
    pub lists: Vec<ListDef>,
}

/// Falco's rule-file is a YAML sequence whose entries are single-key
/// maps (`rule:`, `macro:`, `list:`). We map that wire shape to the
/// typed `RulePack` here.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Entry {
    Rule(Rule),
    Macro(MacroDef),
    List(ListDef),
}

pub fn parse(yaml: &str) -> Result<RulePack> {
    // Try as direct typed entries first.
    if let Ok(seq) = serde_yaml::from_str::<Vec<Entry>>(yaml) {
        let mut pack = RulePack::default();
        for e in seq {
            match e {
                Entry::Rule(r) => pack.rules.push(r),
                Entry::Macro(m) => pack.macros.push(m),
                Entry::List(l) => pack.lists.push(l),
            }
        }
        return Ok(pack);
    }
    // Falco's YAML wraps each entry inside a `rule:` / `macro:` / `list:`
    // single-key map. Re-parse with a value::Value walker.
    let raw: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    let arr = raw.as_sequence().ok_or_else(|| FalcoError::RuleParse(
        "rule pack must be a YAML sequence".into(),
    ))?;
    let mut pack = RulePack::default();
    for v in arr {
        let m = v.as_mapping().ok_or_else(|| FalcoError::RuleParse(
            "rule-pack entry must be a single-key map".into(),
        ))?;
        let (k, body) = m.iter().next().ok_or_else(|| FalcoError::RuleParse(
            "rule-pack entry must have one key".into(),
        ))?;
        let key = k.as_str().ok_or_else(|| FalcoError::RuleParse(
            "rule-pack key must be a string".into(),
        ))?;
        match key {
            "rule" => {
                // Body is a map with `name`, `desc`, `condition`, etc. Inject
                // the body fields back into the Rule struct's expected shape.
                let yaml_body = serde_yaml::to_string(body)?;
                let r: Rule = serde_yaml::from_str(&yaml_body)?;
                pack.rules.push(r);
            }
            "macro" => {
                let yaml_body = serde_yaml::to_string(body)?;
                let m: MacroDef = serde_yaml::from_str(&yaml_body)?;
                pack.macros.push(m);
            }
            "list" => {
                let yaml_body = serde_yaml::to_string(body)?;
                let l: ListDef = serde_yaml::from_str(&yaml_body)?;
                pack.lists.push(l);
            }
            other => {
                return Err(FalcoError::RuleParse(format!("unknown entry type '{other}'")));
            }
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Priority;

    const FALCO_STYLE: &str = r#"
- rule:
    name: Outbound DNS to non-system resolver
    desc: detect when a container does DNS via a non-standard nameserver
    condition: outbound and fd.sport=53
    output: "DNS to %fd.rip from %proc.name"
    priority: NOTICE
    tags: [network]
- macro:
    name: outbound
    condition: evt.type in (sendto, sendmsg) and fd.l4proto=udp
- list:
    name: shell_binaries
    items: [bash, sh, zsh, fish]
"#;

    #[test]
    fn parses_full_falco_style_pack() {
        let p = parse(FALCO_STYLE).unwrap();
        assert_eq!(p.rules.len(), 1);
        assert_eq!(p.macros.len(), 1);
        assert_eq!(p.lists.len(), 1);
        assert_eq!(p.rules[0].priority, Priority::Notice);
        assert_eq!(p.macros[0].name, "outbound");
        assert_eq!(p.lists[0].items.len(), 4);
    }

    #[test]
    fn rejects_non_sequence_payload() {
        let r = parse("key: value");
        assert!(r.is_err());
    }

    #[test]
    fn rejects_unknown_entry_kind() {
        let r = parse("- foo:\n    bar: baz");
        assert!(r.is_err());
    }

    #[test]
    fn parses_empty_pack() {
        let p = parse("[]").unwrap();
        assert!(p.rules.is_empty());
    }

    #[test]
    fn parses_multi_rule_pack() {
        let y = r#"
- rule: { name: A, desc: a, condition: 1=1, priority: WARNING, output: hi }
- rule: { name: B, desc: b, condition: 1=1, priority: ERROR, output: hi }
- rule: { name: C, desc: c, condition: 1=1, priority: CRITICAL, output: hi }
"#;
        let p = parse(y).unwrap();
        assert_eq!(p.rules.len(), 3);
        assert_eq!(p.rules[2].priority, Priority::Critical);
    }
}
