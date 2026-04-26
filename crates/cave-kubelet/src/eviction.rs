//! Eviction manager — node-pressure pod eviction.
//!
//! Mirrors `pkg/kubelet/eviction` semantics: soft and hard thresholds against
//! node-level signals (`memory.available`, `nodefs.available`,
//! `nodefs.inodesFree`, `imagefs.available`, `imagefs.inodesFree`,
//! `pid.available`, `allocatableMemory.available`), grace-period gating for
//! soft thresholds, pod ranking by QoS, priority and resource usage relative
//! to request, and node-condition reporting.
//!
//! All operations are pure / synchronous so the decision logic is testable
//! without touching cgroups or the kernel.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Signal {
    MemoryAvailable,
    NodeFsAvailable,
    NodeFsInodesFree,
    ImageFsAvailable,
    ImageFsInodesFree,
    PidAvailable,
    /// Allocatable memory — distinct from MemoryAvailable; used for
    /// `--enforce-node-allocatable=pods` enforcement.
    AllocatableMemoryAvailable,
}

impl Signal {
    pub fn is_disk(self) -> bool {
        matches!(
            self,
            Signal::NodeFsAvailable
                | Signal::NodeFsInodesFree
                | Signal::ImageFsAvailable
                | Signal::ImageFsInodesFree
        )
    }

    pub fn is_memory(self) -> bool {
        matches!(self, Signal::MemoryAvailable | Signal::AllocatableMemoryAvailable)
    }

