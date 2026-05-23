// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Password policy, brute-force detection, and conditional access.
//!
//! Upstream:
//!   * `services/src/main/java/org/keycloak/policy/*` (PasswordPolicyProvider)
//!   * `services/src/main/java/org/keycloak/services/managers/DefaultBruteForceProtector.java`
//!   * `services/src/main/java/org/keycloak/authentication/authenticators/conditional/*`

use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::error::{KeycloakError, Result};
use crate::models::PasswordPolicy;

/// Validate a candidate password against the realm's password policy.
/// Returns `PasswordPolicyViolation(reason)` on the first rule that fails.
pub fn check_password_policy(pwd: &str, policy: &PasswordPolicy) -> Result<()> {
    let len = pwd.chars().count();
    if (len as u8) < policy.min_length {
        return Err(KeycloakError::PasswordPolicyViolation(format!(
            "min-length: {} < {}",
            len, policy.min_length
        )));
    }
    let mut up = 0u8;
    let mut lo = 0u8;
    let mut di = 0u8;
    let mut sp = 0u8;
    for c in pwd.chars() {
        if c.is_ascii_uppercase() {
            up = up.saturating_add(1);
        } else if c.is_ascii_lowercase() {
            lo = lo.saturating_add(1);
        } else if c.is_ascii_digit() {
            di = di.saturating_add(1);
        } else if !c.is_alphanumeric() {
            sp = sp.saturating_add(1);
        }
    }
    if up < policy.require_uppercase {
        return Err(KeycloakError::PasswordPolicyViolation(format!(
            "upper-case: {} < {}",
            up, policy.require_uppercase
        )));
    }
    if lo < policy.require_lowercase {
        return Err(KeycloakError::PasswordPolicyViolation(format!(
            "lower-case: {} < {}",
            lo, policy.require_lowercase
        )));
    }
    if di < policy.require_digit {
        return Err(KeycloakError::PasswordPolicyViolation(format!(
            "digit: {} < {}",
            di, policy.require_digit
        )));
    }
    if sp < policy.require_special {
        return Err(KeycloakError::PasswordPolicyViolation(format!(
            "special: {} < {}",
            sp, policy.require_special
        )));
    }
    Ok(())
}

/// Brute-force tracker — windowed failure counter per (realm_id, user_id)
/// or per IP for unauthenticated paths. After `max_failures` within
/// `window`, the account is locked for `lockout`.
pub struct BruteForceTracker {
    inner: Mutex<BruteForceInner>,
    pub max_failures: u32,
    pub window: Duration,
    pub lockout: Duration,
}

struct BruteForceInner {
    failures: BTreeMap<String, Vec<DateTime<Utc>>>,
    lockout_until: BTreeMap<String, DateTime<Utc>>,
}

impl Default for BruteForceTracker {
    fn default() -> Self {
        Self::new(5, Duration::seconds(60), Duration::seconds(300))
    }
}

impl BruteForceTracker {
    pub fn new(max_failures: u32, window: Duration, lockout: Duration) -> Self {
        Self {
            inner: Mutex::new(BruteForceInner {
                failures: BTreeMap::new(),
                lockout_until: BTreeMap::new(),
            }),
            max_failures,
            window,
            lockout,
        }
    }

    /// Record a failed login attempt and lock the key if the threshold is
    /// crossed. Returns `CredentialLocked` if the account is now in lockout.
    pub fn record_failure(&self, key: &str) -> Result<()> {
        let now = Utc::now();
        let mut g = self.inner.lock().unwrap();
        let entry = g.failures.entry(key.to_string()).or_default();
        entry.push(now);
        entry.retain(|&t| now - t <= self.window);
        let crossed = entry.len() as u32 >= self.max_failures;
        if crossed {
            entry.clear();
            let until = now + self.lockout;
            g.lockout_until.insert(key.to_string(), until);
            return Err(KeycloakError::CredentialLocked {
                account_id: key.to_string(),
                retry_after_seconds: self.lockout.num_seconds() as u64,
            });
        }
        Ok(())
    }

    /// Reset failure counter — call on a successful login.
    pub fn record_success(&self, key: &str) {
        let mut g = self.inner.lock().unwrap();
        g.failures.remove(key);
        g.lockout_until.remove(key);
    }

    /// Returns `CredentialLocked` if the account is currently locked out.
    pub fn check(&self, key: &str) -> Result<()> {
        let now = Utc::now();
        let mut g = self.inner.lock().unwrap();
        if let Some(&until) = g.lockout_until.get(key) {
            if now < until {
                let remaining = (until - now).num_seconds().max(0) as u64;
                return Err(KeycloakError::CredentialLocked {
                    account_id: key.to_string(),
                    retry_after_seconds: remaining,
                });
            } else {
                g.lockout_until.remove(key);
            }
        }
        Ok(())
    }

    pub fn failure_count(&self, key: &str) -> usize {
        let g = self.inner.lock().unwrap();
        g.failures.get(key).map(|v| v.len()).unwrap_or(0)
    }
}

/// Conditional access rule. Matched against a request `ConditionalContext`
/// to allow / deny / require step-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalRule {
    /// Require MFA when the source IP is outside the listed CIDR prefixes.
    RequireMfaUnlessIp { allow_prefixes: Vec<String> },
    /// Deny when the client is on the deny list.
    DenyClient { client_id: String },
    /// Require a fresh re-auth no older than the given number of seconds.
    RequireReAuthMaxAge { max_age_seconds: u64 },
}

