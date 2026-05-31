// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! RED → GREEN TDD for the constrained-resource mode (256 MB target).
//!
//! Faithful port of two kubelet pieces K3s relies on at the edge:
//!   - node allocatable accounting (`allocatable = capacity - reserved`) and
//!     request-based admission (a pod is admitted only if its memory request
//!     fits the remaining allocatable);
//!   - the hard memory-pressure eviction signal (`memory.available <
//!     threshold`) and the eviction ranking `rankMemoryPressure` —
//!     `OrderedBy(exceedMemoryRequests, priority, memoryUsage)`: pods using
//!     more memory than they requested are evicted first, then by ascending
//!     priority, then by descending usage-above-request. QoS class derives
//!     from request/limit per the Kubernetes QoS rules.
//!
//! Pure accounting/ranking logic — no cgroups, no real memory stats.

use cave_edge_runtime::constrained::{ConstrainedMode, PodResource, QosClass, ResourceBudget};

fn pod(name: &str, request: u64, limit: u64, usage: u64, priority: i32) -> PodResource {
    PodResource {
        name: name.to_string(),
        request_mb: request,
        limit_mb: limit,
        usage_mb: usage,
        priority,
    }
}

// ─── QoS classification ─────────────────────────────────────────────────────

#[test]
fn qos_best_effort_when_no_request_or_limit() {
    assert_eq!(pod("a", 0, 0, 0, 0).qos(), QosClass::BestEffort);
}

#[test]
fn qos_guaranteed_when_request_equals_limit() {
    assert_eq!(pod("a", 64, 64, 0, 0).qos(), QosClass::Guaranteed);
}

#[test]
fn qos_burstable_otherwise() {
    assert_eq!(pod("a", 32, 64, 0, 0).qos(), QosClass::Burstable);
}

// ─── allocatable + admission ────────────────────────────────────────────────

#[test]
fn allocatable_is_capacity_minus_reserved() {
    let b = ResourceBudget {
        total_mb: 256,
        reserved_mb: 56,
    };
    assert_eq!(b.allocatable_mb(), 200);
}

#[test]
fn admits_pods_until_allocatable_is_exhausted() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 256,
        reserved_mb: 56,
    }); // allocatable = 200
    assert!(cm.try_admit(&pod("a", 120, 120, 0, 0)));
    assert!(cm.try_admit(&pod("b", 80, 80, 0, 0))); // 120 + 80 = 200, fits
    // No headroom left for even a tiny request.
    assert!(!cm.try_admit(&pod("c", 16, 16, 0, 0)));
    assert_eq!(cm.used_request_mb(), 200);
}

#[test]
fn rejected_pod_is_not_tracked() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 256,
        reserved_mb: 56,
    });
    assert!(!cm.try_admit(&pod("huge", 300, 300, 0, 0)));
    assert_eq!(cm.used_request_mb(), 0);
}

// ─── memory-pressure signal ─────────────────────────────────────────────────

#[test]
fn under_pressure_when_available_below_threshold() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 256,
        reserved_mb: 56,
    }); // allocatable = 200
    cm.try_admit(&pod("a", 100, 150, 180, 0)); // using 180 of 200
    // available = 200 - 180 = 20 < threshold 50 → pressure.
    assert!(cm.under_memory_pressure(50));
    // available 20 >= threshold 10 → no pressure.
    assert!(!cm.under_memory_pressure(10));
}

// ─── eviction ranking (rankMemoryPressure) ──────────────────────────────────

#[test]
fn pods_exceeding_request_are_evicted_before_those_within() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 1024,
        reserved_mb: 0,
    });
    cm.try_admit(&pod("within", 100, 200, 50, 0)); // usage 50 < request 100
    cm.try_admit(&pod("exceeds", 50, 200, 120, 0)); // usage 120 > request 50
    let order: Vec<String> = cm.eviction_order().into_iter().map(|c| c.name).collect();
    assert_eq!(order, vec!["exceeds".to_string(), "within".to_string()]);
}

#[test]
fn among_exceeders_higher_usage_above_request_goes_first() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 1024,
        reserved_mb: 0,
    });
    cm.try_admit(&pod("small", 10, 200, 40, 0)); // +30 over request
    cm.try_admit(&pod("big", 10, 200, 90, 0)); // +80 over request
    let order: Vec<String> = cm.eviction_order().into_iter().map(|c| c.name).collect();
    assert_eq!(order, vec!["big".to_string(), "small".to_string()]);
}

#[test]
fn lower_priority_evicted_first_when_overage_equal() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 1024,
        reserved_mb: 0,
    });
    cm.try_admit(&pod("high-prio", 10, 200, 60, 100)); // +50 over, prio 100
    cm.try_admit(&pod("low-prio", 10, 200, 60, 1)); // +50 over, prio 1
    let order: Vec<String> = cm.eviction_order().into_iter().map(|c| c.name).collect();
    assert_eq!(order, vec!["low-prio".to_string(), "high-prio".to_string()]);
}

#[test]
fn best_effort_pod_reports_full_usage_as_overage() {
    let mut cm = ConstrainedMode::new(ResourceBudget {
        total_mb: 1024,
        reserved_mb: 0,
    });
    cm.try_admit(&pod("be", 0, 0, 30, 0));
    let cands = cm.eviction_order();
    assert_eq!(cands[0].name, "be");
    assert_eq!(cands[0].usage_above_request_mb, 30);
}