    pub fn node_condition(self) -> Option<&'static str> {
        match self {
            Signal::MemoryAvailable | Signal::AllocatableMemoryAvailable => Some("MemoryPressure"),
            Signal::NodeFsAvailable
            | Signal::NodeFsInodesFree
            | Signal::ImageFsAvailable
            | Signal::ImageFsInodesFree => Some("DiskPressure"),
            Signal::PidAvailable => Some("PIDPressure"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdOp {
    /// Trigger when value <= threshold (the K8s default for availability signals).
    LessThan,
}

/// A threshold value: an absolute byte/inode count or a percent of capacity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ThresholdValue {
    Quantity(u64),
    Percent(f64),
}

impl ThresholdValue {
    pub fn resolve(self, capacity: u64) -> u64 {
        match self {
            ThresholdValue::Quantity(q) => q,
            ThresholdValue::Percent(p) => ((capacity as f64) * (p / 100.0)) as u64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvictionThreshold {
    pub signal: Signal,
    pub op: ThresholdOp,
    pub value: ThresholdValue,
    /// `0` = hard threshold; otherwise soft.
    pub grace_period_seconds: u32,
    /// k8s `--eviction-max-pod-grace-period` override for soft thresholds.
    pub min_reclaim: Option<ThresholdValue>,
}

impl EvictionThreshold {
    pub fn hard(signal: Signal, value: ThresholdValue) -> Self {
        Self {
            signal,
            op: ThresholdOp::LessThan,
            value,
            grace_period_seconds: 0,
            min_reclaim: None,
        }
    }

    pub fn soft(signal: Signal, value: ThresholdValue, grace_seconds: u32) -> Self {
        Self {
            signal,
            op: ThresholdOp::LessThan,
            value,
            grace_period_seconds: grace_seconds,
            min_reclaim: None,
        }
    }

    pub fn is_hard(&self) -> bool {
        self.grace_period_seconds == 0
    }
}

/// A single observation of a node-level signal.
#[derive(Debug, Clone, Copy)]
pub struct SignalObservation {
    pub signal: Signal,
    pub available: u64,
    pub capacity: u64,
    pub time: DateTime<Utc>,
}

/// Observations for a tick — the current view of node pressure.
pub type SignalSet = BTreeMap<Signal, SignalObservation>;

/// QoS class (mirrors core/v1.PodQOSClass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum QosClass {
    /// Highest priority — at least one container has request==limit on every resource.
    Guaranteed,
    /// At least one container has a request or limit, but not all are equal.
    Burstable,
    /// No requests or limits anywhere.
    BestEffort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodForEviction {
    pub uid: String,
    pub name: String,
    pub namespace: String,
    pub qos: QosClass,
    /// `priorityClass` value; higher = less likely to be evicted.
    pub priority: i32,
    /// Memory request total, bytes.
    pub memory_request: u64,
    /// Actual memory usage, bytes.
    pub memory_usage: u64,
    /// Ephemeral-storage request total, bytes.
    pub ephemeral_storage_request: u64,
    /// Actual ephemeral-storage usage, bytes.
    pub ephemeral_storage_usage: u64,
    /// Whether this is a critical / system pod (priority>=2_000_000_000 or
    /// `system-cluster-critical` / `system-node-critical` priorityClass).
    pub critical: bool,
    /// Whether the pod is a static-mirror pod managed by the kubelet directly.
    pub static_pod: bool,
}

impl PodForEviction {
    pub fn memory_overage(&self) -> i64 {
        self.memory_usage as i64 - self.memory_request as i64
    }

    pub fn ephemeral_storage_overage(&self) -> i64 {
        self.ephemeral_storage_usage as i64 - self.ephemeral_storage_request as i64
    }
}

/// Soft-threshold observation history — counts when a soft threshold
/// has been continuously crossed.
#[derive(Debug, Clone, Default)]
pub struct ThresholdObservations {
    /// First time we saw the signal cross.
    first_seen: BTreeMap<Signal, DateTime<Utc>>,
}

impl ThresholdObservations {
    pub fn record_observation(
        &mut self,
        threshold: &EvictionThreshold,
        obs: &SignalObservation,
    ) {
        if signal_crosses(threshold, obs) {
            self.first_seen.entry(threshold.signal).or_insert(obs.time);
        } else {
            self.first_seen.remove(&threshold.signal);
        }
    }

    pub fn first_seen(&self, signal: Signal) -> Option<DateTime<Utc>> {
        self.first_seen.get(&signal).copied()
    }

    pub fn clear(&mut self, signal: Signal) {
        self.first_seen.remove(&signal);
    }
}

pub fn signal_crosses(threshold: &EvictionThreshold, obs: &SignalObservation) -> bool {
    match threshold.op {
        ThresholdOp::LessThan => {
            let value = threshold.value.resolve(obs.capacity);
            obs.available <= value
        }
    }
}

/// Decision returned by the eviction manager for a tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionDecision {
    pub triggered_signals: Vec<Signal>,
    pub node_conditions: Vec<String>,
    /// Pod uids selected for eviction this tick (in eviction order — first first).
    pub evict: Vec<String>,
}

/// Evaluate thresholds against current observations and pod set; returns the
/// decision the kubelet should act on this tick.
pub fn evaluate(
    thresholds: &[EvictionThreshold],
    observations: &SignalSet,
    history: &mut ThresholdObservations,
    pods: &[PodForEviction],
    now: DateTime<Utc>,
) -> EvictionDecision {
    let mut triggered = Vec::new();
    let mut conditions: Vec<String> = Vec::new();

    for t in thresholds {
        if let Some(obs) = observations.get(&t.signal) {
            history.record_observation(t, obs);
            let cross = signal_crosses(t, obs);
            if !cross {
                continue;
            }
            if t.is_hard() {
                triggered.push(t.signal);
            } else if let Some(first) = history.first_seen(t.signal) {
                if now - first >= Duration::seconds(t.grace_period_seconds as i64) {
                    triggered.push(t.signal);
                }
            }
        }
    }

    triggered.sort();
    triggered.dedup();

    for s in &triggered {
        if let Some(c) = s.node_condition() {
            if !conditions.iter().any(|x| x == c) {
                conditions.push(c.to_string());
            }
        }
    }

    let evict = if triggered.is_empty() {
        Vec::new()
    } else {
        select_pods_to_evict(&triggered, pods)
    };

    EvictionDecision {
        triggered_signals: triggered,
        node_conditions: conditions,
        evict,
    }
}

/// Pod ranking implementation — port of `pkg/kubelet/eviction/helpers.go`
/// `rankMemoryPressure` / `rankDiskPressureFunc` ideas:
///   1. critical and static pods last (skipped unless nothing else is available)
///   2. exceeding requests (highest priority)
///   3. higher memory/disk usage above request first
///   4. lower priority first
///   5. higher absolute usage first
pub fn select_pods_to_evict(
    triggered: &[Signal],
    pods: &[PodForEviction],
) -> Vec<String> {
    let mem_pressure = triggered.iter().any(|s| s.is_memory());
    let disk_pressure = triggered.iter().any(|s| s.is_disk());

    let mut ranked: Vec<&PodForEviction> = pods
        .iter()
        .filter(|p| !p.critical && !p.static_pod)
        .collect();

    ranked.sort_by(|a, b| {
        // 1) Pods exceeding their request go first.
        let a_over = if mem_pressure {
            a.memory_overage() > 0
        } else if disk_pressure {
            a.ephemeral_storage_overage() > 0
        } else {
            false
        };
        let b_over = if mem_pressure {
            b.memory_overage() > 0
        } else if disk_pressure {
            b.ephemeral_storage_overage() > 0
        } else {
            false
        };
        match b_over.cmp(&a_over) {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        // 2) BestEffort < Burstable < Guaranteed (BestEffort first).
        match a.qos.cmp(&b.qos).reverse() {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        // 3) Lower priority first.
        match a.priority.cmp(&b.priority) {
            std::cmp::Ordering::Equal => {}
            ord => return ord,
        }
        // 4) Higher overage first; if no pressure-specific overage, use absolute usage.
        let a_amt = if mem_pressure {
            a.memory_overage().max(0) as u64
        } else {
            a.ephemeral_storage_overage().max(0) as u64
        };
        let b_amt = if mem_pressure {
            b.memory_overage().max(0) as u64
        } else {
            b.ephemeral_storage_overage().max(0) as u64
        };
        b_amt.cmp(&a_amt).then_with(|| {
            let a_abs = if mem_pressure { a.memory_usage } else { a.ephemeral_storage_usage };
            let b_abs = if mem_pressure { b.memory_usage } else { b.ephemeral_storage_usage };
            b_abs.cmp(&a_abs)
        })
    });

    // If there are no candidates (only critical/static remain), fall back.
    if ranked.is_empty() {
        return pods
            .iter()
            .filter(|p| !p.critical) // never evict critical
            .map(|p| p.uid.clone())
            .collect();
    }

    ranked.into_iter().map(|p| p.uid.clone()).collect()
}

/// Returns the QoS class of a pod given its container resource specs.
pub fn classify_qos(containers: &[ResourceRequirements]) -> QosClass {
    let mut all_guaranteed = true;
    let mut any_request_or_limit = false;
    for c in containers {
        if c.cpu_request > 0 || c.memory_request > 0 || c.cpu_limit > 0 || c.memory_limit > 0 {
            any_request_or_limit = true;
        }
        let guaranteed = c.cpu_limit > 0
            && c.memory_limit > 0
            && c.cpu_request == c.cpu_limit
            && c.memory_request == c.memory_limit;
        if !guaranteed {
            all_guaranteed = false;
        }
    }
    if !any_request_or_limit {
        QosClass::BestEffort
    } else if all_guaranteed && !containers.is_empty() {
        QosClass::Guaranteed
    } else {
        QosClass::Burstable
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceRequirements {
    pub cpu_request: u64,
    pub cpu_limit: u64,
    pub memory_request: u64,
    pub memory_limit: u64,
}

impl ResourceRequirements {
    pub fn empty() -> Self {
        Self { cpu_request: 0, cpu_limit: 0, memory_request: 0, memory_limit: 0 }
    }
}

/// Allocatable enforcement — when `--enforce-node-allocatable=pods`,
/// the sum of pods' memory/cpu requests must not exceed the node's
/// allocatable; if it does, the latest-admitted-Burstable/BestEffort pods
/// are evicted in QoS+priority order.
pub fn enforce_allocatable(
    allocatable_memory: u64,
    pods: &[PodForEviction],
) -> Vec<String> {
    let total: u64 = pods.iter().map(|p| p.memory_usage).sum();
    if total <= allocatable_memory {
        return Vec::new();
    }
    let mut over = total - allocatable_memory;
    let triggered = vec![Signal::AllocatableMemoryAvailable];
    let mut victims: Vec<String> = Vec::new();
    for uid in select_pods_to_evict(&triggered, pods) {
        if over == 0 {
            break;
        }
        let used = pods
            .iter()
            .find(|p| p.uid == uid)
            .map(|p| p.memory_usage)
            .unwrap_or(0);
        over = over.saturating_sub(used);
        victims.push(uid);
    }
    victims
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(signal: Signal, available: u64, capacity: u64, t: DateTime<Utc>) -> SignalObservation {
        SignalObservation { signal, available, capacity, time: t }
    }

    fn mk_pod(
        uid: &str,
        qos: QosClass,
        priority: i32,
        mem_req: u64,
        mem_use: u64,
    ) -> PodForEviction {
        PodForEviction {
            uid: uid.into(),
            name: uid.into(),
            namespace: "default".into(),
            qos,
            priority,
            memory_request: mem_req,
            memory_usage: mem_use,
            ephemeral_storage_request: 0,
            ephemeral_storage_usage: 0,
            critical: false,
            static_pod: false,
        }
    }

    #[test]
    fn signal_classification() {
        assert!(Signal::MemoryAvailable.is_memory());
        assert!(Signal::NodeFsAvailable.is_disk());
        assert!(Signal::ImageFsAvailable.is_disk());
        assert!(Signal::PidAvailable.node_condition() == Some("PIDPressure"));
    }

    #[test]
    fn threshold_value_percent_resolves_against_capacity() {
        assert_eq!(ThresholdValue::Percent(10.0).resolve(1_000), 100);
        assert_eq!(ThresholdValue::Percent(15.5).resolve(1_000), 155);
        assert_eq!(ThresholdValue::Quantity(500).resolve(1_000), 500);
    }

    #[test]
    fn signal_crosses_when_available_below_threshold() {
        let t = EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Quantity(100));
        assert!(signal_crosses(&t, &obs(Signal::MemoryAvailable, 50, 1000, Utc::now())));
        assert!(signal_crosses(&t, &obs(Signal::MemoryAvailable, 100, 1000, Utc::now())));
        assert!(!signal_crosses(&t, &obs(Signal::MemoryAvailable, 200, 1000, Utc::now())));
    }

    #[test]
    fn signal_crosses_with_percent() {
        let t = EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Percent(10.0));
        // 10% of 1000 = 100. 99 < 100 → cross.
        assert!(signal_crosses(&t, &obs(Signal::MemoryAvailable, 99, 1000, Utc::now())));
        assert!(!signal_crosses(&t, &obs(Signal::MemoryAvailable, 200, 1000, Utc::now())));
    }

    #[test]
    fn hard_threshold_triggers_immediately() {
        let now = Utc::now();
        let mut h = ThresholdObservations::default();
        let t = EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Quantity(100));
        let mut obs_set = SignalSet::new();
        obs_set.insert(Signal::MemoryAvailable, obs(Signal::MemoryAvailable, 50, 1000, now));
        let d = evaluate(&[t], &obs_set, &mut h, &[], now);
        assert_eq!(d.triggered_signals, vec![Signal::MemoryAvailable]);
        assert_eq!(d.node_conditions, vec!["MemoryPressure".to_string()]);
    }

    #[test]
    fn soft_threshold_waits_for_grace_period() {
        let start = Utc::now();
        let mut h = ThresholdObservations::default();
        let t = EvictionThreshold::soft(Signal::MemoryAvailable, ThresholdValue::Quantity(100), 30);
        let mut obs_set = SignalSet::new();
        obs_set.insert(Signal::MemoryAvailable, obs(Signal::MemoryAvailable, 50, 1000, start));
        let d = evaluate(&[t.clone()], &obs_set, &mut h, &[], start);
        assert!(d.triggered_signals.is_empty(), "soft threshold should not fire before grace");

        let later = start + Duration::seconds(31);
        obs_set.insert(Signal::MemoryAvailable, obs(Signal::MemoryAvailable, 50, 1000, later));
        let d2 = evaluate(&[t], &obs_set, &mut h, &[], later);
        assert_eq!(d2.triggered_signals, vec![Signal::MemoryAvailable]);
    }

    #[test]
    fn soft_threshold_resets_when_signal_recovers() {
        let start = Utc::now();
        let mut h = ThresholdObservations::default();
        let t = EvictionThreshold::soft(Signal::MemoryAvailable, ThresholdValue::Quantity(100), 30);
        let mut obs_set = SignalSet::new();
        obs_set.insert(Signal::MemoryAvailable, obs(Signal::MemoryAvailable, 50, 1000, start));
        evaluate(&[t.clone()], &obs_set, &mut h, &[], start);
        // Recover.
        obs_set.insert(
            Signal::MemoryAvailable,
            obs(Signal::MemoryAvailable, 500, 1000, start + Duration::seconds(20)),
        );
        evaluate(&[t.clone()], &obs_set, &mut h, &[], start + Duration::seconds(20));
        assert!(h.first_seen(Signal::MemoryAvailable).is_none());
        // Re-cross — must restart grace.
        obs_set.insert(
            Signal::MemoryAvailable,
            obs(Signal::MemoryAvailable, 50, 1000, start + Duration::seconds(40)),
        );
        let d = evaluate(&[t], &obs_set, &mut h, &[], start + Duration::seconds(40));
        assert!(d.triggered_signals.is_empty(), "grace must restart");
    }

    #[test]
    fn besteffort_evicted_before_burstable_then_guaranteed() {
        let pods = vec![
            mk_pod("g", QosClass::Guaranteed, 0, 100, 50),
            mk_pod("b", QosClass::Burstable, 0, 100, 50),
            mk_pod("be", QosClass::BestEffort, 0, 0, 50),
        ];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order[0], "be");
        assert_eq!(order[1], "b");
        assert_eq!(order[2], "g");
    }

    #[test]
    fn pods_over_request_evicted_first() {
        let pods = vec![
            mk_pod("under", QosClass::Burstable, 0, 100, 50),
            mk_pod("over", QosClass::Burstable, 0, 100, 200),
        ];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order[0], "over");
    }

    #[test]
    fn higher_priority_evicted_last_within_qos() {
        let pods = vec![
            mk_pod("low", QosClass::Burstable, 1, 100, 200),
            mk_pod("high", QosClass::Burstable, 1000, 100, 200),
        ];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order[0], "low");
        assert_eq!(order[1], "high");
    }

    #[test]
    fn larger_overage_evicted_first_when_tied() {
        let pods = vec![
            mk_pod("small", QosClass::Burstable, 0, 100, 110),
            mk_pod("large", QosClass::Burstable, 0, 100, 500),
        ];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order[0], "large");
    }

    #[test]
    fn critical_pods_excluded_from_normal_ranking() {
        let mut crit = mk_pod("crit", QosClass::BestEffort, 0, 0, 1000);
        crit.critical = true;
        let pods = vec![crit, mk_pod("regular", QosClass::Burstable, 0, 100, 50)];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order, vec!["regular".to_string()]);
    }

