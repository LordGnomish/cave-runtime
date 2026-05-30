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

use cave_auth::brute_force::{
    BruteForceConfig, BruteForceProtector, BruteForceStrategy, LoginFailureRecord,
};

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
    p.record_failure(&mut r, 0);
    assert!(!p.is_temporarily_disabled(&r, 1000));
    p.record_failure(&mut r, 2000);
    assert!(!p.is_temporarily_disabled(&r, 3000));

    // 3rd failure: num=3, wait = 60 * (3/3) = 60s, notBefore = 4 + 60 = 64.
    p.record_failure(&mut r, 4000);
    assert_eq!(r.num_failures, 3);
    assert_eq!(r.num_temporary_lockouts, 1);
    assert!(p.is_temporarily_disabled(&r, 5_000)); // 5s < 64s
    assert!(!p.is_temporarily_disabled(&r, 64_000)); // 64s not < 64s
    assert!(!p.is_temporarily_disabled(&r, 100_000));
}

#[test]
fn linear_strategy_increases_wait_each_failure() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Linear));
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, 0); // num=1, wait = 60*(1+1-3) <= 0
    p.record_failure(&mut r, 2000); // num=2, wait = 60*(1+2-3) = 0
    p.record_failure(&mut r, 4000); // num=3, wait = 60*(1+3-3) = 60 -> notBefore 4+60=64
    assert!(p.is_temporarily_disabled(&r, 10_000));
    assert!(!p.is_temporarily_disabled(&r, 64_000));
    // num=4, wait = 60*(1+4-3) = 120 -> notBefore = 6 + 120 = 126.
    p.record_failure(&mut r, 6000);
    assert!(p.is_temporarily_disabled(&r, 120_000)); // 120s < 126s
    assert!(!p.is_temporarily_disabled(&r, 126_000));
}

#[test]
fn quick_login_applies_minimum_wait_without_counting_lockout() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Multiple));
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, 0); // last==0, no quick-login
    // 2nd failure 500ms later: delta < quick_login_check(1000) -> min wait 60s.
    p.record_failure(&mut r, 500);
    assert_eq!(r.num_failures, 2);
    assert_eq!(r.num_temporary_lockouts, 0, "quick-login must not count as a temporary lockout");
    assert!(p.is_temporarily_disabled(&r, 1000)); // notBefore = 0 + 60 = 60
    assert!(!p.is_temporarily_disabled(&r, 60_000));
}

#[test]
fn max_delta_time_resets_failure_count() {
    let p = BruteForceProtector::new(cfg(3, BruteForceStrategy::Multiple));
    let mut r = LoginFailureRecord::default();
    p.record_failure(&mut r, 0);
    p.record_failure(&mut r, 2000);
    assert_eq!(r.num_failures, 2);
    // Next failure after more than max_delta (43_200s): failures cleared, then +1.
    let far = 2000 + 43_200 * 1000 + 1;
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
    p.record_failure(&mut r, 0); // num=1
    p.record_failure(&mut r, 2000); // num=2
    p.record_failure(&mut r, 4000); // num=3 -> temp lockouts=1, not yet permanent
    assert!(!r.permanently_locked);
    p.record_failure(&mut r, 6000); // num=4 -> temp lockouts=2 > max(1) -> permanent
    assert!(r.permanently_locked);
}
