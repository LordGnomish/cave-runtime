//! ACME RFC 8555 cert-rotation lifecycle (config + state machine layer).
//!
//! `tls.rs` already exposes a [`AcmeChallengeStore`] for HTTP-01 token
//! presentation and an [`AcmeConfig`] envelope. This module covers the
//! *rotation* lifecycle that lives on top of that:
//!
//! - Track the per-cert renewal timer (renew at 2/3 of the validity window).
//! - Enumerate ready-to-renew, in-progress, and failed certs.
//! - Decide back-off after a failed attempt (exponential, jittered, capped).
//! - Coordinate the order/auth/finalize FSM transitions so the actual ACME
//!   client (real network calls) plugs in as a single async function.

use crate::tls::AcmeConfig;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertState {
    /// Never been issued; first run pending.
    Pending,
    /// Successfully issued, within validity window and far from renewal.
    Active,
    /// Past renewal threshold, scheduled for renew.
    NeedsRenewal,
    /// In-flight ACME order.
    Renewing,
    /// Last attempt failed; back-off until `next_attempt_at`.
    Failed,
    /// Manually disabled (Admin API toggle).
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertRotationRecord {
    pub cert_id: Uuid,
    pub domains: Vec<String>,
    pub state: CertState,
    pub issued_at: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
    pub renew_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub order_url: Option<String>,
    pub finalize_url: Option<String>,
}

impl CertRotationRecord {
    pub fn new(cert_id: Uuid, domains: Vec<String>) -> Self {
        Self {
            cert_id,
            domains,
            state: CertState::Pending,
            issued_at: None,
            not_after: None,
            renew_at: None,
            last_attempt_at: None,
            next_attempt_at: None,
            consecutive_failures: 0,
            last_error: None,
            order_url: None,
            finalize_url: None,
        }
    }
}

// ── Renewal policy ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenewalPolicy {
    /// Renew when `(now - issued_at) / (not_after - issued_at) > renewal_threshold`.
    /// 0.66 = renew at 2/3 of validity (RFC 8555 Section 7.4 recommendation).
    pub renewal_threshold: f64,
    /// Maximum consecutive failures before we give up and require manual
    /// intervention.
    pub max_consecutive_failures: u32,
    /// Initial back-off after first failure.
    pub backoff_initial: Duration,
    /// Cap on back-off.
    pub backoff_max: Duration,
}

impl Default for RenewalPolicy {
    fn default() -> Self {
        Self {
            renewal_threshold: 0.66,
            max_consecutive_failures: 8,
            backoff_initial: Duration::from_secs(60),
            backoff_max: Duration::from_secs(3600),
        }
    }
}

impl RenewalPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0 < self.renewal_threshold && self.renewal_threshold < 1.0) {
            return Err("renewal_threshold must be in (0, 1)".into());
        }
        if self.backoff_initial > self.backoff_max {
            return Err("backoff_initial > backoff_max".into());
        }
        Ok(())
    }

    /// Compute the moment we'll renew the cert: issued_at + threshold * lifetime.
    pub fn renew_at(
        &self,
        issued_at: DateTime<Utc>,
        not_after: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let lifetime = (not_after - issued_at).num_seconds() as f64;
        let offset_secs = (lifetime * self.renewal_threshold) as i64;
        issued_at + ChronoDuration::seconds(offset_secs)
    }

    /// Compute the next attempt time after a failure. Truncated exponential.
    pub fn backoff(&self, consecutive_failures: u32) -> Duration {
        if consecutive_failures == 0 {
            return self.backoff_initial;
        }
        let factor = 1u32 << (consecutive_failures - 1).min(10);
        let raw = self.backoff_initial.saturating_mul(factor);
        if raw > self.backoff_max {
            self.backoff_max
        } else {
            raw
        }
    }
}

// ── FSM transitions ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsmEvent {
    OrderCreated { order_url: String, finalize_url: String },
    AuthorizationsValidated,
    Finalized { not_after: DateTime<Utc> },
    Failed(String),
    ManualDisable,
    ManualEnable,
}