    #[test]
    fn static_pods_excluded_from_normal_ranking() {
        let mut sp = mk_pod("static", QosClass::Burstable, 0, 100, 1000);
        sp.static_pod = true;
        let pods = vec![sp, mk_pod("regular", QosClass::Burstable, 0, 100, 50)];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order, vec!["regular".to_string()]);
    }

    #[test]
    fn fallback_when_only_critical_pods_remain_returns_empty_for_critical() {
        let mut crit = mk_pod("c", QosClass::BestEffort, 0, 0, 0);
        crit.critical = true;
        let pods = vec![crit];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert!(order.is_empty());
    }

    #[test]
    fn fallback_when_only_static_pods_remain_returns_them() {
        let mut sp = mk_pod("s", QosClass::Burstable, 0, 0, 100);
        sp.static_pod = true;
        let pods = vec![sp];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        assert_eq!(order, vec!["s".to_string()]);
    }

    #[test]
    fn disk_pressure_uses_ephemeral_storage_overage() {
        let mut over = mk_pod("over", QosClass::Burstable, 0, 0, 0);
        over.ephemeral_storage_request = 100;
        over.ephemeral_storage_usage = 500;
        let mut under = mk_pod("under", QosClass::Burstable, 0, 0, 0);
        under.ephemeral_storage_request = 100;
        under.ephemeral_storage_usage = 50;
        let order = select_pods_to_evict(&[Signal::NodeFsAvailable], &[over, under]);
        assert_eq!(order[0], "over");
    }

    #[test]
    fn evaluate_with_no_observations_returns_empty() {
        let mut h = ThresholdObservations::default();
        let d = evaluate(
            &[EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Quantity(100))],
            &SignalSet::new(),
            &mut h,
            &[],
            Utc::now(),
        );
        assert!(d.triggered_signals.is_empty());
        assert!(d.evict.is_empty());
    }

    #[test]
    fn evaluate_dedups_node_conditions_across_disk_signals() {
        let now = Utc::now();
        let mut h = ThresholdObservations::default();
        let ts = vec![
            EvictionThreshold::hard(Signal::NodeFsAvailable, ThresholdValue::Quantity(100)),
            EvictionThreshold::hard(Signal::ImageFsAvailable, ThresholdValue::Quantity(100)),
        ];
        let mut o = SignalSet::new();
        o.insert(Signal::NodeFsAvailable, obs(Signal::NodeFsAvailable, 50, 1000, now));
        o.insert(Signal::ImageFsAvailable, obs(Signal::ImageFsAvailable, 50, 1000, now));
        let d = evaluate(&ts, &o, &mut h, &[], now);
        assert_eq!(d.node_conditions, vec!["DiskPressure".to_string()]);
        assert_eq!(d.triggered_signals.len(), 2);
    }

    #[test]
    fn evaluate_evicts_pods_when_triggered() {
        let now = Utc::now();
        let mut h = ThresholdObservations::default();
        let mut o = SignalSet::new();
        o.insert(Signal::MemoryAvailable, obs(Signal::MemoryAvailable, 0, 1000, now));
        let d = evaluate(
            &[EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Quantity(100))],
            &o,
            &mut h,
            &[mk_pod("p", QosClass::BestEffort, 0, 0, 100)],
            now,
        );
        assert_eq!(d.evict, vec!["p".to_string()]);
    }

    #[test]
    fn classify_qos_besteffort_when_no_resources() {
        let cs = vec![ResourceRequirements::empty()];
        assert_eq!(classify_qos(&cs), QosClass::BestEffort);
    }

    #[test]
    fn classify_qos_guaranteed_when_request_equals_limit_for_all() {
        let c = ResourceRequirements { cpu_request: 100, cpu_limit: 100, memory_request: 1024, memory_limit: 1024 };
        assert_eq!(classify_qos(&[c]), QosClass::Guaranteed);
    }

    #[test]
    fn classify_qos_burstable_when_request_lt_limit() {
        let c = ResourceRequirements { cpu_request: 100, cpu_limit: 200, memory_request: 1024, memory_limit: 2048 };
        assert_eq!(classify_qos(&[c]), QosClass::Burstable);
    }

    #[test]
    fn classify_qos_burstable_when_only_request_set() {
        let c = ResourceRequirements { cpu_request: 100, cpu_limit: 0, memory_request: 0, memory_limit: 0 };
        assert_eq!(classify_qos(&[c]), QosClass::Burstable);
    }

    #[test]
    fn classify_qos_burstable_with_mixed_containers() {
        let g = ResourceRequirements { cpu_request: 100, cpu_limit: 100, memory_request: 1024, memory_limit: 1024 };
        let b = ResourceRequirements { cpu_request: 100, cpu_limit: 200, memory_request: 0, memory_limit: 0 };
        assert_eq!(classify_qos(&[g, b]), QosClass::Burstable);
    }

    #[test]
    fn allocatable_enforcement_returns_empty_when_under() {
        let pods = vec![mk_pod("p1", QosClass::BestEffort, 0, 0, 50)];
        assert!(enforce_allocatable(1000, &pods).is_empty());
    }

    #[test]
    fn allocatable_enforcement_evicts_until_below_target() {
        let pods = vec![
            mk_pod("be", QosClass::BestEffort, 0, 0, 600),
            mk_pod("g", QosClass::Guaranteed, 1000, 1000, 200),
        ];
        // total = 800, allocatable = 700 → over by 100.
        let v = enforce_allocatable(700, &pods);
        assert_eq!(v[0], "be");
    }

    #[test]
    fn allocatable_enforcement_evicts_multiple_when_one_not_enough() {
        let pods = vec![
            mk_pod("be1", QosClass::BestEffort, 0, 0, 100),
            mk_pod("be2", QosClass::BestEffort, 0, 0, 100),
            mk_pod("g", QosClass::Guaranteed, 1000, 1000, 200),
        ];
        // total = 400, allocatable = 150 → must drop 250 → both BE pods.
        let v = enforce_allocatable(150, &pods);
        assert!(v.contains(&"be1".to_string()));
        assert!(v.contains(&"be2".to_string()));
    }

    #[test]
    fn pod_overage_arithmetic() {
        let p = mk_pod("p", QosClass::Burstable, 0, 100, 250);
        assert_eq!(p.memory_overage(), 150);
        let p2 = mk_pod("p2", QosClass::Burstable, 0, 100, 50);
        assert_eq!(p2.memory_overage(), -50);
    }

    #[test]
    fn observations_clear_on_recovery() {
        let now = Utc::now();
        let mut h = ThresholdObservations::default();
        let t = EvictionThreshold::soft(Signal::MemoryAvailable, ThresholdValue::Quantity(100), 30);
        h.record_observation(&t, &obs(Signal::MemoryAvailable, 50, 1000, now));
        assert!(h.first_seen(Signal::MemoryAvailable).is_some());
        h.record_observation(&t, &obs(Signal::MemoryAvailable, 500, 1000, now));
        assert!(h.first_seen(Signal::MemoryAvailable).is_none());
    }

    #[test]
    fn first_seen_persists_across_continued_pressure() {
        let start = Utc::now();
        let mut h = ThresholdObservations::default();
        let t = EvictionThreshold::soft(Signal::MemoryAvailable, ThresholdValue::Quantity(100), 30);
        h.record_observation(&t, &obs(Signal::MemoryAvailable, 50, 1000, start));
        let later = start + Duration::seconds(15);
        h.record_observation(&t, &obs(Signal::MemoryAvailable, 50, 1000, later));
        assert_eq!(h.first_seen(Signal::MemoryAvailable), Some(start));
    }

    #[test]
    fn pid_pressure_signal_node_condition() {
        assert_eq!(Signal::PidAvailable.node_condition(), Some("PIDPressure"));
    }

    #[test]
    fn allocatable_memory_maps_to_memory_pressure_condition() {
        assert_eq!(Signal::AllocatableMemoryAvailable.node_condition(), Some("MemoryPressure"));
    }

    #[test]
    fn nodefs_inodes_maps_to_disk_pressure() {
        assert_eq!(Signal::NodeFsInodesFree.node_condition(), Some("DiskPressure"));
    }

    #[test]
    fn imagefs_inodes_maps_to_disk_pressure() {
        assert_eq!(Signal::ImageFsInodesFree.node_condition(), Some("DiskPressure"));
    }

    #[test]
    fn evaluate_handles_concurrent_memory_and_disk() {
        let now = Utc::now();
        let mut h = ThresholdObservations::default();
        let mut o = SignalSet::new();
        o.insert(Signal::MemoryAvailable, obs(Signal::MemoryAvailable, 0, 1000, now));
        o.insert(Signal::NodeFsAvailable, obs(Signal::NodeFsAvailable, 0, 1000, now));
        let d = evaluate(
            &[
                EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Quantity(50)),
                EvictionThreshold::hard(Signal::NodeFsAvailable, ThresholdValue::Quantity(50)),
            ],
            &o,
            &mut h,
            &[mk_pod("p", QosClass::BestEffort, 0, 0, 100)],
            now,
        );
        assert!(d.node_conditions.contains(&"MemoryPressure".to_string()));
        assert!(d.node_conditions.contains(&"DiskPressure".to_string()));
    }

    #[test]
    fn hard_threshold_helper_grace_zero() {
        let t = EvictionThreshold::hard(Signal::MemoryAvailable, ThresholdValue::Quantity(100));
        assert_eq!(t.grace_period_seconds, 0);
        assert!(t.is_hard());
    }

    #[test]
    fn soft_threshold_helper_grace_set() {
        let t = EvictionThreshold::soft(Signal::MemoryAvailable, ThresholdValue::Quantity(100), 30);
        assert_eq!(t.grace_period_seconds, 30);
        assert!(!t.is_hard());
    }

    #[test]
    fn classify_qos_empty_container_list_is_besteffort() {
        assert_eq!(classify_qos(&[]), QosClass::BestEffort);
    }

    #[test]
    fn evict_returns_in_order_of_priority_then_overage() {
        let pods = vec![
            mk_pod("low_under", QosClass::Burstable, 1, 100, 50),
            mk_pod("low_over", QosClass::Burstable, 1, 100, 300),
            mk_pod("high_over", QosClass::Burstable, 100, 100, 500),
        ];
        let order = select_pods_to_evict(&[Signal::MemoryAvailable], &pods);
        // Over-request first, then within-over the lower priority first.
        assert_eq!(order[0], "low_over");
        assert_eq!(order[1], "high_over");
        assert_eq!(order[2], "low_under");
    }
}
