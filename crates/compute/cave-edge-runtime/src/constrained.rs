// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Constrained-resource mode — kubelet allocatable + eviction at the edge.
//!
//! K3s targets resource-poor hardware (the docs cite a 256 MB-class node).
//! This module ports the two kubelet mechanisms that make that viable:
//!
//!   * **Allocatable accounting + admission.** `allocatable = capacity -
//!     reserved` (system + kube reserved). A pod is admitted only if its
//!     memory *request* fits the remaining allocatable — the request-based
//!     admission gate.
//!
//!   * **Memory-pressure eviction.** The hard signal fires when
//!     `memory.available < threshold`. When evicting, kubelet's
//!     `rankMemoryPressure` orders victims by `OrderedBy(exceedMemoryRequests,
//!     priority, memoryUsage)`: pods using more than they requested go first,
//!     then ascending priority, then descending usage-above-request. QoS
//!     class derives from the request/limit relationship.
//!
//! Pure accounting/ranking — no cgroups, no live memory stats.

/// Kubernetes QoS class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QosClass {
    Guaranteed,
    Burstable,
    BestEffort,
}

/// A pod's memory footprint (all values in MB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodResource {
    pub name: String,
    pub request_mb: u64,
    pub limit_mb: u64,
    pub usage_mb: u64,
    pub priority: i32,
}

impl PodResource {
    /// QoS class from the request/limit relationship (memory-focused).
    pub fn qos(&self) -> QosClass {
        if self.request_mb == 0 && self.limit_mb == 0 {
            QosClass::BestEffort
        } else if self.limit_mb > 0 && self.request_mb == self.limit_mb {
            QosClass::Guaranteed
        } else {
            QosClass::Burstable
        }
    }

    /// Usage above the memory request (clamped at 0). BestEffort pods have a
    /// zero request, so their full usage counts as overage.
    fn usage_above_request(&self) -> i64 {
        self.usage_mb as i64 - self.request_mb as i64
    }
}

/// Node memory budget (all values in MB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBudget {
    pub total_mb: u64,
    pub reserved_mb: u64,
}

impl ResourceBudget {
    /// `allocatable = capacity - reserved` (saturating at 0).
    pub fn allocatable_mb(&self) -> u64 {
        self.total_mb.saturating_sub(self.reserved_mb)
    }
}

/// A ranked eviction victim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionCandidate {
    pub name: String,
    pub usage_above_request_mb: i64,
}

/// The constrained-resource controller for one edge node.
#[derive(Debug, Clone)]
pub struct ConstrainedMode {
    budget: ResourceBudget,
    pods: Vec<PodResource>,
}

impl ConstrainedMode {
    pub fn new(budget: ResourceBudget) -> Self {
        Self {
            budget,
            pods: Vec::new(),
        }
    }

    pub fn budget(&self) -> ResourceBudget {
        self.budget
    }

    /// Request-based admission: admit iff the new request fits the remaining
    /// allocatable. Returns whether admitted; rejected pods are not tracked.
    pub fn try_admit(&mut self, pod: &PodResource) -> bool {
        if self.used_request_mb() + pod.request_mb > self.budget.allocatable_mb() {
            return false;
        }
        self.pods.push(pod.clone());
        true
    }

    /// Sum of admitted memory requests.
    pub fn used_request_mb(&self) -> u64 {
        self.pods.iter().map(|p| p.request_mb).sum()
    }

    /// Sum of observed memory usage.
    pub fn used_usage_mb(&self) -> u64 {
        self.pods.iter().map(|p| p.usage_mb).sum()
    }

    /// Hard memory-pressure signal: `available < threshold`, where
    /// `available = allocatable - used_usage`.
    pub fn under_memory_pressure(&self, threshold_mb: u64) -> bool {
        let available = self.budget.allocatable_mb().saturating_sub(self.used_usage_mb());
        available < threshold_mb
    }

    /// Eviction order per kubelet `rankMemoryPressure`:
    /// `OrderedBy(exceedMemoryRequests, priority, memoryUsage)`.
    pub fn eviction_order(&self) -> Vec<EvictionCandidate> {
        let mut ranked: Vec<&PodResource> = self.pods.iter().collect();
        ranked.sort_by(|a, b| {
            // 1. Pods exceeding their request come first.
            let a_exceeds = a.usage_above_request() > 0;
            let b_exceeds = b.usage_above_request() > 0;
            b_exceeds
                .cmp(&a_exceeds)
                // 2. Ascending priority (lower priority evicted first).
                .then(a.priority.cmp(&b.priority))
                // 3. Descending usage-above-request.
                .then(b.usage_above_request().cmp(&a.usage_above_request()))
        });
        ranked
            .into_iter()
            .map(|p| EvictionCandidate {
                name: p.name.clone(),
                usage_above_request_mb: p.usage_above_request(),
            })
            .collect()
    }
}
