// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD line-port of Keycloak's brute-force protector.
// Upstream (Apache-2.0):
//   services/src/main/java/org/keycloak/services/managers/DefaultBruteForceProtector.java
//
// The wait-time computation, quick-login minimum wait, max-delta failure reset,
// temporary-lockout counting and permanent-lockout trigger are all asserted
// against hand-traced values of the upstream `failure(...)` method.
//
// Timestamps start at BASE (a realistic, non-zero, second-aligned epoch-ms):
// upstream `Time.currentTimeMillis()` is never 0, and a 0 failure time is
// indistinguishable from the "never failed" sentinel (`last_failure == 0`),
// which is exactly the `last > 0` guard Keycloak uses for quick-login detection.

use cave_auth::brute_force::{
    BruteForceConfig, BruteForceProtector, BruteForceStrategy, LoginFailureRecord,
};

/// 1000s — non-zero and a whole number of seconds so `failure_time / 1000` stays tidy.
const BASE: i64 = 1_000_000;

fn cfg(factor: u32, strategy: BruteForceStrategy) -> BruteForceConfig {
    BruteForceConfig {
        wait_increment_seconds: 60,
        failure_factor: factor,
        max_failure_wait_seconds: 900,
        max_delta_time_seconds: 43_200,
        quick_login_check_millis: 1000,
        minimum_quick_login_wait_seconds: 60,
        strategy,
        permanent_lockout: false,
        max_temporary_lockouts: 0,
    }
}

#[test]
fn multiple_strategy_locks_after_failure_factor() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Multiple));
    let mut r = LoginFailureRecord::default();

    // Failures spaced 2s apart: above quick-login window, below max-delta.
    p.record_failure(&mut r, BASE);
    assert!(!p.is_temporarily_disabled(&r, BASE + 1000));
    p.record_failure(&mut r, BASE + 2000);
    assert!(!p.is_temporarily_disabled(&r, BASE + 3000));

    // 3rd failure: num=3, wait = 60 * (3/3) = 60s, notBefore = 1004 + 60 = 1064.
    p.record_failure(&mut r, BASE + 4000);
    assert_eq!(r.num_failures, 3);
    assert_eq!(r.num_temporary_lockouts, 1);
    assert!(p.is_temporarily_disabled(&r, BASE + 5_000)); // 1005s < 1064s
    assert!(!p.is_temporarily_disabled(&r, 1_064_000)); // 1064s not < 1064s
    assert!(!p.is_temporarily_disabled(&r, 1_100_000));
}

#[test]
fn linear_strategy_increases_wait_each_failure() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Linear));
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, BASE); // num=1, wait = 60*(1+1-3) <= 0
    p.record_failure(&mut r, BASE + 2000); // num=2, wait = 60*(1+2-3) = 0
    p.record_failure(&mut r, BASE + 4000); // num=3, wait = 60 -> notBefore 1004+60=1064
    assert!(p.is_temporarily_disabled(&r, BASE + 10_000));
    assert!(!p.is_temporarily_disabled(&r, 1_064_000));
    // num=4, wait = 60*(1+4-3) = 120 -> notBefore = 1006 + 120 = 1126.
    p.record_failure(&mut r, BASE + 6000);
    assert!(p.is_temporarily_disabled(&r, 1_120_000)); // 1120s < 1126s
    assert!(!p.is_temporarily_disabled(&r, 1_126_000));
}

#[test]
fn quick_login_applies_minimum_wait_without_counting_lockout() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Multiple));
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, BASE); // last>0 only after this
    // 2nd failure 500ms later: delta < quick_login_check(1000) -> min wait 60s.
    p.record_failure(&mut r, BASE + 500);
    assert_eq!(r.num_failures, 2);
    assert_eq!(r.num_temporary_lockouts, 0, "quick-login must not count as a temporary lockout");
    assert!(p.is_temporarily_disabled(&r, BASE + 1000)); // notBefore = 1000 + 60 = 1060
    assert!(!p.is_temporarily_disabled(&r, 1_060_000));
}

#[test]
fn max_delta_time_resets_failure_count() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Multiple));
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, BASE);
    p.record_failure(&mut r, BASE + 2000);
    assert_eq!(r.num_failures, 2);
    // Next failure after more than max_delta (43_200s): failures cleared, then +1.
    let far = BASE + 2000 + 43_200 * 1000 + 1;
    p.record_failure(&mut r, far);
    assert_eq!(r.num_failures, 1);
    assert!(!p.is_temporarily_disabled(&r, far + 1000));
}

#[test]
fn permanent_lockout_after_exceeding_max_temporary_lockouts() {
    let mut c = cfg(3, BruteForceStrategy::Multiple);
    c.permanent_lockout = true;
    c.max_temporary_lockouts = 1;
    let p = BruteForceProtector::new(c);
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, BASE); // num=1
    p.record_failure(&mut r, BASE + 2000); // num=2
    p.record_failure(&mut r, BASE + 4000); // num=3 -> temp lockouts=1, not yet permanent
    assert!(!r.permanently_locked);
    p.record_failure(&mut r, BASE + 6000); // num=4 -> temp lockouts=2 > max(1) -> permanent
    assert!(r.permanently_locked);
}
