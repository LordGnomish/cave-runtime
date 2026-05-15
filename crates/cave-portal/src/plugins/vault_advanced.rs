// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vault wrap — policies, leases, AppRole auth.
//!
//! Layered atop [`super::vault`]. The basic wrap models the secret engine
//! mounts and entries; this module adds the policy / lease / auth surface
//! tenants normally see in OpenBao's web UI.

use super::ViewPersona;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Create,
    Read,
    Update,
    Delete,
    List,
    Sudo,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub path_glob: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub tenant: String,
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdvancedVaultError {
    #[error("invalid policy name: {0}")]
    InvalidName(String),
    #[error("policy not found: {0}")]
    NotFound(String),
    #[error("path glob may not be empty")]
    EmptyGlob,
    #[error("rule mixes deny with other capabilities")]
    DenyMixed,
    #[error("forbidden for persona {0:?}")]
    Forbidden(&'static str),
    #[error("approle bind_secret_id required")]
    ApproleBindRequired,
    #[error("invalid ttl: {0}")]
    InvalidTtl(u64),
    #[error("lease not found: {0}")]
    LeaseNotFound(String),
    #[error("lease already revoked")]
    LeaseRevoked,
}

fn validate_policy_name(name: &str) -> Result<(), AdvancedVaultError> {
    if name.is_empty() || name.len() > 64 {
        return Err(AdvancedVaultError::InvalidName(name.into()));
    }
    for ch in name.chars() {
        let ok = ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_';
        if !ok {
            return Err(AdvancedVaultError::InvalidName(name.into()));
        }
    }
    Ok(())
}

impl Policy {
    pub fn new(name: impl Into<String>, tenant: impl Into<String>) -> Result<Self, AdvancedVaultError> {
        let name = name.into();
        validate_policy_name(&name)?;
        Ok(Self {
            name,
            tenant: tenant.into(),
            rules: Vec::new(),
        })
    }

    pub fn add_rule(&mut self, rule: PolicyRule) -> Result<(), AdvancedVaultError> {
        if rule.path_glob.is_empty() {
            return Err(AdvancedVaultError::EmptyGlob);
        }
        let has_deny = rule.capabilities.contains(&Capability::Deny);
        if has_deny && rule.capabilities.len() > 1 {
            return Err(AdvancedVaultError::DenyMixed);
        }
        self.rules.push(rule);
        Ok(())
    }

    /// Test whether the policy permits `cap` on `path`. Path-glob matching
    /// is `*` for "match the rest of segment" only.
    pub fn permits(&self, path: &str, cap: Capability) -> bool {
        let mut allow = false;
        for rule in &self.rules {
            if !glob_match(&rule.path_glob, path) {
                continue;
            }
            if rule.capabilities.contains(&Capability::Deny) {
                return false;
            }
            if rule.capabilities.contains(&cap) || rule.capabilities.contains(&Capability::Sudo) {
                allow = true;
            }
        }
        allow
    }
}

fn glob_match(glob: &str, path: &str) -> bool {
    if glob == path {
        return true;
    }
    if let Some(prefix) = glob.strip_suffix('*') {
        return path.starts_with(prefix);
    }
    false
}

#[derive(Debug, Default)]
pub struct PolicyStore {
    policies: Vec<Policy>,
}

impl PolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(
        &mut self,
        persona: ViewPersona,
        policy: Policy,
    ) -> Result<(), AdvancedVaultError> {
        if !matches!(persona, ViewPersona::Admin) {
            return Err(AdvancedVaultError::Forbidden("policy edits are admin-only"));
        }
        if let Some(idx) = self
            .policies
            .iter()
            .position(|p| p.name == policy.name && p.tenant == policy.tenant)
        {
            self.policies[idx] = policy;
        } else {
            self.policies.push(policy);
        }
        Ok(())
    }

    pub fn delete(
        &mut self,
        persona: ViewPersona,
        tenant: &str,
        name: &str,
    ) -> Result<(), AdvancedVaultError> {
        if !matches!(persona, ViewPersona::Admin) {
            return Err(AdvancedVaultError::Forbidden("policy edits are admin-only"));
        }
        let idx = self
            .policies
            .iter()
            .position(|p| p.tenant == tenant && p.name == name)
            .ok_or_else(|| AdvancedVaultError::NotFound(name.into()))?;
        self.policies.remove(idx);
        Ok(())
    }

    pub fn find(&self, tenant: &str, name: &str) -> Option<&Policy> {
        self.policies.iter().find(|p| p.tenant == tenant && p.name == name)
    }

    pub fn list(&self, tenant: &str) -> Vec<&Policy> {
        let mut out: Vec<&Policy> =
            self.policies.iter().filter(|p| p.tenant == tenant).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn count(&self) -> usize {
        self.policies.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRole {
    pub tenant: String,
    pub role_id: String,
    pub bound_cidrs: Vec<String>,
    pub bind_secret_id: bool,
    pub token_ttl_secs: u64,
    pub max_token_ttl_secs: u64,
    pub policies: Vec<String>,
}

impl AppRole {
    pub fn new(tenant: impl Into<String>, role_id: impl Into<String>) -> Self {
        Self {
            tenant: tenant.into(),
            role_id: role_id.into(),
            bound_cidrs: Vec::new(),
            bind_secret_id: true,
            token_ttl_secs: 3600,
            max_token_ttl_secs: 86400,
            policies: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), AdvancedVaultError> {
        if !self.bind_secret_id && self.bound_cidrs.is_empty() {
            return Err(AdvancedVaultError::ApproleBindRequired);
        }
        if self.token_ttl_secs == 0 {
            return Err(AdvancedVaultError::InvalidTtl(0));
        }
        if self.max_token_ttl_secs < self.token_ttl_secs {
            return Err(AdvancedVaultError::InvalidTtl(self.max_token_ttl_secs));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub id: String,
    pub tenant: String,
    pub mount: String,
    pub issued_unix: u64,
    pub ttl_secs: u64,
    pub renewable: bool,
    pub renewal_count: u32,
    pub revoked: bool,
}

impl Lease {
    pub fn expires_unix(&self) -> u64 {
        self.issued_unix + self.ttl_secs
    }

    pub fn is_active_at(&self, now: u64) -> bool {
        !self.revoked && now < self.expires_unix()
    }
}

#[derive(Debug, Default)]
pub struct LeaseStore {
    leases: Vec<Lease>,
    seq: u64,
}

impl LeaseStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue(
        &mut self,
        tenant: impl Into<String>,
        mount: impl Into<String>,
        now: u64,
        ttl_secs: u64,
        renewable: bool,
    ) -> Result<&Lease, AdvancedVaultError> {
        if ttl_secs == 0 || ttl_secs > 30 * 86400 {
            return Err(AdvancedVaultError::InvalidTtl(ttl_secs));
        }
        self.seq += 1;
        let lease = Lease {
            id: format!("lease-{:08}", self.seq),
            tenant: tenant.into(),
            mount: mount.into(),
            issued_unix: now,
            ttl_secs,
            renewable,
            renewal_count: 0,
            revoked: false,
        };
        self.leases.push(lease);
        Ok(self.leases.last().unwrap())
    }

    pub fn renew(&mut self, id: &str, extra_ttl: u64) -> Result<&Lease, AdvancedVaultError> {
        if extra_ttl == 0 || extra_ttl > 30 * 86400 {
            return Err(AdvancedVaultError::InvalidTtl(extra_ttl));
        }
        let lease = self
            .leases
            .iter_mut()
            .find(|l| l.id == id)
            .ok_or_else(|| AdvancedVaultError::LeaseNotFound(id.into()))?;
        if !lease.renewable {
            return Err(AdvancedVaultError::Forbidden("lease not renewable"));
        }
        if lease.revoked {
            return Err(AdvancedVaultError::LeaseRevoked);
        }
        lease.ttl_secs += extra_ttl;
        lease.renewal_count += 1;
        Ok(&*lease)
    }

    pub fn revoke(&mut self, id: &str) -> Result<(), AdvancedVaultError> {
        let lease = self
            .leases
            .iter_mut()
            .find(|l| l.id == id)
            .ok_or_else(|| AdvancedVaultError::LeaseNotFound(id.into()))?;
        if lease.revoked {
            return Err(AdvancedVaultError::LeaseRevoked);
        }
        lease.revoked = true;
        Ok(())
    }

    pub fn active(&self, tenant: &str, now: u64) -> Vec<&Lease> {
        let mut out: Vec<&Lease> = self
            .leases
            .iter()
            .filter(|l| l.tenant == tenant && l.is_active_at(now))
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn count(&self) -> usize {
        self.leases.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str, caps: &[Capability]) -> PolicyRule {
        PolicyRule {
            path_glob: path.into(),
            capabilities: caps.to_vec(),
        }
    }

    #[test]
    fn validate_policy_name_lowercase_dashes_ok() {
        assert!(validate_policy_name("acme-prod-read").is_ok());
    }

    #[test]
    fn validate_policy_name_empty_rejected() {
        let err = validate_policy_name("").unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidName(_)));
    }

    #[test]
    fn validate_policy_name_uppercase_rejected() {
        let err = validate_policy_name("Acme").unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidName(_)));
    }

    #[test]
    fn validate_policy_name_too_long_rejected() {
        let n = "a".repeat(65);
        let err = validate_policy_name(&n).unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidName(_)));
    }

    #[test]
    fn policy_new_validates_name() {
        let err = Policy::new("BAD", "acme").unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidName(_)));
    }

    #[test]
    fn add_rule_empty_glob_rejected() {
        let mut p = Policy::new("p", "acme").unwrap();
        let err = p.add_rule(rule("", &[Capability::Read])).unwrap_err();
        assert_eq!(err, AdvancedVaultError::EmptyGlob);
    }

    #[test]
    fn add_rule_deny_alone_ok() {
        let mut p = Policy::new("p", "acme").unwrap();
        assert!(p.add_rule(rule("kv/*", &[Capability::Deny])).is_ok());
    }

    #[test]
    fn add_rule_deny_mixed_rejected() {
        let mut p = Policy::new("p", "acme").unwrap();
        let err = p.add_rule(rule("kv/*", &[Capability::Deny, Capability::Read])).unwrap_err();
        assert_eq!(err, AdvancedVaultError::DenyMixed);
    }

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("kv/x", "kv/x"));
        assert!(!glob_match("kv/x", "kv/y"));
    }

    #[test]
    fn glob_match_trailing_star_prefix() {
        assert!(glob_match("kv/*", "kv/anything"));
        assert!(glob_match("kv/*", "kv/"));
        assert!(!glob_match("kv/*", "other/x"));
    }

    #[test]
    fn permits_grants_when_capability_listed() {
        let mut p = Policy::new("p", "acme").unwrap();
        p.add_rule(rule("kv/*", &[Capability::Read, Capability::List])).unwrap();
        assert!(p.permits("kv/secret", Capability::Read));
        assert!(p.permits("kv/secret", Capability::List));
        assert!(!p.permits("kv/secret", Capability::Update));
    }

    #[test]
    fn permits_sudo_grants_all() {
        let mut p = Policy::new("p", "acme").unwrap();
        p.add_rule(rule("kv/*", &[Capability::Sudo])).unwrap();
        assert!(p.permits("kv/secret", Capability::Read));
        assert!(p.permits("kv/secret", Capability::Update));
        assert!(p.permits("kv/secret", Capability::Delete));
    }

    #[test]
    fn permits_deny_overrides() {
        let mut p = Policy::new("p", "acme").unwrap();
        p.add_rule(rule("kv/*", &[Capability::Read])).unwrap();
        p.add_rule(rule("kv/danger/*", &[Capability::Deny])).unwrap();
        assert!(p.permits("kv/safe", Capability::Read));
        assert!(!p.permits("kv/danger/x", Capability::Read));
    }

    #[test]
    fn permits_no_match_is_deny() {
        let p = Policy::new("p", "acme").unwrap();
        assert!(!p.permits("kv/x", Capability::Read));
    }

    #[test]
    fn store_upsert_admin_only() {
        let mut s = PolicyStore::new();
        let p = Policy::new("p", "acme").unwrap();
        let err = s.upsert(ViewPersona::Tenant, p.clone()).unwrap_err();
        assert!(matches!(err, AdvancedVaultError::Forbidden(_)));
    }

    #[test]
    fn store_upsert_admin_succeeds() {
        let mut s = PolicyStore::new();
        let p = Policy::new("p", "acme").unwrap();
        s.upsert(ViewPersona::Admin, p).unwrap();
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn store_upsert_replaces_same_name_tenant() {
        let mut s = PolicyStore::new();
        let mut p1 = Policy::new("p", "acme").unwrap();
        p1.add_rule(rule("kv/*", &[Capability::Read])).unwrap();
        s.upsert(ViewPersona::Admin, p1).unwrap();
        let mut p2 = Policy::new("p", "acme").unwrap();
        p2.add_rule(rule("kv/*", &[Capability::Sudo])).unwrap();
        s.upsert(ViewPersona::Admin, p2).unwrap();
        assert_eq!(s.count(), 1);
        assert!(s.find("acme", "p").unwrap().permits("kv/x", Capability::Update));
    }

    #[test]
    fn store_delete_unknown_errors() {
        let mut s = PolicyStore::new();
        let err = s.delete(ViewPersona::Admin, "acme", "ghost").unwrap_err();
        assert!(matches!(err, AdvancedVaultError::NotFound(_)));
    }

    #[test]
    fn store_delete_succeeds() {
        let mut s = PolicyStore::new();
        s.upsert(ViewPersona::Admin, Policy::new("p", "acme").unwrap()).unwrap();
        s.delete(ViewPersona::Admin, "acme", "p").unwrap();
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn store_list_sorted() {
        let mut s = PolicyStore::new();
        for n in ["zeta", "alpha", "mu"] {
            s.upsert(ViewPersona::Admin, Policy::new(n, "acme").unwrap()).unwrap();
        }
        let names: Vec<&str> = s.list("acme").iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn approle_default_validates() {
        let r = AppRole::new("acme", "ci");
        assert!(r.validate().is_ok());
    }

    #[test]
    fn approle_bind_required() {
        let mut r = AppRole::new("acme", "ci");
        r.bind_secret_id = false;
        let err = r.validate().unwrap_err();
        assert_eq!(err, AdvancedVaultError::ApproleBindRequired);
    }

    #[test]
    fn approle_bind_replaceable_by_cidr() {
        let mut r = AppRole::new("acme", "ci");
        r.bind_secret_id = false;
        r.bound_cidrs = vec!["10.0.0.0/8".into()];
        assert!(r.validate().is_ok());
    }

    #[test]
    fn approle_zero_ttl_invalid() {
        let mut r = AppRole::new("acme", "ci");
        r.token_ttl_secs = 0;
        let err = r.validate().unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidTtl(_)));
    }

    #[test]
    fn approle_max_below_default_invalid() {
        let mut r = AppRole::new("acme", "ci");
        r.token_ttl_secs = 1000;
        r.max_token_ttl_secs = 500;
        let err = r.validate().unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidTtl(_)));
    }

    #[test]
    fn lease_issue_assigns_id() {
        let mut s = LeaseStore::new();
        let l = s.issue("acme", "kv", 1000, 3600, true).unwrap();
        assert!(l.id.starts_with("lease-"));
        assert_eq!(l.ttl_secs, 3600);
    }

    #[test]
    fn lease_issue_zero_ttl_rejected() {
        let mut s = LeaseStore::new();
        let err = s.issue("acme", "kv", 0, 0, false).unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidTtl(0)));
    }

    #[test]
    fn lease_issue_huge_ttl_rejected() {
        let mut s = LeaseStore::new();
        let err = s.issue("acme", "kv", 0, 60 * 86400, false).unwrap_err();
        assert!(matches!(err, AdvancedVaultError::InvalidTtl(_)));
    }

    #[test]
    fn lease_renew_extends_ttl() {
        let mut s = LeaseStore::new();
        let id = s.issue("acme", "kv", 1000, 3600, true).unwrap().id.clone();
        let l = s.renew(&id, 600).unwrap();
        assert_eq!(l.ttl_secs, 4200);
        assert_eq!(l.renewal_count, 1);
    }

    #[test]
    fn lease_renew_non_renewable_rejected() {
        let mut s = LeaseStore::new();
        let id = s.issue("acme", "kv", 1000, 3600, false).unwrap().id.clone();
        let err = s.renew(&id, 600).unwrap_err();
        assert!(matches!(err, AdvancedVaultError::Forbidden(_)));
    }

    #[test]
    fn lease_renew_unknown() {
        let mut s = LeaseStore::new();
        let err = s.renew("ghost", 600).unwrap_err();
        assert!(matches!(err, AdvancedVaultError::LeaseNotFound(_)));
    }

    #[test]
    fn lease_renew_revoked_rejected() {
        let mut s = LeaseStore::new();
        let id = s.issue("acme", "kv", 1000, 3600, true).unwrap().id.clone();
        s.revoke(&id).unwrap();
        let err = s.renew(&id, 600).unwrap_err();
        assert_eq!(err, AdvancedVaultError::LeaseRevoked);
    }

    #[test]
    fn lease_revoke_twice_errors() {
        let mut s = LeaseStore::new();
        let id = s.issue("acme", "kv", 1000, 3600, true).unwrap().id.clone();
        s.revoke(&id).unwrap();
        let err = s.revoke(&id).unwrap_err();
        assert_eq!(err, AdvancedVaultError::LeaseRevoked);
    }

    #[test]
    fn lease_active_filters_by_time() {
        let mut s = LeaseStore::new();
        s.issue("acme", "kv", 1000, 100, false).unwrap();
        s.issue("acme", "kv", 1000, 200, false).unwrap();
        let active = s.active("acme", 1150);
        assert_eq!(active.len(), 1); // first expired at 1100, second at 1200
    }

    #[test]
    fn lease_active_filters_revoked() {
        let mut s = LeaseStore::new();
        let id = s.issue("acme", "kv", 1000, 100, false).unwrap().id.clone();
        s.revoke(&id).unwrap();
        assert!(s.active("acme", 1050).is_empty());
    }

    #[test]
    fn lease_active_filters_by_tenant() {
        let mut s = LeaseStore::new();
        s.issue("acme", "kv", 1000, 100, false).unwrap();
        s.issue("globex", "kv", 1000, 100, false).unwrap();
        assert_eq!(s.active("acme", 1050).len(), 1);
    }

    #[test]
    fn lease_expires_at_calculates() {
        let l = Lease {
            id: "x".into(),
            tenant: "t".into(),
            mount: "kv".into(),
            issued_unix: 1000,
            ttl_secs: 200,
            renewable: false,
            renewal_count: 0,
            revoked: false,
        };
        assert_eq!(l.expires_unix(), 1200);
    }

    #[test]
    fn lease_round_trips_json() {
        let l = Lease {
            id: "x".into(),
            tenant: "t".into(),
            mount: "kv".into(),
            issued_unix: 1000,
            ttl_secs: 200,
            renewable: true,
            renewal_count: 2,
            revoked: false,
        };
        let s = serde_json::to_string(&l).unwrap();
        let back: Lease = serde_json::from_str(&s).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn capability_serializes_snake_case() {
        let s = serde_json::to_string(&Capability::Sudo).unwrap();
        assert_eq!(s, "\"sudo\"");
    }
}
