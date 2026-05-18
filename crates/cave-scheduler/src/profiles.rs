// SPDX-License-Identifier: AGPL-3.0-or-later
//! Multiple scheduling profiles per cluster — pods opt-in via spec.schedulerName.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/apis/config/types.go (KubeSchedulerProfile)
//!   pkg/scheduler/scheduler.go (Profiles map)

use crate::framework::{Framework, Pod, Status};
use std::collections::HashMap;

/// One scheduling profile = a name + a fully-configured Framework. A cluster may
/// host many profiles; pods route to one via `spec.schedulerName`.
pub struct Profile {
    pub name: String,
    pub framework: Framework,
}

pub struct ProfileRegistry {
    profiles: HashMap<String, Profile>,
    default_profile: String,
}

impl ProfileRegistry {
    pub fn new(default_profile: &str) -> Self {
        Self { profiles: HashMap::new(), default_profile: default_profile.into() }
    }

    pub fn register(&mut self, profile: Profile) {
        self.profiles.insert(profile.name.clone(), profile);
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.profiles.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn get(&self, name: &str) -> Option<&Profile> { self.profiles.get(name) }

    /// Look up the profile a pod should use (its spec.schedulerName, or the default).
    pub fn for_pod(&self, pod: &Pod) -> Result<&Profile, Status> {
        let want = if pod.spec.scheduler_name.is_empty() {
            &self.default_profile
        } else {
            &pod.spec.scheduler_name
        };
        self.profiles.get(want)
            .ok_or_else(|| Status::unschedulable("ProfileRegistry", format!("no profile named {}", want)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::{ClusterSnapshot, FilterPlugin, Pod, ScorePlugin, MAX_NODE_SCORE};
    use crate::models::{Node, NodeStatus, ResourceCapacity};
    use chrono::Utc;
    use uuid::Uuid;

    struct AlwaysOk;
    impl FilterPlugin for AlwaysOk {
        fn name(&self) -> &str { "AlwaysOk" }
        fn filter(&self, _: &Pod, _: &Node, _: &ClusterSnapshot) -> Status { Status::success("AlwaysOk") }
    }
    struct Five;
    impl ScorePlugin for Five {
        fn name(&self) -> &str { "Five" }
        fn score(&self, _: &Pod, _: &Node, _: &ClusterSnapshot) -> i64 { 5 }
    }

    fn ready(name: &str) -> Node {
        Node { name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity { cpu_millicores: 1000, memory_bytes: 1, pods: 10, ephemeral_storage_bytes: 0 },
            allocatable: ResourceCapacity { cpu_millicores: 1000, memory_bytes: 1, pods: 10, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity::default(),
            labels: std::collections::HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    #[test]
    fn empty_scheduler_name_uses_default() {
        let mut reg = ProfileRegistry::new("default-scheduler");
        reg.register(Profile {
            name: "default-scheduler".into(),
            framework: Framework::new().with_filter(Box::new(AlwaysOk)).with_score(Box::new(Five)),
        });
        let pod = Pod::new("t", "ns", "p");
        let prof = reg.for_pod(&pod).expect("default lookup");
        assert_eq!(prof.name, "default-scheduler");
    }

    #[test]
    fn explicit_scheduler_name_routes_to_that_profile() {
        let mut reg = ProfileRegistry::new("default-scheduler");
        reg.register(Profile { name: "default-scheduler".into(), framework: Framework::new() });
        reg.register(Profile { name: "ml-scheduler".into(), framework: Framework::new().with_filter(Box::new(AlwaysOk)) });

        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.scheduler_name = "ml-scheduler".into();
        let prof = reg.for_pod(&pod).expect("ml lookup");
        assert_eq!(prof.name, "ml-scheduler");
        assert_eq!(prof.framework.filters.len(), 1);
    }

    #[test]
    fn unknown_scheduler_name_unschedulable() {
        let reg = ProfileRegistry::new("default-scheduler");
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.scheduler_name = "ghost".into();
        let err = match reg.for_pod(&pod) { Ok(_) => panic!("expected error"), Err(e) => e };
        assert_eq!(err.plugin, "ProfileRegistry");
    }

    // ── Multi-profile end-to-end ─────────────────────────────────────────

    #[test]
    fn registered_profile_count_round_trip() {
        let mut reg = ProfileRegistry::new("p1");
        reg.register(Profile { name: "p1".into(), framework: Framework::new() });
        reg.register(Profile { name: "p2".into(), framework: Framework::new() });
        reg.register(Profile { name: "p3".into(), framework: Framework::new() });
        let names = reg.names();
        assert_eq!(names, vec!["p1".to_string(), "p2".to_string(), "p3".to_string()]);
    }

    #[test]
    fn default_profile_falls_back_when_pod_carries_empty_scheduler_name() {
        let mut reg = ProfileRegistry::new("default-scheduler");
        reg.register(Profile {
            name: "default-scheduler".into(),
            framework: Framework::new().with_filter(Box::new(AlwaysOk)),
        });
        reg.register(Profile { name: "ml".into(), framework: Framework::new() });
        let pod = Pod::new("t", "ns", "p");
        let prof = reg.for_pod(&pod).unwrap();
        assert_eq!(prof.name, "default-scheduler");
    }

    #[test]
    fn pod_routed_to_named_profile() {
        let mut reg = ProfileRegistry::new("default-scheduler");
        reg.register(Profile { name: "default-scheduler".into(), framework: Framework::new() });
        reg.register(Profile {
            name: "batch".into(),
            framework: Framework::new().with_filter(Box::new(AlwaysOk)).with_score(Box::new(Five)),
        });
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.scheduler_name = "batch".into();
        let prof = reg.for_pod(&pod).unwrap();
        assert_eq!(prof.name, "batch");
        assert_eq!(prof.framework.scores.len(), 1);
    }

    #[test]
    fn pod_with_unknown_scheduler_name_yields_unschedulable_status() {
        let reg = ProfileRegistry::new("default");
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.scheduler_name = "nope".into();
        let err = reg.for_pod(&pod).err().expect("missing profile");
        assert_eq!(err.code, crate::framework::Code::Unschedulable);
        assert!(err.reasons[0].contains("nope"));
    }

    // ── Profile + Bind handshake ─────────────────────────────────────────

    #[test]
    fn profile_bind_chain_runs_default_binder() {
        use crate::bind::{DefaultBinder, NoopPreBinder, PostBindLogger};
        use crate::cycle_state::CycleState;

        let binder = std::sync::Arc::new(DefaultBinder::new());
        let logger = std::sync::Arc::new(PostBindLogger::new());

        struct WrapBind(std::sync::Arc<DefaultBinder>);
        impl crate::extension_points::BindPlugin for WrapBind {
            fn name(&self) -> &str { "DefaultBinder" }
            fn bind(&self, p: &Pod, n: &str, s: &CycleState) -> Status {
                self.0.bind(p, n, s)
            }
        }

        struct WrapPostBind(std::sync::Arc<PostBindLogger>);
        impl crate::extension_points::PostBindPlugin for WrapPostBind {
            fn name(&self) -> &str { "PostBindLogger" }
            fn post_bind(&self, p: &Pod, n: &str, s: &CycleState) {
                self.0.post_bind(p, n, s);
            }
        }

        let mut reg = ProfileRegistry::new("default");
        reg.register(Profile {
            name: "default".into(),
            framework: Framework::new()
                .with_pre_bind(Box::new(NoopPreBinder))
                .with_bind(Box::new(WrapBind(binder.clone())))
                .with_post_bind(Box::new(WrapPostBind(logger.clone()))),
        });

        let pod = Pod::new("t", "ns", "p");
        let prof = reg.for_pod(&pod).unwrap();
        let cs = CycleState::new();
        assert!(prof.framework.run_pre_bind(&pod, "n1", &cs).is_success());
        assert!(prof.framework.run_bind(&pod, "n1", &cs).is_success());
        prof.framework.run_post_bind(&pod, "n1", &cs);
        assert_eq!(binder.count(), 1);
        assert_eq!(logger.events().len(), 1);
    }

    #[test]
    fn profile_pre_enqueue_with_scheduling_gates() {
        use crate::gates::SchedulingGates;

        let mut reg = ProfileRegistry::new("default");
        reg.register(Profile {
            name: "default".into(),
            framework: Framework::new().with_pre_enqueue(Box::new(SchedulingGates)),
        });
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.scheduling_gates.push("acme.com/wait".into());

        let prof = reg.for_pod(&pod).unwrap();
        let st = prof.framework.run_pre_enqueue(&pod);
        assert!(st.is_pending());
        assert_eq!(st.plugin, "SchedulingGates");
    }

    #[test]
    fn profile_uses_priority_sort_queue_sort_plugin() {
        use crate::priority_sort::PrioritySort;
        let mut reg = ProfileRegistry::new("default");
        reg.register(Profile {
            name: "default".into(),
            framework: Framework::new().with_queue_sort(Box::new(PrioritySort)),
        });
        let prof = reg.get("default").unwrap();
        let mut a = Pod::new("t", "ns", "a"); a.spec.priority = 100;
        let mut b = Pod::new("t", "ns", "b"); b.spec.priority = 50;
        assert_eq!(prof.framework.queue_sort(&a, &b), std::cmp::Ordering::Less);
    }

    #[test]
    fn profile_each_has_isolated_state() {
        use crate::bind::DefaultBinder;
        struct WrapBind(std::sync::Arc<DefaultBinder>);
        impl crate::extension_points::BindPlugin for WrapBind {
            fn name(&self) -> &str { "DefaultBinder" }
            fn bind(&self, p: &Pod, n: &str, s: &crate::cycle_state::CycleState) -> Status {
                self.0.bind(p, n, s)
            }
        }
        let bind_a = std::sync::Arc::new(DefaultBinder::new());
        let bind_b = std::sync::Arc::new(DefaultBinder::new());
        let mut reg = ProfileRegistry::new("a");
        reg.register(Profile {
            name: "a".into(),
            framework: Framework::new().with_bind(Box::new(WrapBind(bind_a.clone()))),
        });
        reg.register(Profile {
            name: "b".into(),
            framework: Framework::new().with_bind(Box::new(WrapBind(bind_b.clone()))),
        });
        let cs = crate::cycle_state::CycleState::new();
        let pod_a = { let mut p = Pod::new("t", "ns", "p"); p.spec.scheduler_name = "a".into(); p };
        let pod_b = { let mut p = Pod::new("t", "ns", "q"); p.spec.scheduler_name = "b".into(); p };
        reg.for_pod(&pod_a).unwrap().framework.run_bind(&pod_a, "n", &cs);
        reg.for_pod(&pod_b).unwrap().framework.run_bind(&pod_b, "n", &cs);
        reg.for_pod(&pod_b).unwrap().framework.run_bind(&pod_b, "n", &cs);
        assert_eq!(bind_a.count(), 1);
        assert_eq!(bind_b.count(), 2);
    }

    #[test]
    fn each_profile_has_independent_weights() {
        let mut reg = ProfileRegistry::new("p1");
        reg.register(Profile {
            name: "p1".into(),
            framework: Framework::new().with_score(Box::new(Five)).with_weight("Five", 1),
        });
        reg.register(Profile {
            name: "p2".into(),
            framework: Framework::new().with_score(Box::new(Five)).with_weight("Five", 10),
        });

        let snap = ClusterSnapshot { nodes: vec![ready("a")], pods_by_node: std::collections::HashMap::new() };
        let pod = Pod::new("t", "ns", "p");
        let s1 = reg.get("p1").unwrap().framework.run_scores(&pod, &["a".into()], &snap)["a"];
        let s2 = reg.get("p2").unwrap().framework.run_scores(&pod, &["a".into()], &snap)["a"];
        assert_eq!(s1, 5);
        assert_eq!(s2, 50);
        assert!(s2 <= MAX_NODE_SCORE * 10);
    }
}
