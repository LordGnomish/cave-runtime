// SPDX-License-Identifier: AGPL-3.0-or-later
//! Policy engine — path-based ACL with capabilities.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::VaultError;

// ── Capabilities ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Create,
    Read,
    Update,
    Delete,
    List,
    Sudo,
    Deny,
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Capability::Create => "create",
            Capability::Read   => "read",
            Capability::Update => "update",
            Capability::Delete => "delete",
            Capability::List   => "list",
            Capability::Sudo   => "sudo",
            Capability::Deny   => "deny",
        };
        write!(f, "{s}")
    }
}

// ── Policy ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPath {
    /// Glob-style path, e.g. "secret/data/*" or "auth/token/create".
    pub path: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub paths: Vec<PolicyPath>,
}

impl Policy {
    /// Return the capabilities for a specific path.
    /// Uses the most-specific matching path glob.
    pub fn capabilities_for(&self, path: &str) -> Vec<&Capability> {
        // Find the most-specific (longest) matching rule
        let best = self
            .paths
            .iter()
            .filter(|pp| path_matches(&pp.path, path))
            .max_by_key(|pp| pp.path.len());

        match best {
            Some(pp) => pp.capabilities.iter().collect(),
            None     => vec![],
        }
    }

    pub fn allows(&self, path: &str, cap: &Capability) -> bool {
        let caps = self.capabilities_for(path);
        if caps.contains(&&Capability::Deny) {
            return false;
        }
        caps.contains(&cap)
    }
}

/// Simple glob match: `*` matches any sequence of non-`/` chars; `**` or
/// trailing `*` matches everything.
fn path_matches(pattern: &str, path: &str) -> bool {
    // Bare "**" matches everything
    if pattern == "**" || pattern == "*" {
        return true;
    }
    if pattern == path {
        return true;
    }
    // "secret/**" matches any depth below "secret/"
    if let Some(prefix) = pattern.strip_suffix("/**") {
        return path.starts_with(&format!("{prefix}/")) || path == prefix;
    }
    // "secret/*" matches exactly one level below "secret/"
    if let Some(prefix) = pattern.strip_suffix("/*") {
        if prefix.is_empty() {
            return !path.contains('/');
        }
        let pfx = format!("{prefix}/");
        return path.starts_with(&pfx) && !path[pfx.len()..].contains('/');
    }
    false
}

// ── Built-in policies ─────────────────────────────────────────────────────────

pub fn root_policy() -> Policy {
    Policy {
        name: "root".to_string(),
        paths: vec![PolicyPath {
            path: "**".to_string(),
            capabilities: vec![
                Capability::Create,
                Capability::Read,
                Capability::Update,
                Capability::Delete,
                Capability::List,
                Capability::Sudo,
            ],
        }],
    }
}

pub fn default_policy() -> Policy {
    Policy {
        name: "default".to_string(),
        paths: vec![
            PolicyPath {
                path: "auth/token/lookup-self".to_string(),
                capabilities: vec![Capability::Read],
            },
            PolicyPath {
                path: "auth/token/renew-self".to_string(),
                capabilities: vec![Capability::Update],
            },
            PolicyPath {
                path: "auth/token/revoke-self".to_string(),
                capabilities: vec![Capability::Update],
            },
            PolicyPath {
                path: "sys/capabilities-self".to_string(),
                capabilities: vec![Capability::Update],
            },
        ],
    }
}

// ── Policy store ──────────────────────────────────────────────────────────────

pub struct PolicyEngine {
    policies: HashMap<String, Policy>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        let mut pe = Self {
            policies: HashMap::new(),
        };
        pe.put(root_policy());
        pe.put(default_policy());
        pe
    }

    pub fn put(&mut self, policy: Policy) {
        self.policies.insert(policy.name.clone(), policy);
    }

    pub fn get(&self, name: &str) -> Option<&Policy> {
        self.policies.get(name)
    }

    pub fn delete(&mut self, name: &str) -> bool {
        if name == "root" || name == "default" {
            return false; // immutable built-ins
        }
        self.policies.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.policies.keys().cloned().collect();
        names.sort();
        names
    }

    /// Check whether a set of policy names collectively grants `cap` on `path`.
    pub fn check(
        &self,
        policy_names: &[String],
        path: &str,
        cap: &Capability,
    ) -> Result<(), VaultError> {
        for name in policy_names {
            if let Some(policy) = self.policies.get(name) {
                let caps = policy.capabilities_for(path);
                // Explicit deny overrides everything
                if caps.contains(&&Capability::Deny) {
                    // detail in tracing; the canonical VaultError::PermissionDenied
                    // is a unit variant (sweep-002 cleanup; orphan policy.rs
                    // assumed the older tuple shape).
                    tracing::debug!(
                        policy = %name, capability = %cap, path = %path,
                        "policy denied"
                    );
                    return Err(VaultError::PermissionDenied);
                }
                if caps.contains(&cap) || caps.contains(&&Capability::Sudo) {
                    return Ok(());
                }
            }
        }
        tracing::debug!(capability = %cap, path = %path, "no policy grants capability");
        Err(VaultError::PermissionDenied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom_policy() -> Policy {
        Policy {
            name: "ops".to_string(),
            paths: vec![
                PolicyPath {
                    path: "secret/data/*".to_string(),
                    capabilities: vec![Capability::Read, Capability::Create, Capability::Update],
                },
                PolicyPath {
                    path: "sys/seal".to_string(),
                    capabilities: vec![Capability::Deny],
                },
            ],
        }
    }

    #[test]
    fn test_policy_allows_matching_path() {
        let p = custom_policy();
        assert!(p.allows("secret/data/mykey", &Capability::Read));
        assert!(p.allows("secret/data/mykey", &Capability::Create));
        assert!(!p.allows("secret/data/mykey", &Capability::Delete));
    }

    #[test]
    fn test_policy_deny_overrides() {
        let p = custom_policy();
        assert!(!p.allows("sys/seal", &Capability::Read));
    }

    #[test]
    fn test_policy_engine_check_grants() {
        let mut pe = PolicyEngine::new();
        pe.put(custom_policy());
        pe.check(&["ops".to_string()], "secret/data/foo", &Capability::Read)
            .unwrap();
    }

    #[test]
    fn test_policy_engine_check_denied() {
        let mut pe = PolicyEngine::new();
        pe.put(custom_policy());
        let result = pe.check(&["ops".to_string()], "sys/seal", &Capability::Update);
        assert!(result.is_err());
    }

    #[test]
    fn test_root_policy_allows_all() {
        let pe = PolicyEngine::new();
        pe.check(&["root".to_string()], "arbitrary/path/anything", &Capability::Delete)
            .unwrap();
    }
}
