//! Outlier detection — Envoy `cluster.v3.OutlierDetection` parity.
//!
//! Tracks per-endpoint runtime stats (consecutive 5xx, consecutive gateway
//! failures, success rate, consecutive local origin failures) and, when
//! thresholds are crossed, ejects the endpoint from the load-balancer for
//! `base_ejection_time × n` where `n` is the number of past ejections,
//! capped at `max_ejection_time`.
//!
//! Plays alongside `circuit_breaker.rs`: the breaker is per-target inside
//! the proxy hot path; outlier detection is per-target across the cluster.
//! Both can fire; the first wins.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutlierDetectionConfig {
    pub consecutive_5xx: u32,
    pub consecutive_gateway_failure: u32,
    pub consecutive_local_origin_failure: u32,
    pub interval: Duration,
    pub base_ejection_time: Duration,
    pub max_ejection_time: Duration,
    pub max_ejection_percent: u32,
    pub enforcing_consecutive_5xx: u32,
    pub enforcing_consecutive_gateway_failure: u32,
    pub enforcing_consecutive_local_origin_failure: u32,
    pub success_rate_minimum_hosts: u32,
    pub success_rate_request_volume: u32,
    pub success_rate_stdev_factor: f64,
    pub enforcing_success_rate: u32,
    pub split_external_local_origin_errors: bool,
}

impl Default for OutlierDetectionConfig {
    fn default() -> Self {
        Self {
            consecutive_5xx: 5,
            consecutive_gateway_failure: 5,
            consecutive_local_origin_failure: 5,
            interval: Duration::from_secs(10),
            base_ejection_time: Duration::from_secs(30),
            max_ejection_time: Duration::from_secs(300),
            max_ejection_percent: 10,
            enforcing_consecutive_5xx: 100,
            enforcing_consecutive_gateway_failure: 0,
            enforcing_consecutive_local_origin_failure: 0,
            success_rate_minimum_hosts: 5,
            success_rate_request_volume: 100,
            success_rate_stdev_factor: 1.9,
            enforcing_success_rate: 100,
            split_external_local_origin_errors: false,
        }
    }
}

impl OutlierDetectionConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (n, v) in [
            ("enforcing_consecutive_5xx", self.enforcing_consecutive_5xx),
            (
                "enforcing_consecutive_gateway_failure",
                self.enforcing_consecutive_gateway_failure,
            ),
            (
                "enforcing_consecutive_local_origin_failure",
                self.enforcing_consecutive_local_origin_failure,
            ),
            ("enforcing_success_rate", self.enforcing_success_rate),
            ("max_ejection_percent", self.max_ejection_percent),
        ] {
            if v > 100 {
                return Err(format!("{} must be in [0, 100]", n));
            }
        }
        if self.base_ejection_time > self.max_ejection_time {
            return Err("base_ejection_time > max_ejection_time".into());
        }
        if self.interval.is_zero() {
            return Err("interval must be > 0".into());
        }
        Ok(())
    }
}

// ── Outcome categories ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutlierOutcome {
    /// 2xx/3xx response.
    Success,
    /// 5xx response (no gateway-specific failure).
    Server5xx,
    /// 502 / 503 / 504 — distinct gateway errors that may also count.
    GatewayFailure,
    /// Locally observed failure: connect refused, timeout, reset.
    LocalOriginFailure,
}

// ── Per-endpoint stats ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct EndpointStats {
    consecutive_5xx: AtomicU32,
    consecutive_gateway_failure: AtomicU32,
    consecutive_local_origin_failure: AtomicU32,
    success_count: AtomicU32,
    request_count: AtomicU32,
    /// Number of times this endpoint has been ejected, ever.
    ejection_count: AtomicU32,
    /// State guarded by the parent DashMap entry mutability.
    state: Mutex<EjectionState>,
}

