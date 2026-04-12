use crate::error::{VaultError, VaultResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Create,
    Read,
    Update,
    Delete,
    List,
    Sudo,
    Deny,
    Patch,
}

impl Capability {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "create" => Some(Self::Create),
            "read" => Some(Self::Read),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            "list" => Some(Self::List),
            "sudo" => Some(Self::Sudo),
            "deny" => Some(Self::Deny),
            "patch" => Some(Self::Patch),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub path: String,
    pub capabilities: Vec<Capability>,
}

impl PolicyRule {
    pub fn matches(&self, path: &str) -> bool {
        if self.path.ends_with('*') {
            let prefix = &self.path[..self.path.len() - 1];
            path.starts_with(prefix)
        } else if self.path.ends_with('+') {
            let prefix = &self.path[..self.path.len() - 1];
            if let Some(rest) = path.strip_prefix(prefix) {
                !rest.contains('/')
            } else {
                false
            }
        } else {
            self.path == path
        }
    }

    pub fn allows(&self, cap: &Capability) -> bool {
        if self.capabilities.contains(&Capability::Deny) {
            return false;
        }
        self.capabilities.contains(cap)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub rules: Vec<PolicyRule>,
    pub raw: String,
}

impl Policy {
    pub fn parse(name: &str, hcl: &str) -> VaultResult<Self> {
        let mut rules = Vec::new();
        let path_re = regex::Regex::new(r#"path\s+"([^"]+)"\s*\{([^}]*)\}"#)
            .map_err(|e| VaultError::Internal(e.to_string()))?;
        let cap_re = regex::Regex::new(r#"capabilities\s*=\s*\[([^\]]*)\]"#)
            .map_err(|e| VaultError::Internal(e.to_string()))?;

        for caps in path_re.captures_iter(hcl) {
            let path = caps[1].to_string();
            let body = &caps[2];
            let mut capabilities = Vec::new();
            if let Some(cap_match) = cap_re.captures(body) {
                for cap_str in cap_match[1].split(',') {
                    let cap_str = cap_str.trim().trim_matches('"');
                    if let Some(cap) = Capability::from_str(cap_str) {
                        capabilities.push(cap);
                    }
                }
            }
            rules.push(PolicyRule { path, capabilities });
        }
        Ok(Policy { name: name.to_string(), rules, raw: hcl.to_string() })
    }

    pub fn allows(&self, path: &str, cap: &Capability) -> bool {
        let mut best: Option<&PolicyRule> = None;
        let mut best_len = 0;
        for rule in &self.rules {
            if rule.matches(path) {
                let rule_len = rule.path.trim_end_matches('*').trim_end_matches('+').len();
                if rule_len > best_len {
                    best_len = rule_len;
                    best = Some(rule);
                }
            }
        }
        best.map_or(false, |r| r.allows(cap))
    }
}

#[derive(Default)]
pub struct PolicyStore {
    policies: HashMap<String, Policy>,
}

impl PolicyStore {
    pub fn new() -> Self {
        let mut store = Self::default();
        store.policies.insert("root".to_string(), Policy {
            name: "root".to_string(),
            rules: vec![PolicyRule {
                path: "*".to_string(),
                capabilities: vec![Capability::Create, Capability::Read, Capability::Update,
                    Capability::Delete, Capability::List, Capability::Sudo],
            }],
            raw: r#"path "*" { capabilities = ["create", "read", "update", "delete", "list", "sudo"] }"#.to_string(),
        });
        store.policies.insert("default".to_string(), Policy {
            name: "default".to_string(),
            rules: vec![
                PolicyRule { path: "auth/token/lookup-self".to_string(), capabilities: vec![Capability::Read] },
                PolicyRule { path: "auth/token/renew-self".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "auth/token/revoke-self".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "sys/capabilities-self".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "sys/leases/lookup".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "sys/leases/renew".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "sys/renew".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "sys/tools/hash".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "sys/tools/random/*".to_string(), capabilities: vec![Capability::Update] },
                PolicyRule { path: "identity/oidc/provider/+/userinfo".to_string(), capabilities: vec![Capability::Read, Capability::Update] },
            ],
            raw: String::new(),
        });
        store
    }

    pub fn get(&self, name: &str) -> Option<&Policy> {
        self.policies.get(name)
    }

    pub fn put(&mut self, policy: Policy) {
        self.policies.insert(policy.name.clone(), policy);
    }

    pub fn delete(&mut self, name: &str) -> bool {
        if name == "root" || name == "default" {
            return false;
        }
        self.policies.remove(name).is_some()
    }

    pub fn list(&self) -> Vec<String> {
        let mut names: Vec<String> = self.policies.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn check(&self, policy_names: &[String], path: &str, cap: &Capability) -> bool {
        if policy_names.contains(&"root".to_string()) {
            return true;
        }
        for name in policy_names {
            if let Some(policy) = self.policies.get(name) {
                if policy.allows(path, cap) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_parse() {
        let hcl = r#"
path "secret/*" {
  capabilities = ["read", "list"]
}
path "secret/admin/*" {
  capabilities = ["create", "read", "update", "delete", "list"]
}
"#;
        let policy = Policy::parse("test", hcl).unwrap();
        assert_eq!(policy.rules.len(), 2);
        assert!(policy.allows("secret/foo", &Capability::Read));
        assert!(policy.allows("secret/foo", &Capability::List));
        assert!(!policy.allows("secret/foo", &Capability::Create));
        assert!(policy.allows("secret/admin/foo", &Capability::Create));
    }

    #[test]
    fn test_policy_store_root_allows_all() {
        let store = PolicyStore::new();
        let policies = vec!["root".to_string()];
        assert!(store.check(&policies, "anything/path", &Capability::Delete));
        assert!(store.check(&policies, "sys/seal", &Capability::Sudo));
    }

    #[test]
    fn test_policy_deny() {
        let policy = Policy {
            name: "test".to_string(),
            rules: vec![PolicyRule {
                path: "secret/forbidden".to_string(),
                capabilities: vec![Capability::Deny],
            }],
            raw: String::new(),
        };
        assert!(!policy.allows("secret/forbidden", &Capability::Read));
    }
}