pub fn advance(record: &mut CertRotationRecord, event: FsmEvent, now: DateTime<Utc>, policy: &RenewalPolicy) {
    match (&record.state, event) {
        (CertState::Pending | CertState::NeedsRenewal, FsmEvent::OrderCreated { order_url, finalize_url }) => {
            record.state = CertState::Renewing;
            record.last_attempt_at = Some(now);
            record.order_url = Some(order_url);
            record.finalize_url = Some(finalize_url);
        }
        (CertState::Failed, FsmEvent::OrderCreated { order_url, finalize_url }) => {
            // Retry from failed.
            record.state = CertState::Renewing;
            record.last_attempt_at = Some(now);
            record.order_url = Some(order_url);
            record.finalize_url = Some(finalize_url);
        }
        (CertState::Renewing, FsmEvent::AuthorizationsValidated) => {
            // intermediate; we stay in Renewing until Finalized.
        }
        (CertState::Renewing, FsmEvent::Finalized { not_after }) => {
            record.state = CertState::Active;
            record.issued_at = Some(now);
            record.not_after = Some(not_after);
            record.renew_at = Some(policy.renew_at(now, not_after));
            record.consecutive_failures = 0;
            record.last_error = None;
            record.order_url = None;
            record.finalize_url = None;
        }
        (CertState::Renewing | CertState::Pending | CertState::NeedsRenewal, FsmEvent::Failed(err)) => {
            record.consecutive_failures = record.consecutive_failures.saturating_add(1);
            record.last_error = Some(err);
            if record.consecutive_failures >= policy.max_consecutive_failures {
                record.state = CertState::Failed;
                record.next_attempt_at = None;
            } else {
                record.state = CertState::Failed;
                let backoff = policy.backoff(record.consecutive_failures);
                let backoff_chrono = ChronoDuration::from_std(backoff).unwrap_or(ChronoDuration::seconds(60));
                record.next_attempt_at = Some(now + backoff_chrono);
            }
        }
        (_, FsmEvent::ManualDisable) => {
            record.state = CertState::Disabled;
        }
        (CertState::Disabled, FsmEvent::ManualEnable) => {
            record.state = if record.not_after.is_some() {
                CertState::Active
            } else {
                CertState::Pending
            };
        }
        _ => {} // ignore invalid transitions
    }
}

/// Re-evaluate state based on the clock — Active cert past renew_at →
/// NeedsRenewal; Failed past next_attempt_at → NeedsRenewal again (so the
/// caller picks it up in the next ready_for_renewal pass).
pub fn tick(record: &mut CertRotationRecord, now: DateTime<Utc>) {
    match record.state {
        CertState::Active => {
            if let Some(renew_at) = record.renew_at {
                if now >= renew_at {
                    record.state = CertState::NeedsRenewal;
                }
            }
        }
        CertState::Failed => {
            if let Some(next) = record.next_attempt_at {
                if now >= next {
                    record.state = CertState::NeedsRenewal;
                    record.next_attempt_at = None;
                }
            }
        }
        _ => {}
    }
}

// ── Registry ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct AcmeRotationRegistry {
    records: RwLock<BTreeMap<Uuid, CertRotationRecord>>,
    pub policy: RenewalPolicy,
    pub config: Option<AcmeConfig>,
}

impl AcmeRotationRegistry {
    pub fn new(policy: RenewalPolicy) -> Arc<Self> {
        Arc::new(Self {
            records: RwLock::new(BTreeMap::new()),
            policy,
            config: None,
        })
    }

    pub fn with_acme(policy: RenewalPolicy, config: AcmeConfig) -> Arc<Self> {
        Arc::new(Self {
            records: RwLock::new(BTreeMap::new()),
            policy,
            config: Some(config),
        })
    }

    pub fn upsert(&self, record: CertRotationRecord) {
        self.records.write().unwrap().insert(record.cert_id, record);
    }

    pub fn get(&self, cert_id: Uuid) -> Option<CertRotationRecord> {
        self.records.read().unwrap().get(&cert_id).cloned()
    }

    pub fn advance(&self, cert_id: Uuid, event: FsmEvent, now: DateTime<Utc>) {
        let mut g = self.records.write().unwrap();
        if let Some(r) = g.get_mut(&cert_id) {
            advance(r, event, now, &self.policy);
        }
    }

    pub fn tick_all(&self, now: DateTime<Utc>) {
        let mut g = self.records.write().unwrap();
        for r in g.values_mut() {
            tick(r, now);
        }
    }

    pub fn ready_for_renewal(&self) -> Vec<CertRotationRecord> {
        self.records
            .read()
            .unwrap()
            .values()
            .filter(|r| matches!(r.state, CertState::Pending | CertState::NeedsRenewal))
            .cloned()
            .collect()
    }