#[derive(Debug, Default, Clone)]
struct EjectionState {
    ejected_at: Option<Instant>,
    eject_until: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EjectionStatus {
    Active,
    Ejected,
}

// ── OutlierRegistry — public API ────────────────────────────────────────────

#[derive(Clone)]
pub struct OutlierRegistry {
    cfg: Arc<OutlierDetectionConfig>,
    stats: Arc<DashMap<Uuid, EndpointStats>>,
    /// Cluster-level total endpoint count, so we can enforce
    /// `max_ejection_percent`.
    cluster_size: Arc<DashMap<String, u32>>,
    /// Per-cluster current ejected count.
    ejected: Arc<DashMap<String, u32>>,
}

impl OutlierRegistry {
    pub fn new(cfg: OutlierDetectionConfig) -> Self {
        Self {
            cfg: Arc::new(cfg),
            stats: Arc::new(DashMap::new()),
            cluster_size: Arc::new(DashMap::new()),
            ejected: Arc::new(DashMap::new()),
        }
    }

    pub fn config(&self) -> &OutlierDetectionConfig {
        &self.cfg
    }

    pub fn set_cluster_size(&self, cluster: &str, size: u32) {
        self.cluster_size.insert(cluster.to_string(), size);
    }

    fn entry(&self, target: Uuid) -> dashmap::mapref::one::RefMut<'_, Uuid, EndpointStats> {
        self.stats.entry(target).or_default()
    }

    /// Record an outcome and decide whether to eject.
    /// Returns true if this call resulted in a fresh ejection.
    pub fn observe(&self, cluster: &str, target: Uuid, outcome: OutlierOutcome) -> bool {
        let cfg = self.cfg.clone();
        let entry = self.entry(target);

        entry.request_count.fetch_add(1, Ordering::Relaxed);

        match outcome {
            OutlierOutcome::Success => {
                entry.success_count.fetch_add(1, Ordering::Relaxed);
                entry.consecutive_5xx.store(0, Ordering::Relaxed);
                entry.consecutive_gateway_failure.store(0, Ordering::Relaxed);
                entry.consecutive_local_origin_failure.store(0, Ordering::Relaxed);
                return false;
            }
            OutlierOutcome::Server5xx => {
                entry.consecutive_5xx.fetch_add(1, Ordering::Relaxed);
                entry.consecutive_gateway_failure.store(0, Ordering::Relaxed);
                entry.consecutive_local_origin_failure.store(0, Ordering::Relaxed);
            }
            OutlierOutcome::GatewayFailure => {
                entry.consecutive_5xx.fetch_add(1, Ordering::Relaxed);
                entry.consecutive_gateway_failure.fetch_add(1, Ordering::Relaxed);
                entry.consecutive_local_origin_failure.store(0, Ordering::Relaxed);
            }
            OutlierOutcome::LocalOriginFailure => {
                entry.consecutive_local_origin_failure.fetch_add(1, Ordering::Relaxed);
                entry.consecutive_5xx.store(0, Ordering::Relaxed);
                entry.consecutive_gateway_failure.store(0, Ordering::Relaxed);
            }
        }

        // Don't eject if already ejected.
        let already_ejected = {
            let state = entry.state.lock().unwrap();
            state.ejected_at.is_some()
        };
        if already_ejected {
            return false;
        }

        let trigger = check_consecutive_triggers(&cfg, &entry);
        if !trigger {
            return false;
        }

        // Cluster-level cap: max_ejection_percent.
        let cluster_size = self
            .cluster_size
            .get(cluster)
            .map(|v| *v)
            .unwrap_or(0);
        let currently_ejected = self
            .ejected
            .get(cluster)
            .map(|v| *v)
            .unwrap_or(0);
        if cluster_size > 0 {
            let limit = ((cluster_size as u64 * cfg.max_ejection_percent as u64) / 100) as u32;
            // Always allow 1 eject when limit==0 but cluster is small? Envoy
            // always allows at least 1 for max_ejection_percent>0 only when
            // a 1-host cluster makes the integer round to 0 — in our config
            // layer we mirror Envoy: if limit < 1 we allow the very first
            // ejection only if max_ejection_percent > 0.
            if currently_ejected >= limit && !(currently_ejected == 0 && cfg.max_ejection_percent > 0) {
                return false;
            }
        }

        let prior_ejections = entry.ejection_count.fetch_add(1, Ordering::Relaxed);
        let factor = (prior_ejections + 1).max(1) as u64;
        let raw = cfg.base_ejection_time.saturating_mul(factor as u32);
        let duration = if raw > cfg.max_ejection_time { cfg.max_ejection_time } else { raw };
        let now = Instant::now();
        {
            let mut state = entry.state.lock().unwrap();
            state.ejected_at = Some(now);
            state.eject_until = Some(now + duration);
        }
        *self.ejected.entry(cluster.to_string()).or_insert(0) += 1;
        true
    }

