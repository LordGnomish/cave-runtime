// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Brute-force login protection.
//!
//! Line-ported from Keycloak (Apache-2.0):
//!   `services/.../managers/DefaultBruteForceProtector.java` — the `failure(...)`
//!   computation, `isTemporarilyDisabled` check, and `permanentUserLockOut` trigger.
//!
//! The Java code threads its state through `UserLoginFailureModel` and the
//! `KeycloakSession`; here the per-user state is the plain [`LoginFailureRecord`]
//! the caller owns, making the algorithm a pure, deterministic function of
//! `(config, record, failure_time)`.

/// Wait-time growth strategy — mirrors `RealmRepresentation.BruteForceStrategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BruteForceStrategy {
    /// `waitIncrement * (numFailures / failureFactor)` (integer division).
    Multiple,
    /// `waitIncrement * (1 + numFailures - failureFactor)`.
    Linear,
}

/// Realm-level brute-force configuration (the relevant `RealmModel` getters).
#[derive(Debug, Clone)]
pub struct BruteForceConfig {
    pub wait_increment_seconds: u64,
    pub failure_factor: u32,
    pub max_failure_wait_seconds: u64,
    pub max_delta_time_seconds: u64,
    pub quick_login_check_millis: u64,
    pub minimum_quick_login_wait_seconds: u64,
    pub strategy: BruteForceStrategy,
    pub permanent_lockout: bool,
    pub max_temporary_lockouts: u32,
}

impl Default for BruteForceConfig {
    /// Keycloak realm defaults.
    fn default() -> Self {
        Self {
            wait_increment_seconds: 60,
            failure_factor: 30,
            max_failure_wait_seconds: 900,
            max_delta_time_seconds: 60 * 60 * 12,
            quick_login_check_millis: 1000,
            minimum_quick_login_wait_seconds: 60,
            strategy: BruteForceStrategy::Multiple,
            permanent_lockout: false,
            max_temporary_lockouts: 0,
        }
    }
}

/// Per-user login-failure state — the port of `UserLoginFailureModel`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginFailureRecord {
    pub num_failures: u32,
    /// Time (millis) of the last recorded failure; `0` means "never".
    pub last_failure: i64,
    /// Epoch-seconds before which the account is temporarily disabled.
    pub failed_login_not_before: i64,
    pub num_temporary_lockouts: u32,
    pub permanently_locked: bool,
}

impl LoginFailureRecord {
    fn clear_failures(&mut self) {
        self.num_failures = 0;
        self.failed_login_not_before = 0;
        self.num_temporary_lockouts = 0;
    }
}

/// Stateless protector — owns only the realm config.
#[derive(Debug, Clone)]
pub struct BruteForceProtector {
    config: BruteForceConfig,
}

impl BruteForceProtector {
    pub fn new(config: BruteForceConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &BruteForceConfig {
        &self.config
    }

    /// Port of `DefaultBruteForceProtector.failure(...)`.
    ///
    /// `failure_time` is in milliseconds (Keycloak's `Time.currentTimeMillis()`).
    pub fn record_failure(&self, r: &mut LoginFailureRecord, failure_time: i64) {
        let c = &self.config;
        let permanent_zero = c.permanent_lockout && c.max_temporary_lockouts == 0;

        let last = r.last_failure;
        let delta_time = if last > 0 { failure_time - last } else { 0 };
        r.last_failure = failure_time;

        // If the last failure was more than max-delta ago, clear the counters.
        if !permanent_zero && delta_time > 0 && delta_time > c.max_delta_time_seconds as i64 * 1000 {
            r.clear_failures();
        }
        r.num_failures += 1;

        let mut wait_seconds: i64 = 0;
        if !permanent_zero {
            wait_seconds = match c.strategy {
                BruteForceStrategy::Multiple => {
                    c.wait_increment_seconds as i64 * (r.num_failures / c.failure_factor) as i64
                }
                BruteForceStrategy::Linear => {
                    c.wait_increment_seconds as i64
                        * (1 + r.num_failures as i64 - c.failure_factor as i64)
                }
            };
        }

        let mut quick_login_failure = false;
        if wait_seconds <= 0
            && last > 0
            && delta_time < c.quick_login_check_millis as i64
        {
            wait_seconds = c.minimum_quick_login_wait_seconds as i64;
            quick_login_failure = true;
        }

        if wait_seconds > 0 {
            if !c.permanent_lockout || c.max_temporary_lockouts > 0 {
                wait_seconds = wait_seconds.min(c.max_failure_wait_seconds as i64);
            }
            if !quick_login_failure {
                r.num_temporary_lockouts += 1;
            }
            if quick_login_failure
                || !c.permanent_lockout
                || r.num_temporary_lockouts <= c.max_temporary_lockouts
            {
                let not_before = failure_time / 1000 + wait_seconds;
                r.failed_login_not_before = not_before.min(i32::MAX as i64);
            }
        }

        if !c.permanent_lockout {
            return;
        }
        if r.num_temporary_lockouts > c.max_temporary_lockouts
            || (c.max_temporary_lockouts == 0 && r.num_failures >= c.failure_factor)
        {
            r.permanently_locked = true;
        }
    }

    /// Port of `isTemporarilyDisabled` — `currentTimeSeconds < failedLoginNotBefore`.
    pub fn is_temporarily_disabled(&self, r: &LoginFailureRecord, now_millis: i64) -> bool {
        now_millis / 1000 < r.failed_login_not_before
    }

    /// Port of `isPermanentlyLockedOut`.
    pub fn is_permanently_locked(&self, r: &LoginFailureRecord) -> bool {
        r.permanently_locked
    }
}
