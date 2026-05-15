// SPDX-License-Identifier: AGPL-3.0-or-later
//
// WebAuthn Prometheus metrics.
//
// We expose the metric names the dashboards + alert rules reference in
// observability/{dashboards,alerts}/cave-auth.{json,yml}.  The struct
// `WebauthnMetrics` is a thread-safe atomic counter bag that can be
// snapshot'd into a scrape response by the host runtime.  The host is
// expected to translate these atomics into prometheus_client::Family
// instances on its own (cave-auth deliberately does not depend on
// prometheus_client to keep its dep graph small).
//
// Metric names mirror the cave-auth dashboard:
//   cave_auth_webauthn_registrations_total{format, result}
//   cave_auth_webauthn_authn_total{result}
//   cave_auth_webauthn_replay_attempts_total
//   cave_auth_webauthn_mds_last_update_unix
//   cave_auth_webauthn_credentials{alg}

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct WebauthnMetrics {
    pub reg_success: AtomicU64,
    pub reg_failure: AtomicU64,
    pub authn_success: AtomicU64,
    pub authn_counter_regression: AtomicU64,
    pub authn_bad_signature: AtomicU64,
    pub authn_uv_required: AtomicU64,
    pub authn_origin_mismatch: AtomicU64,
    pub replay_attempts: AtomicU64,
    pub mds_last_update_unix: AtomicI64,
}

impl WebauthnMetrics {
    pub const fn new() -> Self {
        Self {
            reg_success: AtomicU64::new(0),
            reg_failure: AtomicU64::new(0),
            authn_success: AtomicU64::new(0),
            authn_counter_regression: AtomicU64::new(0),
            authn_bad_signature: AtomicU64::new(0),
            authn_uv_required: AtomicU64::new(0),
            authn_origin_mismatch: AtomicU64::new(0),
            replay_attempts: AtomicU64::new(0),
            mds_last_update_unix: AtomicI64::new(0),
        }
    }

    pub fn record_registration_success(&self) {
        self.reg_success.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_registration_failure(&self) {
        self.reg_failure.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_authn_success(&self) {
        self.authn_success.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_counter_regression(&self) {
        self.replay_attempts.fetch_add(1, Ordering::Relaxed);
        self.authn_counter_regression.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_bad_signature(&self) {
        self.authn_bad_signature.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_uv_required(&self) {
        self.authn_uv_required.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_origin_mismatch(&self) {
        self.authn_origin_mismatch.fetch_add(1, Ordering::Relaxed);
    }
    pub fn mds_blob_refreshed(&self, now_unix: i64) {
        self.mds_last_update_unix.store(now_unix, Ordering::Relaxed);
    }
}

/// Snapshot view a `/metrics` scrape can render into Prom text format.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WebauthnMetricsSnapshot {
    pub reg_success: u64,
    pub reg_failure: u64,
    pub authn_success: u64,
    pub authn_counter_regression: u64,
    pub authn_bad_signature: u64,
    pub authn_uv_required: u64,
    pub authn_origin_mismatch: u64,
    pub replay_attempts: u64,
    pub mds_last_update_unix: i64,
}

impl WebauthnMetrics {
    pub fn snapshot(&self) -> WebauthnMetricsSnapshot {
        WebauthnMetricsSnapshot {
            reg_success: self.reg_success.load(Ordering::Relaxed),
            reg_failure: self.reg_failure.load(Ordering::Relaxed),
            authn_success: self.authn_success.load(Ordering::Relaxed),
            authn_counter_regression: self.authn_counter_regression.load(Ordering::Relaxed),
            authn_bad_signature: self.authn_bad_signature.load(Ordering::Relaxed),
            authn_uv_required: self.authn_uv_required.load(Ordering::Relaxed),
            authn_origin_mismatch: self.authn_origin_mismatch.load(Ordering::Relaxed),
            replay_attempts: self.replay_attempts.load(Ordering::Relaxed),
            mds_last_update_unix: self.mds_last_update_unix.load(Ordering::Relaxed),
        }
    }

    /// Render Prom text-format directly — useful when the host wants
    /// to mount it onto its `/metrics` exposition without going through
    /// prometheus_client.
    pub fn render_prom(&self) -> String {
        let s = self.snapshot();
        format!(
            "# HELP cave_auth_webauthn_registrations_total Total registration ceremonies.\n\
             # TYPE cave_auth_webauthn_registrations_total counter\n\
             cave_auth_webauthn_registrations_total{{result=\"success\"}} {rs}\n\
             cave_auth_webauthn_registrations_total{{result=\"failure\"}} {rf}\n\
             # HELP cave_auth_webauthn_authn_total Total authentication ceremonies.\n\
             # TYPE cave_auth_webauthn_authn_total counter\n\
             cave_auth_webauthn_authn_total{{result=\"success\"}} {as_}\n\
             cave_auth_webauthn_authn_total{{result=\"counter_regression\"}} {acr}\n\
             cave_auth_webauthn_authn_total{{result=\"bad_signature\"}} {abs}\n\
             cave_auth_webauthn_authn_total{{result=\"uv_required\"}} {auv}\n\
             cave_auth_webauthn_authn_total{{result=\"origin_mismatch\"}} {ao}\n\
             # HELP cave_auth_webauthn_replay_attempts_total Counter regressions detected.\n\
             # TYPE cave_auth_webauthn_replay_attempts_total counter\n\
             cave_auth_webauthn_replay_attempts_total {ra}\n\
             # HELP cave_auth_webauthn_mds_last_update_unix UNIX time of last MDS3 blob refresh.\n\
             # TYPE cave_auth_webauthn_mds_last_update_unix gauge\n\
             cave_auth_webauthn_mds_last_update_unix {mu}\n",
            rs = s.reg_success,
            rf = s.reg_failure,
            as_ = s.authn_success,
            acr = s.authn_counter_regression,
            abs = s.authn_bad_signature,
            auv = s.authn_uv_required,
            ao = s.authn_origin_mismatch,
            ra = s.replay_attempts,
            mu = s.mds_last_update_unix,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero() {
        let m = WebauthnMetrics::new();
        let s = m.snapshot();
        assert_eq!(s.reg_success, 0);
        assert_eq!(s.replay_attempts, 0);
    }

    #[test]
    fn counter_regression_also_bumps_replay() {
        let m = WebauthnMetrics::new();
        m.record_counter_regression();
        m.record_counter_regression();
        let s = m.snapshot();
        assert_eq!(s.replay_attempts, 2);
        assert_eq!(s.authn_counter_regression, 2);
    }

    #[test]
    fn render_prom_contains_metric_names() {
        let m = WebauthnMetrics::new();
        m.record_registration_success();
        m.record_authn_success();
        let prom = m.render_prom();
        assert!(prom.contains("cave_auth_webauthn_registrations_total{result=\"success\"} 1"));
        assert!(prom.contains("cave_auth_webauthn_authn_total{result=\"success\"} 1"));
        assert!(prom.contains("# TYPE cave_auth_webauthn_replay_attempts_total counter"));
    }

    #[test]
    fn mds_refresh_updates_gauge() {
        let m = WebauthnMetrics::new();
        m.mds_blob_refreshed(1_700_000_000);
        assert_eq!(m.snapshot().mds_last_update_unix, 1_700_000_000);
    }
}