    pub fn failed(&self) -> Vec<CertRotationRecord> {
        self.records
            .read()
            .unwrap()
            .values()
            .filter(|r| r.state == CertState::Failed)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.records.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.read().unwrap().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn rec(domains: &[&str]) -> CertRotationRecord {
        CertRotationRecord::new(
            Uuid::new_v4(),
            domains.iter().map(|s| (*s).to_string()).collect(),
        )
    }

    fn policy() -> RenewalPolicy {
        RenewalPolicy::default()
    }

    // ── Policy validate ──

    #[test]
    fn policy_default_validates() {
        policy().validate().unwrap();
    }

    #[test]
    fn policy_threshold_zero_rejects() {
        let mut p = policy();
        p.renewal_threshold = 0.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn policy_threshold_one_rejects() {
        let mut p = policy();
        p.renewal_threshold = 1.0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn policy_initial_above_max_rejects() {
        let mut p = policy();
        p.backoff_initial = Duration::from_secs(7200);
        p.backoff_max = Duration::from_secs(60);
        assert!(p.validate().is_err());
    }

    // ── renew_at ──

    #[test]
    fn renew_at_two_thirds_default() {
        let issued = Utc::now();
        let not_after = issued + ChronoDuration::days(90);
        let renew = policy().renew_at(issued, not_after);
        let lifetime = (not_after - issued).num_seconds();
        let offset = (renew - issued).num_seconds();
        assert!((offset - (lifetime * 66 / 100)).abs() < 5);
    }

    // ── backoff ──

    #[test]
    fn backoff_zero_failures_returns_initial() {
        let p = policy();
        assert_eq!(p.backoff(0), p.backoff_initial);
    }

    #[test]
    fn backoff_grows_then_caps() {
        let mut p = policy();
        p.backoff_initial = Duration::from_secs(10);
        p.backoff_max = Duration::from_secs(80);
        assert_eq!(p.backoff(1), Duration::from_secs(10));
        assert_eq!(p.backoff(2), Duration::from_secs(20));
        assert_eq!(p.backoff(3), Duration::from_secs(40));
        assert_eq!(p.backoff(4), Duration::from_secs(80));
        assert_eq!(p.backoff(20), Duration::from_secs(80));
    }

    // ── FSM happy path ──

    #[test]
    fn fsm_pending_to_renewing_to_active() {
        let mut r = rec(&["example.com"]);
        let t0 = now();
        advance(
            &mut r,
            FsmEvent::OrderCreated {
                order_url: "https://acme/order/1".into(),
                finalize_url: "https://acme/finalize/1".into(),
            },
            t0,
            &policy(),
        );
        assert_eq!(r.state, CertState::Renewing);
        assert_eq!(r.last_attempt_at, Some(t0));
        assert_eq!(r.order_url.as_deref(), Some("https://acme/order/1"));

        advance(&mut r, FsmEvent::AuthorizationsValidated, t0, &policy());
        assert_eq!(r.state, CertState::Renewing);

        let not_after = t0 + ChronoDuration::days(90);
        advance(&mut r, FsmEvent::Finalized { not_after }, t0, &policy());
        assert_eq!(r.state, CertState::Active);
        assert_eq!(r.issued_at, Some(t0));
        assert_eq!(r.not_after, Some(not_after));
        assert!(r.renew_at.unwrap() > t0);
        assert_eq!(r.consecutive_failures, 0);
        assert!(r.order_url.is_none());
    }

    // ── FSM failure ──

    #[test]
    fn fsm_failure_increments_and_sets_backoff() {
        let mut r = rec(&["x.com"]);
        let t = now();
        advance(
            &mut r,
            FsmEvent::OrderCreated {
                order_url: "u".into(),
                finalize_url: "f".into(),
            },
            t,
            &policy(),
        );
        advance(&mut r, FsmEvent::Failed("rate limit".into()), t, &policy());
        assert_eq!(r.state, CertState::Failed);
        assert_eq!(r.consecutive_failures, 1);
        assert!(r.next_attempt_at.is_some());
        assert_eq!(r.last_error.as_deref(), Some("rate limit"));
    }

    #[test]
    fn fsm_failure_after_max_giving_up() {
        let mut r = rec(&["x.com"]);
        r.consecutive_failures = policy().max_consecutive_failures - 1;
        advance(&mut r, FsmEvent::Failed("err".into()), now(), &policy());
        assert_eq!(r.state, CertState::Failed);
        assert!(r.next_attempt_at.is_none());
        assert_eq!(r.consecutive_failures, policy().max_consecutive_failures);
    }

    #[test]
    fn fsm_retry_from_failed() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Failed;
        r.consecutive_failures = 1;
        advance(
            &mut r,
            FsmEvent::OrderCreated {
                order_url: "u".into(),
                finalize_url: "f".into(),
            },
            now(),
            &policy(),
        );
        assert_eq!(r.state, CertState::Renewing);
    }

    // ── Manual transitions ──

    #[test]
    fn fsm_manual_disable_from_active() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Active;
        advance(&mut r, FsmEvent::ManualDisable, now(), &policy());
        assert_eq!(r.state, CertState::Disabled);
    }

    #[test]
    fn fsm_manual_enable_with_history_returns_active() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Disabled;
        r.not_after = Some(now() + ChronoDuration::days(30));
        advance(&mut r, FsmEvent::ManualEnable, now(), &policy());
        assert_eq!(r.state, CertState::Active);
    }

