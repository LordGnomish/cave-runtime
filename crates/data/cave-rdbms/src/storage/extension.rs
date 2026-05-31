// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Extension framework — `CREATE`/`DROP EXTENSION` and the `.control` format.
//!
//! Pure-Rust port of PostgreSQL's `src/backend/commands/extension.c` and the
//! `<name>.control` file grammar. An extension ships a control file declaring
//! its `default_version`, human `comment`, the other extensions it `requires`,
//! and flags such as `relocatable` / `trusted`. Installing one records a row
//! in the (in-memory) `pg_extension` catalog, but only once every required
//! extension is already present; dropping one is refused while another
//! installed extension still depends on it.

use std::collections::{HashMap, HashSet};

/// Parsed `<name>.control` contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionControl {
    pub name: String,
    pub default_version: String,
    pub comment: String,
    pub requires: Vec<String>,
    pub relocatable: bool,
    pub trusted: bool,
    pub schema: Option<String>,
}

impl ExtensionControl {
    /// A minimal control with no dependencies or flags (e.g. built-in
    /// `plpgsql`).
    pub fn bare(name: &str, default_version: &str) -> Self {
        ExtensionControl {
            name: name.to_string(),
            default_version: default_version.to_string(),
            comment: String::new(),
            requires: Vec::new(),
            relocatable: false,
            trusted: false,
            schema: None,
        }
    }
}

/// `parse_extension_control_file` — parse the `key = value` control grammar.
/// Lines starting with `#` and blank lines are ignored; string values may be
/// single-quoted; `requires` is a comma-separated list; booleans accept
/// `true`/`false`.
pub fn parse_control(name: &str, text: &str) -> ExtensionControl {
    let mut c = ExtensionControl::bare(name, "");
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim().trim_matches('\'').trim();
        match key {
            "default_version" => c.default_version = val.to_string(),
            "comment" => c.comment = val.to_string(),
            "schema" => c.schema = Some(val.to_string()),
            "relocatable" => c.relocatable = val.eq_ignore_ascii_case("true"),
            "trusted" => c.trusted = val.eq_ignore_ascii_case("true"),
            "requires" => {
                c.requires = val
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    c
}

/// Errors from `CreateExtension` / `RemoveExtensionById`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtError {
    /// a `requires` dependency is not installed
    MissingDependency(String),
    /// the extension is already in `pg_extension`
    AlreadyInstalled,
    /// drop refused: a still-installed extension depends on this one
    DependencyExists(String),
    /// no such installed extension
    NotFound,
    /// `requires` graph has a cycle (no valid install order)
    CyclicDependency,
}

#[derive(Debug, Clone)]
struct Installed {
    version: String,
    requires: Vec<String>,
}

/// The in-memory `pg_extension` catalog.
#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    installed: HashMap<String, Installed>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        ExtensionRegistry {
            installed: HashMap::new(),
        }
    }

    /// `CreateExtension` — install at the control's `default_version`, after
    /// verifying every `requires` dependency is present.
    pub fn create(&mut self, control: &ExtensionControl) -> Result<(), ExtError> {
        if self.installed.contains_key(&control.name) {
            return Err(ExtError::AlreadyInstalled);
        }
        for dep in &control.requires {
            if !self.installed.contains_key(dep) {
                return Err(ExtError::MissingDependency(dep.clone()));
            }
        }
        self.installed.insert(
            control.name.clone(),
            Installed {
                version: control.default_version.clone(),
                requires: control.requires.clone(),
            },
        );
        Ok(())
    }

    /// `RemoveExtensionById` — refused while any installed extension lists
    /// `name` in its `requires`.
    pub fn drop(&mut self, name: &str) -> Result<(), ExtError> {
        if !self.installed.contains_key(name) {
            return Err(ExtError::NotFound);
        }
        if let Some(dependent) = self
            .installed
            .iter()
            .filter(|(n, _)| n.as_str() != name)
            .find(|(_, e)| e.requires.iter().any(|r| r == name))
            .map(|(n, _)| n.clone())
        {
            return Err(ExtError::DependencyExists(dependent));
        }
        self.installed.remove(name);
        Ok(())
    }

    pub fn installed_version(&self, name: &str) -> Option<String> {
        self.installed.get(name).map(|e| e.version.clone())
    }

    pub fn is_installed(&self, name: &str) -> bool {
        self.installed.contains_key(name)
    }

    /// Topologically order a set of controls so every extension follows the
    /// dependencies it `requires`. Dependencies outside the set are treated as
    /// already satisfied. Errors on a dependency cycle.
    pub fn install_order(controls: &[ExtensionControl]) -> Result<Vec<ExtensionControl>, ExtError> {
        let in_set: HashSet<&str> = controls.iter().map(|c| c.name.as_str()).collect();
        let mut done: HashSet<String> = HashSet::new();
        let mut order: Vec<ExtensionControl> = Vec::with_capacity(controls.len());

        // Deterministic: repeatedly emit any not-yet-emitted control whose
        // in-set requirements are all already emitted.
        while order.len() < controls.len() {
            let mut progressed = false;
            for c in controls {
                if done.contains(&c.name) {
                    continue;
                }
                let ready = c
                    .requires
                    .iter()
                    .all(|r| !in_set.contains(r.as_str()) || done.contains(r));
                if ready {
                    done.insert(c.name.clone());
                    order.push(c.clone());
                    progressed = true;
                }
            }
            if !progressed {
                return Err(ExtError::CyclicDependency);
            }
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_requires_is_rejected() {
        let mut a = ExtensionControl::bare("a", "1");
        a.requires = vec!["b".into()];
        let mut b = ExtensionControl::bare("b", "1");
        b.requires = vec!["a".into()];
        assert_eq!(
            ExtensionRegistry::install_order(&[a, b]),
            Err(ExtError::CyclicDependency)
        );
    }

    #[test]
    fn comment_only_line_is_ignored() {
        let c = parse_control("x", "# just a comment\ndefault_version = '2'\n");
        assert_eq!(c.default_version, "2");
        assert!(c.comment.is_empty());
    }
}
