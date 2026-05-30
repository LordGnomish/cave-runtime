// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Component registration framework.
//!
//! Line-ported from grafana/alloy `internal/component/registry.go` +
//! `internal/featuregate/featuregate.go` (v1.5.0, Apache-2.0).
//!
//! Where upstream uses a process-global `registered` map and `panic`s on
//! invalid registrations, this port uses an explicit [`Registry`] value and
//! returns `Result` errors — the validation rules are identical.

use std::collections::HashMap;

/// Overall stability level of a component, mirroring `featuregate.Stability`.
/// Ordering is significant: a higher level is "more stable".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stability {
    /// The default value, indicating an error — should never be used.
    Undefined,
    /// Experimental features.
    Experimental,
    /// Public Preview features.
    PublicPreview,
    /// Generally Available features.
    GenerallyAvailable,
}

impl Stability {
    /// String name, mirroring `Stability.String`.
    pub fn name(self) -> &'static str {
        match self {
            Stability::Undefined => "undefined",
            Stability::Experimental => "experimental",
            Stability::PublicPreview => "public-preview",
            Stability::GenerallyAvailable => "generally-available",
        }
    }
}

/// Reports whether a feature at `stability` is permitted when the minimum
/// allowed stability is `min_stability`. Mirrors `featuregate.AllowAtStability`.
///
/// An [`Stability::Undefined`] on either side is an error.
#[allow(non_snake_case)]
pub fn AllowAtStability(stability: Stability, min_stability: Stability) -> Result<bool, String> {
    if stability == Stability::Undefined || min_stability == Stability::Undefined {
        return Err(format!(
            "stability levels must be defined: got {} as stability and {} as the minimum stability",
            stability.name(),
            min_stability.name()
        ));
    }
    Ok(stability >= min_stability)
}

/// Describes a single registered component. A trimmed port of
/// `component.Registration` carrying the fields the validation rules read
/// (the `Args`/`Exports`/`Build` reflection machinery is out of scope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// Period-delimited component name (e.g. `"remote.s3"`).
    pub name: String,
    /// Overall stability level. Must be non-`Undefined` unless `community`.
    pub stability: Stability,
    /// True if this is a community component.
    pub community: bool,
}

/// A registry of components keyed by name. Mirrors the module-level
/// `registered` + `parsedNames` maps in `registry.go`.
#[derive(Debug, Default)]
pub struct Registry {
    registered: HashMap<String, Registration>,
    parsed_names: HashMap<String, Vec<String>>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Registers a component, mirroring `component.Register`'s validation:
    ///   - the name must not already be registered,
    ///   - non-community components must have a defined stability level,
    ///   - community components must leave stability undefined,
    ///   - the name must be valid,
    ///   - the name must not be solely a prefix of (or prefixed by) an existing
    ///     component name.
    pub fn register(&mut self, r: Registration) -> Result<(), String> {
        if self.registered.contains_key(&r.name) {
            return Err(format!("Component name {:?} already registered", r.name));
        }
        if !r.community && r.stability == Stability::Undefined {
            return Err(format!(
                "Component {:?} has an undefined stability level - please provide stability level when registering the component",
                r.name
            ));
        }
        if r.community && r.stability != Stability::Undefined {
            return Err(format!(
                "Community component {:?} has a defined stability level - community components should remain `undefined`",
                r.name
            ));
        }

        let parsed = parse_component_name(&r.name)?;
        validate_prefix_match(&parsed, &self.parsed_names)?;

        self.parsed_names.insert(r.name.clone(), parsed);
        self.registered.insert(r.name.clone(), r);
        Ok(())
    }

    /// Returns the registration for `name`, if present.
    pub fn get(&self, name: &str) -> Option<&Registration> {
        self.registered.get(name)
    }

    /// Number of registered components.
    pub fn len(&self) -> usize {
        self.registered.len()
    }

    /// True if no components are registered.
    pub fn is_empty(&self) -> bool {
        self.registered.is_empty()
    }
}

/// Parses and validates a component name. `"remote.http"` returns
/// `["remote", "http"]`. Mirrors `parseComponentName`: parts are split on `.`,
/// none may be empty, and each must match `^[A-Za-z][0-9A-Za-z_]*$`.
pub fn parse_component_name(name: &str) -> Result<Vec<String>, String> {
    let parts: Vec<&str> = name.split('.').collect();
    if parts.is_empty() {
        return Err("missing name".to_string());
    }
    for part in &parts {
        if part.is_empty() {
            return Err("found empty identifier".to_string());
        }
        if !is_valid_component_identifier(part) {
            return Err(format!("identifier {:?} is not valid", part));
        }
    }
    Ok(parts.into_iter().map(|s| s.to_string()).collect())
}

/// `^[A-Za-z][0-9A-Za-z_]*$`
fn is_valid_component_identifier(part: &str) -> bool {
    let mut chars = part.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validates that no component name is solely a prefix of another. Mirrors
/// `validatePrefixMatch`: a trailing `.` is appended to each name so only
/// complete segments match.
fn validate_prefix_match(
    check: &[String],
    against: &HashMap<String, Vec<String>>,
) -> Result<(), String> {
    let name = format!("{}.", check.join("."));
    for other in against.values() {
        let other_name = format!("{}.", other.join("."));
        if other_name.starts_with(&name) || name.starts_with(&other_name) {
            return Err(format!(
                "{:?} cannot be used because it is incompatible with {:?}",
                check.join("."),
                other.join(".")
            ));
        }
    }
    Ok(())
}