#[derive(Debug, Clone)]
pub struct ConditionalContext {
    pub client_id: String,
    pub ip_address: String,
    pub last_auth_age_seconds: u64,
    pub mfa_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    Allow,
    Deny(String),
    StepUp(String),
}

pub fn evaluate(rules: &[ConditionalRule], ctx: &ConditionalContext) -> AccessDecision {
    for r in rules {
        match r {
            ConditionalRule::DenyClient { client_id } if client_id == &ctx.client_id => {
                return AccessDecision::Deny(format!("client denied: {}", client_id));
            }
            ConditionalRule::RequireMfaUnlessIp { allow_prefixes } => {
                let in_allow = allow_prefixes.iter().any(|p| ctx.ip_address.starts_with(p));
                if !in_allow && !ctx.mfa_present {
                    return AccessDecision::StepUp("mfa-required".into());
                }
            }
            ConditionalRule::RequireReAuthMaxAge { max_age_seconds } => {
                if ctx.last_auth_age_seconds > *max_age_seconds {
                    return AccessDecision::StepUp("re-auth-required".into());
                }
            }
            _ => {}
        }
    }
    AccessDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_length_violation_reported() {
        let p = PasswordPolicy::default();
        assert!(check_password_policy("short", &p).is_err());
    }

    #[test]
    fn complex_policy_enforced() {
        let p = PasswordPolicy {
            min_length: 10,
            require_uppercase: 1,
            require_lowercase: 1,
            require_digit: 1,
            require_special: 1,
            history_count: 0,
            hash_algorithm: crate::models::HashAlgorithm::Pbkdf2Sha512,
            hash_iterations: 100_000,
        };
        assert!(check_password_policy("Abcdef1!ghij", &p).is_ok());
        assert!(check_password_policy("abcdefghij1!", &p).is_err()); // no upper
        assert!(check_password_policy("ABCDEFGHIJ1!", &p).is_err()); // no lower
        assert!(check_password_policy("Abcdefghij!", &p).is_err()); // no digit
        assert!(check_password_policy("Abcdefghij1", &p).is_err()); // no special
    }

    #[test]
    fn brute_force_locks_after_threshold() {
        let t = BruteForceTracker::new(3, Duration::seconds(60), Duration::seconds(30));
        assert!(t.record_failure("u1").is_ok());
        assert!(t.record_failure("u1").is_ok());
        let err = t.record_failure("u1").unwrap_err();
        match err {
            KeycloakError::CredentialLocked { account_id, retry_after_seconds } => {
                assert_eq!(account_id, "u1");
                assert!(retry_after_seconds >= 1);
            }
            _ => panic!("expected CredentialLocked, got {:?}", err),
        }
        assert!(t.check("u1").is_err());
    }

    #[test]
    fn brute_force_success_clears_state() {
        let t = BruteForceTracker::new(3, Duration::seconds(60), Duration::seconds(30));
        let _ = t.record_failure("u1");
        let _ = t.record_failure("u1");
        t.record_success("u1");
        assert_eq!(t.failure_count("u1"), 0);
        assert!(t.check("u1").is_ok());
    }

    #[test]
    fn conditional_deny_client_short_circuits() {
        let rules = vec![ConditionalRule::DenyClient { client_id: "evil".into() }];
        let ctx = ConditionalContext {
            client_id: "evil".into(),
            ip_address: "10.0.0.1".into(),
            last_auth_age_seconds: 0,
            mfa_present: true,
        };
        assert!(matches!(evaluate(&rules, &ctx), AccessDecision::Deny(_)));
    }

    #[test]
    fn conditional_step_up_when_no_mfa_outside_allow_ip() {
        let rules = vec![ConditionalRule::RequireMfaUnlessIp {
            allow_prefixes: vec!["10.".into()],
        }];
        let ctx_outside = ConditionalContext {
            client_id: "spa".into(),
            ip_address: "203.0.113.1".into(),
            last_auth_age_seconds: 0,
            mfa_present: false,
        };
        let ctx_inside = ConditionalContext {
            client_id: "spa".into(),
            ip_address: "10.1.2.3".into(),
            last_auth_age_seconds: 0,
            mfa_present: false,
        };
        assert!(matches!(evaluate(&rules, &ctx_outside), AccessDecision::StepUp(_)));
        assert_eq!(evaluate(&rules, &ctx_inside), AccessDecision::Allow);
    }

    #[test]
    fn conditional_reauth_step_up_when_stale() {
        let rules = vec![ConditionalRule::RequireReAuthMaxAge { max_age_seconds: 300 }];
        let stale = ConditionalContext {
            client_id: "spa".into(),
            ip_address: "10.0.0.1".into(),
            last_auth_age_seconds: 999,
            mfa_present: true,
        };
        let fresh = ConditionalContext {
            client_id: "spa".into(),
            ip_address: "10.0.0.1".into(),
            last_auth_age_seconds: 60,
            mfa_present: true,
        };
        assert!(matches!(evaluate(&rules, &stale), AccessDecision::StepUp(_)));
        assert_eq!(evaluate(&rules, &fresh), AccessDecision::Allow);
    }
}