    /// Has the ejection expired? Move back to active if so.
    pub fn refresh(&self, cluster: &str, target: Uuid) {
        let entry = self.entry(target);
        let mut state = entry.state.lock().unwrap();
        if let Some(until) = state.eject_until {
            if Instant::now() >= until {
                state.ejected_at = None;
                state.eject_until = None;
                drop(state);
                if let Some(mut v) = self.ejected.get_mut(cluster) {
                    *v = v.saturating_sub(1);
                }
            }
        }
    }

    pub fn status(&self, target: Uuid) -> EjectionStatus {
        if let Some(e) = self.stats.get(&target) {
            let state = e.state.lock().unwrap();
            if let Some(until) = state.eject_until {
                if Instant::now() < until {
                    return EjectionStatus::Ejected;
                }
            }
        }
        EjectionStatus::Active
    }

    /// Compute success-rate ejection candidates across a cluster: returns the
    /// list of targets whose success rate is more than `stdev_factor` standard
    /// deviations below the mean.
    pub fn success_rate_outliers(
        &self,
        cluster: &str,
        targets: &[Uuid],
    ) -> Vec<Uuid> {
        let cfg = &self.cfg;
        if (targets.len() as u32) < cfg.success_rate_minimum_hosts {
            return vec![];
        }

        let mut rates: Vec<(Uuid, f64)> = Vec::new();
        for t in targets {
            let entry = self.entry(*t);
            let req = entry.request_count.load(Ordering::Relaxed);
            if req < cfg.success_rate_request_volume {
                continue;
            }
            let succ = entry.success_count.load(Ordering::Relaxed);
            let rate = (succ as f64) / (req as f64);
            rates.push((*t, rate));
        }
        if (rates.len() as u32) < cfg.success_rate_minimum_hosts {
            return vec![];
        }

        let mean = rates.iter().map(|(_, r)| r).sum::<f64>() / rates.len() as f64;
        let variance = rates
            .iter()
            .map(|(_, r)| (r - mean).powi(2))
            .sum::<f64>()
            / rates.len() as f64;
        let stdev = variance.sqrt();
        let threshold = mean - cfg.success_rate_stdev_factor * stdev;

        // Cluster-cap as before.
        let cluster_size = self
            .cluster_size
            .get(cluster)
            .map(|v| *v)
            .unwrap_or(targets.len() as u32);
        let limit =
            ((cluster_size as u64 * cfg.max_ejection_percent as u64) / 100) as u32;
        let already = self.ejected.get(cluster).map(|v| *v).unwrap_or(0);

        let mut outliers: Vec<(Uuid, f64)> = rates
            .into_iter()
            .filter(|(_, r)| *r < threshold)
            .collect();
        outliers.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let max_to_eject = limit.saturating_sub(already) as usize;
        outliers.truncate(max_to_eject);

        outliers.into_iter().map(|(t, _)| t).collect()
    }
}