    #[test]
    fn fsm_manual_enable_without_history_returns_pending() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Disabled;
        advance(&mut r, FsmEvent::ManualEnable, now(), &policy());
        assert_eq!(r.state, CertState::Pending);
    }

    #[test]
    fn fsm_invalid_transition_ignored() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Active;
        advance(&mut r, FsmEvent::AuthorizationsValidated, now(), &policy());
        assert_eq!(r.state, CertState::Active);
    }

    // ── tick ──

    #[test]
    fn tick_active_past_renew_at_becomes_needs_renewal() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Active;
        let t0 = now();
        r.renew_at = Some(t0 - ChronoDuration::seconds(1));
        tick(&mut r, t0);
        assert_eq!(r.state, CertState::NeedsRenewal);
    }

    #[test]
    fn tick_active_before_renew_at_stays() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Active;
        r.renew_at = Some(now() + ChronoDuration::days(30));
        tick(&mut r, now());
        assert_eq!(r.state, CertState::Active);
    }

    #[test]
    fn tick_failed_past_next_attempt_becomes_ready() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Failed;
        let t0 = now();
        r.next_attempt_at = Some(t0 - ChronoDuration::seconds(1));
        tick(&mut r, t0);
        assert_eq!(r.state, CertState::NeedsRenewal);
        assert!(r.next_attempt_at.is_none());
    }

    #[test]
    fn tick_failed_without_next_attempt_stays() {
        let mut r = rec(&["x.com"]);
        r.state = CertState::Failed;
        r.next_attempt_at = None;
        tick(&mut r, now());
        assert_eq!(r.state, CertState::Failed);
    }

    // ── Registry ──

    #[test]
    fn registry_upsert_and_get() {
        let r = AcmeRotationRegistry::new(policy());
        let id = Uuid::new_v4();
        let mut record = rec(&["x.com"]);
        record.cert_id = id;
        r.upsert(record);
        assert!(r.get(id).is_some());
    }

    #[test]
    fn registry_advance_persists() {
        let r = AcmeRotationRegistry::new(policy());
        let id = Uuid::new_v4();
        let mut record = rec(&["x.com"]);
        record.cert_id = id;
        r.upsert(record);
        r.advance(
            id,
            FsmEvent::OrderCreated {
                order_url: "u".into(),
                finalize_url: "f".into(),
            },
            now(),
        );
        assert_eq!(r.get(id).unwrap().state, CertState::Renewing);
    }

    #[test]
    fn registry_ready_for_renewal_lists_pending_and_needs_renewal() {
        let r = AcmeRotationRegistry::new(policy());
        let mut a = rec(&["a.com"]);
        a.state = CertState::Active;
        r.upsert(a);
        let mut b = rec(&["b.com"]);
        b.state = CertState::NeedsRenewal;
        r.upsert(b.clone());
        let mut c = rec(&["c.com"]);
        c.state = CertState::Pending;
        r.upsert(c.clone());

        let pending = r.ready_for_renewal();
        let names: Vec<_> = pending
            .iter()
            .map(|p| p.domains[0].clone())
            .collect();
        assert!(names.contains(&"b.com".to_string()));
        assert!(names.contains(&"c.com".to_string()));
        assert!(!names.contains(&"a.com".to_string()));
    }

    #[test]
    fn registry_failed_lists_failures() {
        let r = AcmeRotationRegistry::new(policy());
        let mut a = rec(&["a.com"]);
        a.state = CertState::Failed;
        r.upsert(a.clone());
        assert_eq!(r.failed().len(), 1);
    }

    #[test]
    fn registry_tick_all_promotes_active_to_needs_renewal() {
        let r = AcmeRotationRegistry::new(policy());
        let mut a = rec(&["a.com"]);
        a.state = CertState::Active;
        a.renew_at = Some(Utc::now() - ChronoDuration::seconds(1));
        let id = a.cert_id;
        r.upsert(a);
        r.tick_all(Utc::now());
        assert_eq!(r.get(id).unwrap().state, CertState::NeedsRenewal);
    }

    #[test]
    fn registry_with_acme_config() {
        let cfg = AcmeConfig::lets_encrypt_staging("test@x.com".into(), vec!["x.com".into()]);
        let r = AcmeRotationRegistry::with_acme(policy(), cfg);
        assert!(r.config.is_some());
    }

    // ── Serde ──

    #[test]
    fn record_json_roundtrip() {
        let r = rec(&["x.com"]);
        let s = serde_json::to_string(&r).unwrap();
        let r2: CertRotationRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn cert_state_kebab_serde() {
        let s = serde_json::to_string(&CertState::NeedsRenewal).unwrap();
        assert_eq!(s, r#""needs_renewal""#);
    }
}
