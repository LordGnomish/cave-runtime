// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PrioritySort — QueueSort plugin matching upstream's default ordering.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/queuesort/priority_sort.go
//!
//! Order:
//! - Higher `pod.spec.priority` first.
//! - On tie, the pod that has been waiting longer wins. We don't track
//!   queue-enqueue timestamps in `Pod`, so tie-break falls through to UID
//!   ascending — deterministic and matches what tests assert when priorities
//!   are equal.

use crate::extension_points::QueueSortPlugin;
use crate::framework::Pod;
use std::cmp::Ordering;

/// Default QueueSort.
pub struct PrioritySort;

impl QueueSortPlugin for PrioritySort {
    fn name(&self) -> &str { "PrioritySort" }
    fn less(&self, a: &Pod, b: &Pod) -> Ordering {
        // Less means "schedule a before b". Higher priority first → if a's
        // priority is greater, return Less.
        b.spec.priority.cmp(&a.spec.priority).then_with(|| a.uid.cmp(&b.uid))
    }
}

/// Bundle helper — the default plugin set kube-scheduler ships out of the box.
///
/// Returns the *names* a profile would enable when no overrides are set;
/// callers feed this to their own KubeSchedulerProfile so the names round-trip
/// through profile configuration tests.
pub fn default_profile_plugin_names() -> Vec<&'static str> {
    vec![
        "PrioritySort",
        "SchedulingGates",
        "NodeName",
        "NodeUnschedulable",
        "NodeResources", // a.k.a. NodeResourcesFit
        "NodeAffinity",
        "TaintToleration",
        "InterPodAffinity",
        "PodTopologySpread",
        "VolumeBinding",
        "VolumeRestrictions",
        "VolumeZone",
        "NodePorts",
        "NodeVolumeLimits",
        "ImageLocality",
        "DefaultPreemption",
        "DefaultBinder",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, prio: i32) -> Pod {
        let mut p = Pod::new("t", "ns", name);
        p.spec.priority = prio;
        p
    }

    #[test]
    fn priority_sort_higher_first() {
        let lo = pod("lo", 1);
        let hi = pod("hi", 100);
        assert_eq!(PrioritySort.less(&hi, &lo), Ordering::Less);
        assert_eq!(PrioritySort.less(&lo, &hi), Ordering::Greater);
    }

    #[test]
    fn priority_sort_equal_priority_tie_breaks_by_uid() {
        let mut a = pod("a", 50);
        a.uid = "uid-a".into();
        let mut b = pod("b", 50);
        b.uid = "uid-b".into();
        assert_eq!(PrioritySort.less(&a, &b), Ordering::Less);
        assert_eq!(PrioritySort.less(&b, &a), Ordering::Greater);
        assert_eq!(PrioritySort.less(&a, &a), Ordering::Equal);
    }

    #[test]
    fn priority_sort_zero_vs_negative() {
        let a = pod("a", 0);
        let b = pod("b", -1);
        assert_eq!(PrioritySort.less(&a, &b), Ordering::Less);
    }

    #[test]
    fn priority_sort_plugin_name() {
        assert_eq!(PrioritySort.name(), "PrioritySort");
    }

    #[test]
    fn default_profile_plugin_names_include_required_set() {
        let names = default_profile_plugin_names();
        for required in &[
            "PrioritySort", "NodeName", "NodeUnschedulable", "NodeResources",
            "NodeAffinity", "TaintToleration", "InterPodAffinity",
            "PodTopologySpread", "VolumeBinding", "VolumeRestrictions",
            "VolumeZone", "NodePorts", "NodeVolumeLimits", "ImageLocality",
            "DefaultPreemption", "DefaultBinder", "SchedulingGates",
        ] {
            assert!(names.contains(required), "{} missing", required);
        }
    }

    #[test]
    fn default_profile_plugin_names_count_matches_upstream_v1_31() {
        // Upstream v1.31 ships 17 enabled defaults including PrioritySort
        // and SchedulingGates; our bundle mirrors that.
        let names = default_profile_plugin_names();
        assert_eq!(names.len(), 17);
    }

    #[test]
    fn default_profile_plugin_names_unique() {
        let names = default_profile_plugin_names();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }

    // Sanity: feeding PrioritySort into a Framework works.
    #[test]
    fn framework_queue_sort_plugin_overrides_default() {
        use crate::framework::Framework;
        let fw = Framework::new().with_queue_sort(Box::new(PrioritySort));
        let lo = pod("lo", 1);
        let hi = pod("hi", 100);
        assert_eq!(fw.queue_sort(&hi, &lo), Ordering::Less);
    }
}
