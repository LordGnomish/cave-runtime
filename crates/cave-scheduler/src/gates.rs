// SPDX-License-Identifier: AGPL-3.0-or-later
//! SchedulingGates PreEnqueue plugin (KEP-3521 GA in v1.30).
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/schedulinggates/scheduling_gates.go
//!
//! A pod with non-empty `spec.scheduling_gates` cannot enter the active queue
//! until every gate has been removed by a controller. This plugin returns
//! `Pending` while gates remain, keeping the pod in the unschedulable subqueue
//! where it waits for a cluster event (gate removal) to re-enqueue it.

use crate::extension_points::PreEnqueuePlugin;
use crate::framework::{Pod, Status};

/// Pod gate name. Matches upstream `PodSchedulingGate.name` semantics — a
/// human-readable string a controller adds to declare "I'm not done preparing
/// this pod yet."
pub type SchedulingGate = String;

/// Plugin: returns `Pending` while any gate remains; `Success` when all gates
/// are gone.
pub struct SchedulingGates;

impl PreEnqueuePlugin for SchedulingGates {
    fn name(&self) -> &str { "SchedulingGates" }
    fn pre_enqueue(&self, pod: &Pod) -> Status {
        if pod.spec.scheduling_gates.is_empty() {
            return Status::success("SchedulingGates");
        }
        let names: Vec<&str> = pod.spec.scheduling_gates.iter().map(String::as_str).collect();
        Status::pending(
            "SchedulingGates",
            format!("waiting on scheduling gates: [{}]", names.join(", ")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gates_succeeds() {
        let p = Pod::new("t", "ns", "p");
        let s = SchedulingGates.pre_enqueue(&p);
        assert!(s.is_success());
    }

    #[test]
    fn single_gate_blocks_with_pending() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.scheduling_gates.push("acme.com/awaiting-quota".into());
        let s = SchedulingGates.pre_enqueue(&p);
        assert!(s.is_pending());
        assert!(s.reasons[0].contains("acme.com/awaiting-quota"));
    }

    #[test]
    fn multiple_gates_listed_in_message() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.scheduling_gates.push("a".into());
        p.spec.scheduling_gates.push("b".into());
        p.spec.scheduling_gates.push("c".into());
        let s = SchedulingGates.pre_enqueue(&p);
        assert!(s.is_pending());
        let r = &s.reasons[0];
        assert!(r.contains("a"));
        assert!(r.contains("b"));
        assert!(r.contains("c"));
    }

    #[test]
    fn cleared_gates_unblock() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.scheduling_gates.push("g".into());
        assert!(SchedulingGates.pre_enqueue(&p).is_pending());
        p.spec.scheduling_gates.clear();
        assert!(SchedulingGates.pre_enqueue(&p).is_success());
    }

    #[test]
    fn pending_status_does_not_count_as_rejected() {
        let mut p = Pod::new("t", "ns", "p");
        p.spec.scheduling_gates.push("g".into());
        let s = SchedulingGates.pre_enqueue(&p);
        assert!(!s.is_rejected());
        assert!(!s.is_success());
        assert!(s.is_pending());
    }
}