fn check_consecutive_triggers(cfg: &OutlierDetectionConfig, entry: &EndpointStats) -> bool {
    let c5xx = entry.consecutive_5xx.load(Ordering::Relaxed);
    let cgw = entry.consecutive_gateway_failure.load(Ordering::Relaxed);
    let cloc = entry.consecutive_local_origin_failure.load(Ordering::Relaxed);

    if cfg.consecutive_5xx > 0
        && c5xx >= cfg.consecutive_5xx
        && cfg.enforcing_consecutive_5xx > 0
    {
        return true;
    }
    if cfg.consecutive_gateway_failure > 0
        && cgw >= cfg.consecutive_gateway_failure
        && cfg.enforcing_consecutive_gateway_failure > 0
    {
        return true;
    }
    if cfg.consecutive_local_origin_failure > 0
        && cloc >= cfg.consecutive_local_origin_failure
        && cfg.enforcing_consecutive_local_origin_failure > 0
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(consecutive_5xx: u32, max_pct: u32) -> OutlierDetectionConfig {
        OutlierDetectionConfig {
            consecutive_5xx,
            enforcing_consecutive_5xx: 100,
            max_ejection_percent: max_pct,
            base_ejection_time: Duration::from_millis(50),
            max_ejection_time: Duration::from_millis(200),
            ..Default::default()
        }
    }

    // ── Config validation ──

    #[test]
    fn config_default_validates() {
        OutlierDetectionConfig::default().validate().unwrap();
    }

    #[test]
    fn config_invalid_pct_rejects() {
        let mut c = OutlierDetectionConfig::default();
        c.enforcing_consecutive_5xx = 200;
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_base_above_max_rejects() {
        let mut c = OutlierDetectionConfig::default();
        c.base_ejection_time = Duration::from_secs(100);
        c.max_ejection_time = Duration::from_secs(10);
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_zero_interval_rejects() {
        let mut c = OutlierDetectionConfig::default();
        c.interval = Duration::ZERO;
        assert!(c.validate().is_err());
    }

    // ── Consecutive 5xx triggers ──

    #[test]
    fn consecutive_5xx_ejects_after_threshold() {
        let r = OutlierRegistry::new(cfg(3, 100));
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        assert!(!r.observe("c", t, OutlierOutcome::Server5xx));
        assert!(!r.observe("c", t, OutlierOutcome::Server5xx));
        assert!(r.observe("c", t, OutlierOutcome::Server5xx));
        assert_eq!(r.status(t), EjectionStatus::Ejected);
    }

    #[test]
    fn success_resets_consecutive_counters() {
        let r = OutlierRegistry::new(cfg(3, 100));
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        r.observe("c", t, OutlierOutcome::Server5xx);
        r.observe("c", t, OutlierOutcome::Server5xx);
        r.observe("c", t, OutlierOutcome::Success);
        assert!(!r.observe("c", t, OutlierOutcome::Server5xx));
        assert_eq!(r.status(t), EjectionStatus::Active);
    }

    #[test]
    fn enforcing_zero_does_not_eject() {
        let mut c = cfg(2, 100);
        c.enforcing_consecutive_5xx = 0;
        let r = OutlierRegistry::new(c);
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        for _ in 0..5 {
            assert!(!r.observe("c", t, OutlierOutcome::Server5xx));
        }
        assert_eq!(r.status(t), EjectionStatus::Active);
    }

    #[test]
    fn gateway_failure_increments_5xx_too() {
        let r = OutlierRegistry::new(cfg(2, 100));
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        r.observe("c", t, OutlierOutcome::GatewayFailure);
        assert!(r.observe("c", t, OutlierOutcome::GatewayFailure));
    }

    #[test]
    fn local_origin_failure_separate_track() {
        let mut c = OutlierDetectionConfig {
            consecutive_local_origin_failure: 2,
            enforcing_consecutive_local_origin_failure: 100,
            consecutive_5xx: 0,
            ..Default::default()
        };
        c.base_ejection_time = Duration::from_millis(50);
        c.max_ejection_time = Duration::from_millis(200);
        let r = OutlierRegistry::new(c);
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        r.observe("c", t, OutlierOutcome::LocalOriginFailure);
        assert!(r.observe("c", t, OutlierOutcome::LocalOriginFailure));
    }

    // ── Ejection state ──

    #[test]
    fn already_ejected_does_not_re_eject() {
        let r = OutlierRegistry::new(cfg(2, 100));
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        r.observe("c", t, OutlierOutcome::Server5xx);
        assert!(r.observe("c", t, OutlierOutcome::Server5xx));
        assert!(!r.observe("c", t, OutlierOutcome::Server5xx));
    }

    #[test]
    fn refresh_clears_expired_ejection() {
        let r = OutlierRegistry::new(cfg(1, 100));
        r.set_cluster_size("c", 1);
        let t = Uuid::new_v4();
        r.observe("c", t, OutlierOutcome::Server5xx);
        assert_eq!(r.status(t), EjectionStatus::Ejected);
        std::thread::sleep(Duration::from_millis(70));
        r.refresh("c", t);
        assert_eq!(r.status(t), EjectionStatus::Active);
    }

    // ── Cluster cap ──

    #[test]
    fn cluster_cap_enforces_max_ejection_percent() {
        let r = OutlierRegistry::new(cfg(1, 50)); // up to 50% of cluster
        r.set_cluster_size("c", 4); // limit = 2
        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        let t3 = Uuid::new_v4();
        assert!(r.observe("c", t1, OutlierOutcome::Server5xx));
        assert!(r.observe("c", t2, OutlierOutcome::Server5xx));
        // The third would exceed the cap.
        assert!(!r.observe("c", t3, OutlierOutcome::Server5xx));
    }

    #[test]
    fn cluster_cap_zero_pct_blocks_all() {
        let r = OutlierRegistry::new(cfg(1, 0));
        r.set_cluster_size("c", 10);
        let t = Uuid::new_v4();
        assert!(!r.observe("c", t, OutlierOutcome::Server5xx));
    }

    // ── Success rate ──

    #[test]
    fn success_rate_no_action_below_min_hosts() {
        let mut c = cfg(0, 100);
        c.success_rate_minimum_hosts = 5;
        c.success_rate_request_volume = 1;
        let r = OutlierRegistry::new(c);
        let targets: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        for t in &targets {
            r.observe("c", *t, OutlierOutcome::Success);
        }
        let outliers = r.success_rate_outliers("c", &targets);
        assert!(outliers.is_empty());
    }

    #[test]
    fn success_rate_finds_outlier() {
        let mut c = cfg(0, 100);
        c.success_rate_minimum_hosts = 3;
        c.success_rate_request_volume = 5;
        c.success_rate_stdev_factor = 1.0;
        let r = OutlierRegistry::new(c);
        r.set_cluster_size("c", 4);
        let good: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
        let bad = Uuid::new_v4();
        for t in &good {
            for _ in 0..5 {
                r.observe("c", *t, OutlierOutcome::Success);
            }
        }
        for _ in 0..5 {
            r.observe("c", bad, OutlierOutcome::Server5xx);
        }
        let mut targets = good.clone();
        targets.push(bad);
        let outliers = r.success_rate_outliers("c", &targets);
        assert!(outliers.contains(&bad));
        assert!(!outliers.iter().any(|x| good.contains(x)));
    }

    #[test]
    fn success_rate_respects_max_ejection_percent() {
        let mut c = cfg(0, 25); // 25% of 4 = 1 ejection allowed
        c.success_rate_minimum_hosts = 3;
        c.success_rate_request_volume = 5;
        c.success_rate_stdev_factor = 0.5;
        let r = OutlierRegistry::new(c);
        r.set_cluster_size("c", 4);
        let mut targets = Vec::new();
        // 1 healthy host, 3 unhealthy → all 3 are outliers, but cap = 1.
        let healthy = Uuid::new_v4();
        for _ in 0..5 {
            r.observe("c", healthy, OutlierOutcome::Success);
        }
        targets.push(healthy);
        for _ in 0..3 {
            let t = Uuid::new_v4();
            for _ in 0..5 {
                r.observe("c", t, OutlierOutcome::Server5xx);
            }
            targets.push(t);
        }
        let outliers = r.success_rate_outliers("c", &targets);
        assert!(outliers.len() <= 1);
    }

    #[test]
    fn success_rate_request_volume_below_threshold_skips() {
        let mut c = cfg(0, 100);
        c.success_rate_minimum_hosts = 2;
        c.success_rate_request_volume = 100;
        c.success_rate_stdev_factor = 1.0;
        let r = OutlierRegistry::new(c);
        let t1 = Uuid::new_v4();
        let t2 = Uuid::new_v4();
        // Only 5 requests each → below volume threshold.
        for _ in 0..5 {
            r.observe("c", t1, OutlierOutcome::Success);
            r.observe("c", t2, OutlierOutcome::Server5xx);
        }
        assert!(r.success_rate_outliers("c", &[t1, t2]).is_empty());
    }

    // ── Status / config getter ──

    #[test]
    fn status_unknown_target_returns_active() {
        let r = OutlierRegistry::new(cfg(1, 100));
        assert_eq!(r.status(Uuid::new_v4()), EjectionStatus::Active);
    }

    #[test]
    fn config_getter() {
        let c = cfg(7, 50);
        let r = OutlierRegistry::new(c.clone());
        assert_eq!(r.config().consecutive_5xx, 7);
        assert_eq!(r.config().max_ejection_percent, 50);
    }

    // ── Serde ──

    #[test]
    fn config_json_roundtrip() {
        let c = OutlierDetectionConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let c2: OutlierDetectionConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }
}
