// SPDX-License-Identifier: AGPL-3.0-or-later
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyRule {
    pub path: String,
    pub capabilities: Vec<Capability>,
    /// Cite: openbao `vault/policy.go:139` (`AllowedParametersHCL`).
    /// Whitelist of request body parameters; if non-empty, only these
    /// keys may appear in a write.
    #[serde(default)]
    pub allowed_parameters: Vec<String>,
    /// Cite: openbao `vault/policy.go:140` (`DeniedParametersHCL`).
    /// Blacklist of request body parameters; takes precedence over
    /// `allowed_parameters`.
    #[serde(default)]
    pub denied_parameters: Vec<String>,
    /// Cite: openbao `vault/policy.go:141` (`RequiredParametersHCL`).
    /// Mandatory request body parameters.
    #[serde(default)]
    pub required_parameters: Vec<String>,
    /// Cite: openbao `vault/policy.go:137` (`MinWrappingTTLHCL`).
    /// Minimum response-wrap TTL in seconds for matching paths.
    #[serde(default)]
    pub min_wrapping_ttl_seconds: i64,
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

    /// Cite: openbao `vault/acl.go::AllowOperation` parameter check —
    /// the request body keys are validated against `required` /
    /// `allowed` / `denied`. Returns `Ok(())` when allowed, otherwise
    /// `Err(reason)` describing the first failure.
    pub fn check_parameters<I>(&self, body_keys: I) -> Result<(), String>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let keys: Vec<String> = body_keys.into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();

        // Required: every required key must be present.
        for req in &self.required_parameters {
            if !keys.iter().any(|k| k == req) {
                return Err(format!("missing required parameter: {}", req));
            }
        }

        for k in &keys {
            // Denied takes precedence over allowed.
            if self.denied_parameters.iter().any(|d| d == k || d == "*") {
                return Err(format!("parameter '{}' is denied", k));
            }
        }

        if !self.allowed_parameters.is_empty()
            && !self.allowed_parameters.iter().any(|a| a == "*")
        {
            for k in &keys {
                if !self.allowed_parameters.iter().any(|a| a == k) {
                    return Err(format!("parameter '{}' is not in allowed_parameters", k));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub rules: Vec<PolicyRule>,
    pub raw: String,
}

impl Policy {
    /// Parse HCL ACL policy. Mirrors openbao
    /// `vault/policy.go:253` (ParseACLPolicy) + `:302` (parsePaths). Strips
    /// `# …` and `// …` line comments before regex extraction so reviewer-
    /// friendly policies parse cleanly.
    pub fn parse(name: &str, hcl: &str) -> VaultResult<Self> {
        let stripped = strip_hcl_comments(hcl);
        let mut rules = Vec::new();
        let cap_re = regex::Regex::new(r#"capabilities\s*=\s*\[([^\]]*)\]"#)
            .map_err(|e| VaultError::Internal(e.to_string()))?;
        let required_re = regex::Regex::new(r#"required_parameters\s*=\s*\[([^\]]*)\]"#)
            .map_err(|e| VaultError::Internal(e.to_string()))?;
        let min_wrap_re = regex::Regex::new(r#"min_wrapping_ttl\s*=\s*"?([0-9]+)([smhd]?)"?"#)
            .map_err(|e| VaultError::Internal(e.to_string()))?;

        for (path, body) in extract_path_blocks(&stripped) {
            let body = body.as_str();
            let mut capabilities = Vec::new();
            if let Some(cap_match) = cap_re.captures(body) {
                for cap_str in split_csv_keep_quoted(&cap_match[1]) {
                    let cap_str = cap_str.trim().trim_matches('"').trim();
                    if cap_str.is_empty() { continue; }
                    if let Some(cap) = Capability::from_str(cap_str) {
                        capabilities.push(cap);
                    }
                }
            }

            let allowed_parameters = extract_inner_block(body, "allowed_parameters")
                .map(|inner| extract_param_keys(&inner)).unwrap_or_default();
            let denied_parameters = extract_inner_block(body, "denied_parameters")
                .map(|inner| extract_param_keys(&inner)).unwrap_or_default();
            let required_parameters = required_re.captures(body)
                .map(|m| split_csv_keep_quoted(&m[1])
                    .into_iter()
                    .map(|s| s.trim().trim_matches('"').trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect())
                .unwrap_or_default();
            let min_wrapping_ttl_seconds = min_wrap_re.captures(body)
                .map(|m| {
                    let n: i64 = m[1].parse().unwrap_or(0);
                    let unit = m.get(2).map(|x| x.as_str()).unwrap_or("");
                    match unit {
                        "m" => n * 60,
                        "h" => n * 3600,
                        "d" => n * 86_400,
                        _   => n,
                    }
                })
                .unwrap_or(0);

            rules.push(PolicyRule {
                path,
                capabilities,
                allowed_parameters,
                denied_parameters,
                required_parameters,
                min_wrapping_ttl_seconds,
            });
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
                ..Default::default()
            }],
            raw: r#"path "*" { capabilities = ["create", "read", "update", "delete", "list", "sudo"] }"#.to_string(),
        });
        store.policies.insert("default".to_string(), Policy {
            name: "default".to_string(),
            rules: vec![
                PolicyRule { path: "auth/token/lookup-self".to_string(), capabilities: vec![Capability::Read] , ..Default::default() },
                PolicyRule { path: "auth/token/renew-self".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "auth/token/revoke-self".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "sys/capabilities-self".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "sys/leases/lookup".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "sys/leases/renew".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "sys/renew".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "sys/tools/hash".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "sys/tools/random/*".to_string(), capabilities: vec![Capability::Update] , ..Default::default() },
                PolicyRule { path: "identity/oidc/provider/+/userinfo".to_string(), capabilities: vec![Capability::Read, Capability::Update] , ..Default::default() },
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

/// Walk an HCL document and yield `(path, body)` for every
/// `path "<x>" { … }` block. Brace-aware so nested `{ … }` blocks
/// (e.g. `denied_parameters = { … }`) don't truncate the body.
fn extract_path_blocks(hcl: &str) -> Vec<(String, String)> {
    let path_header = regex::Regex::new(r#"path\s+"([^"]+)"\s*\{"#).expect("static regex");
    let mut out = Vec::new();
    let bytes = hcl.as_bytes();
    let mut cursor = 0usize;
    while let Some(m) = path_header.find_at(hcl, cursor) {
        let header_end = m.end();
        let path = path_header.captures(&hcl[m.start()..])
            .and_then(|c| c.get(1))
            .map(|g| g.as_str().to_string())
            .unwrap_or_default();

        // Walk forward from header_end balancing braces. The header_end
        // is positioned right after the opening `{` of the path block.
        let mut depth: i32 = 1;
        let mut idx = header_end;
        while idx < bytes.len() && depth > 0 {
            match bytes[idx] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            idx += 1;
        }
        if depth == 0 {
            // body is exclusive of the closing brace
            let body = &hcl[header_end..idx - 1];
            out.push((path, body.to_string()));
            cursor = idx;
        } else {
            // unbalanced — skip the rest
            break;
        }
    }
    out
}

/// Locate `<key> = { … }` inside a path body and return the inner text.
/// Brace-aware so further nested blocks don't truncate.
fn extract_inner_block(body: &str, key: &str) -> Option<String> {
    let header = regex::Regex::new(&format!(r#"{key}\s*=\s*\{{"#, key = regex::escape(key)))
        .ok()?;
    let m = header.find(body)?;
    let header_end = m.end();
    let bytes = body.as_bytes();
    let mut depth: i32 = 1;
    let mut idx = header_end;
    while idx < bytes.len() && depth > 0 {
        match bytes[idx] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        idx += 1;
    }
    if depth == 0 {
        Some(body[header_end..idx - 1].to_string())
    } else {
        None
    }
}

/// Strip `# …` and `// …` line comments from an HCL document. Preserves
/// in-string `#` characters by toggling on `"` boundaries.
fn strip_hcl_comments(hcl: &str) -> String {
    let mut out = String::with_capacity(hcl.len());
    let mut in_string = false;
    let mut prev = '\0';
    let mut iter = hcl.chars().peekable();
    while let Some(c) = iter.next() {
        if c == '"' && prev != '\\' {
            in_string = !in_string;
            out.push(c);
            prev = c;
            continue;
        }
        if !in_string {
            if c == '#' {
                while let Some(&n) = iter.peek() {
                    if n == '\n' { break; }
                    iter.next();
                }
                continue;
            }
            if c == '/' && iter.peek() == Some(&'/') {
                iter.next();
                while let Some(&n) = iter.peek() {
                    if n == '\n' { break; }
                    iter.next();
                }
                continue;
            }
        }
        out.push(c);
        prev = c;
    }
    out
}

/// Split a CSV-style HCL list, respecting `"…"` quoting (so values
/// containing commas don't split incorrectly).
fn split_csv_keep_quoted(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in input.chars() {
        match c {
            '"' => { in_quotes = !in_quotes; cur.push(c); }
            ',' if !in_quotes => {
                if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

/// Extract parameter keys from an HCL map body. Each entry has the
/// form `"key" = [...values...]`; we only care about the keys here.
fn extract_param_keys(body: &str) -> Vec<String> {
    let key_re = regex::Regex::new(r#""([^"]+)"\s*=\s*\["#).expect("static regex");
    key_re.captures_iter(body)
        .map(|c| c[1].to_string())
        .collect()
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
                ..Default::default()
            }],
            raw: String::new(),
        };
        assert!(!policy.allows("secret/forbidden", &Capability::Read));
    }

    #[test]
    fn test_policy_allows_matching_path() {
        let policy = Policy {
            name: "p".into(),
            rules: vec![PolicyRule {
                path: "secret/data/myapp".into(),
                capabilities: vec![Capability::Read, Capability::Update],
                ..Default::default()
            }],
            raw: String::new(),
        };
        assert!(policy.allows("secret/data/myapp", &Capability::Read));
        assert!(policy.allows("secret/data/myapp", &Capability::Update));
        assert!(!policy.allows("secret/data/myapp", &Capability::Delete));
        assert!(!policy.allows("secret/data/other", &Capability::Read));
    }

    #[test]
    fn test_policy_deny_overrides() {
        // A rule with Deny in capabilities must reject all caps regardless of others.
        let r = PolicyRule {
            path: "secret/x".into(),
            capabilities: vec![Capability::Read, Capability::Deny],
            ..Default::default()
        };
        assert!(!r.allows(&Capability::Read));
        assert!(!r.allows(&Capability::Update));
    }

    #[test]
    fn test_root_policy_allows_all() {
        let store = PolicyStore::new();
        let names = vec!["root".to_string()];
        // Any path, any cap should be allowed for the root policy.
        assert!(store.check(&names, "v1/secret/data/anything", &Capability::Delete));
        assert!(store.check(&names, "sys/seal", &Capability::Sudo));
        assert!(store.check(&names, "auth/token/revoke", &Capability::Update));
    }

    #[test]
    fn test_policy_glob_suffix_match() {
        let r = PolicyRule {
            path: "kv/data/*".into(),
            capabilities: vec![Capability::Read],
            ..Default::default()
        };
        assert!(r.matches("kv/data/foo"));
        assert!(r.matches("kv/data/foo/bar"));
        assert!(!r.matches("kv/metadata/foo"));
    }

    #[test]
    fn test_policy_plus_segment_match() {
        // `+` matches a single segment without slashes.
        let r = PolicyRule {
            path: "kv/data/+".into(),
            capabilities: vec![Capability::Read],
            ..Default::default()
        };
        assert!(r.matches("kv/data/foo"));
        assert!(!r.matches("kv/data/foo/bar"));
    }

    #[test]
    fn test_policy_check_required_parameters() {
        let r = PolicyRule {
            path: "auth/x".into(),
            capabilities: vec![Capability::Update],
            required_parameters: vec!["username".into()],
            ..Default::default()
        };
        assert!(r.check_parameters(["username"]).is_ok());
        assert!(r.check_parameters(["password"]).is_err());
    }

    #[test]
    fn test_policy_store_default_includes_default() {
        let store = PolicyStore::new();
        let names = store.list();
        assert!(names.contains(&"root".to_string()));
        assert!(names.contains(&"default".to_string()));
    }

    #[test]
    fn test_policy_store_root_default_undeletable() {
        let mut store = PolicyStore::new();
        assert!(!store.delete("root"));
        assert!(!store.delete("default"));
    }

    #[test]
    fn test_policy_capability_from_str_all_variants() {
        for s in ["create", "read", "update", "delete", "list", "sudo", "deny", "patch"] {
            assert!(Capability::from_str(s).is_some(), "missing {s}");
        }
        assert!(Capability::from_str("invalid").is_none());
    }
}
