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
