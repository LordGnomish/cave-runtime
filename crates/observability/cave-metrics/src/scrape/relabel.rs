// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Relabeling engine — line-by-line port of prometheus/prometheus
//! `model/relabel/relabel.go` (v3.12.0, source_sha
//! a0524eeca91b19eb60d2b02f8a1c0019954e3405).
//!
//! Relabeling rewrites a label set before it is scraped or ingested. Each
//! [`RelabelConfig`] concatenates the values of `source_labels` with
//! `separator`, matches the result against the anchored `regex`, and then
//! applies an [`Action`]. [`process`] runs a chain of configs in order and
//! returns `false` as soon as a `keep`/`drop`/`*equal` rule says to drop the
//! whole target (mirroring upstream's `ProcessBuilder`).

use crate::model::Labels;
use md5::{Digest, Md5};
use regex::Regex;

/// The action performed by a relabel rule. Same set as upstream's `Action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Regex replacement into `target_label` (the default).
    Replace,
    /// Drop the target if the concatenation does **not** match `regex`.
    Keep,
    /// Drop the target if the concatenation **does** match `regex`.
    Drop,
    /// Drop the target unless the source concatenation equals `target_label`.
    KeepEqual,
    /// Drop the target if the source concatenation equals `target_label`.
    DropEqual,
    /// Set `target_label` to `hash(concat) % modulus`.
    HashMod,
    /// Copy every label whose **name** matches `regex` to a new name.
    LabelMap,
    /// Delete every label whose name matches `regex`.
    LabelDrop,
    /// Delete every label whose name does **not** match `regex`.
    LabelKeep,
    /// Lower-case the concatenation into `target_label`.
    Lowercase,
    /// Upper-case the concatenation into `target_label`.
    Uppercase,
}

/// A single relabeling rule. Defaults mirror upstream `DefaultRelabelConfig`:
/// `Replace`, separator `;`, regex `(.*)`, replacement `$1`.
#[derive(Debug, Clone)]
pub struct RelabelConfig {
    pub source_labels: Vec<String>,
    pub separator: String,
    pub regex: String,
    pub modulus: u64,
    pub target_label: String,
    pub replacement: String,
    pub action: Action,
}

impl Default for RelabelConfig {
    fn default() -> Self {
        Self {
            source_labels: Vec::new(),
            separator: ";".to_string(),
            regex: "(.*)".to_string(),
            modulus: 0,
            target_label: String::new(),
            replacement: "$1".to_string(),
            action: Action::Replace,
        }
    }
}

/// Apply a chain of relabel configs to `lb`. Returns `false` if any rule
/// drops the target (upstream `ProcessBuilder`).
pub fn process(lb: &mut Labels, cfgs: &[RelabelConfig]) -> bool {
    for cfg in cfgs {
        if !relabel_one(cfg, lb) {
            return false;
        }
    }
    true
}

/// Anchor a relabel regex exactly like upstream `NewRegexp`: `^(?s:RE)$`.
fn compile(re: &str) -> Option<Regex> {
    Regex::new(&format!("^(?s:{})$", re)).ok()
}

/// Prometheus legacy label-name validity: `[a-zA-Z_][a-zA-Z0-9_]*`.
fn is_valid_label_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn relabel_one(cfg: &RelabelConfig, lb: &mut Labels) -> bool {
    // Concatenate source label values with the separator.
    let values: Vec<&str> = cfg
        .source_labels
        .iter()
        .map(|n| lb.get(n).unwrap_or(""))
        .collect();
    let val = values.join(&cfg.separator);

    let Some(re) = compile(&cfg.regex) else {
        // Invalid regex: behave as a no-op keep, never crash a scrape loop.
        return true;
    };

    match cfg.action {
        Action::Drop => {
            if re.is_match(&val) {
                return false;
            }
        }
        Action::Keep => {
            if !re.is_match(&val) {
                return false;
            }
        }
        Action::DropEqual => {
            if lb.get(&cfg.target_label).unwrap_or("") == val {
                return false;
            }
        }
        Action::KeepEqual => {
            if lb.get(&cfg.target_label).unwrap_or("") != val {
                return false;
            }
        }
        Action::Replace => {
            // Fast path: empty input + default regex + literal target/replacement
            // simply adds (or sets) the label pair.
            if val.is_empty()
                && cfg.regex == "(.*)"
                && !cfg.target_label.contains('$')
                && !cfg.replacement.contains('$')
            {
                lb.insert(cfg.target_label.clone(), cfg.replacement.clone());
                return true;
            }
            if let Some(caps) = re.captures(&val) {
                let mut target = String::new();
                caps.expand(&cfg.target_label, &mut target);
                if !is_valid_label_name(&target) {
                    return true;
                }
                let mut res = String::new();
                caps.expand(&cfg.replacement, &mut res);
                if res.is_empty() {
                    lb.0.remove(&target);
                } else {
                    lb.insert(target, res);
                }
            }
        }
        Action::Lowercase => {
            lb.insert(cfg.target_label.clone(), val.to_lowercase());
        }
        Action::Uppercase => {
            lb.insert(cfg.target_label.clone(), val.to_uppercase());
        }
        Action::HashMod => {
            let mut hasher = Md5::new();
            hasher.update(val.as_bytes());
            let digest = hasher.finalize();
            // Use only the last 8 bytes of the hash (big-endian) so the result
            // matches every earlier version of Prometheus's hashmod.
            let last8: [u8; 8] = digest[8..16]
                .try_into()
                .expect("md5 digest is 16 bytes");
            let h = u64::from_be_bytes(last8);
            let m = if cfg.modulus == 0 { 0 } else { h % cfg.modulus };
            lb.insert(cfg.target_label.clone(), m.to_string());
        }
        Action::LabelMap => {
            let snapshot: Vec<(String, String)> = lb
                .0
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            for (name, value) in snapshot {
                if let Some(caps) = re.captures(&name) {
                    let mut new_name = String::new();
                    caps.expand(&cfg.replacement, &mut new_name);
                    if is_valid_label_name(&new_name) {
                        lb.insert(new_name, value);
                    }
                }
            }
        }
        Action::LabelDrop => {
            let names: Vec<String> = lb.0.keys().cloned().collect();
            for n in names {
                if re.is_match(&n) {
                    lb.0.remove(&n);
                }
            }
        }
        Action::LabelKeep => {
            let names: Vec<String> = lb.0.keys().cloned().collect();
            for n in names {
                if !re.is_match(&n) {
                    lb.0.remove(&n);
                }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_replace_with_dot_star() {
        let c = RelabelConfig::default();
        assert_eq!(c.action, Action::Replace);
        assert_eq!(c.separator, ";");
        assert_eq!(c.regex, "(.*)");
        assert_eq!(c.replacement, "$1");
    }

    #[test]
    fn invalid_regex_is_a_noop_keep() {
        let mut lb = Labels::from_pairs([("a", "x")]);
        let keep = process(
            &mut lb,
            &[RelabelConfig {
                source_labels: vec!["a".into()],
                regex: "(".into(), // does not compile
                action: Action::Keep,
                ..RelabelConfig::default()
            }],
        );
        assert!(keep);
    }
}
