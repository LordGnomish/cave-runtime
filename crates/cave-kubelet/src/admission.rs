//! Pod admission predicates — node-side eligibility checks.
//!
//! Mirrors `pkg/kubelet/lifecycle/predicate.go` and the relevant predicates
//! from `pkg/scheduler/framework/plugins`: NodeAffinity (Required vs
//! Preferred ignored at admission), Taints/Tolerations (NoSchedule + NoExecute
//! semantics), node-level resource fit (cpu/memory/pods/ephemeral-storage),
//! node selector, pod-fits-host (`spec.nodeName`), and OS / arch matching.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub taints: Vec<Taint>,
    pub allocatable: NodeResources,
    pub allocated: NodeResources,
    pub max_pods: u32,
    pub current_pods: u32,
    pub os: String,
    pub arch: String,
    pub unschedulable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeResources {
    pub cpu_milli: u64,
    pub memory_bytes: u64,
    pub ephemeral_storage_bytes: u64,
    pub hugepages_2mi: u64,
    pub hugepages_1gi: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Taint {
    pub key: String,
    pub value: String,
    pub effect: TaintEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: TolerationOperator,
    pub value: Option<String>,
    pub effect: Option<TaintEffect>,
    pub toleration_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TolerationOperator {
    Equal,
    Exists,
}

impl Toleration {
    /// Whether this toleration matches the given taint per upstream
    /// `pkg/apis/core/v1/helper/helpers.go` `Toleration.MatchTaint`:
    ///   - Effect must match (None tolerates all effects).
    ///   - Operator=Exists matches any value (key may be empty to match all keys).
    ///   - Operator=Equal requires key+value match.
    pub fn matches(&self, taint: &Taint) -> bool {
        if let Some(e) = self.effect {
            if e != taint.effect {
                return false;
            }
        }
        match self.operator {
            TolerationOperator::Exists => match &self.key {
                None => true, // empty key + Exists tolerates all taints
                Some(k) if k.is_empty() => true,
                Some(k) => k == &taint.key,
            },
            TolerationOperator::Equal => match (&self.key, &self.value) {
                (Some(k), Some(v)) => k == &taint.key && v == &taint.value,
                _ => false,
            },
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAffinity {
    /// Required terms — ANDed groups, ORed across groups.
    pub required: Vec<NodeSelectorTerm>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSelectorTerm {
    pub match_expressions: Vec<NodeSelectorRequirement>,
    pub match_fields: Vec<NodeSelectorRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeSelectorRequirement {
    pub key: String,
    pub operator: NodeSelectorOperator,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeSelectorOperator {
    In,
    NotIn,
    Exists,
    DoesNotExist,
    Gt,
    Lt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodAdmission {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub node_name: Option<String>,
    pub node_selector: BTreeMap<String, String>,
    pub node_affinity: Option<NodeAffinity>,
    pub tolerations: Vec<Toleration>,
    pub cpu_request_milli: u64,
    pub memory_request_bytes: u64,
    pub ephemeral_storage_request_bytes: u64,
    pub hugepages_2mi_request: u64,
    pub hugepages_1gi_request: u64,
    pub host_network: bool,
    pub priority: i32,
    pub critical: bool,
    pub os: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionVerdict {
    Allowed,
    Denied(DenyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    NodeNameMismatch,
    Unschedulable,
    NodeSelectorMismatch,
    NodeAffinityMismatch,
    UntoleratedTaint,
    InsufficientCpu,
    InsufficientMemory,
    InsufficientEphemeralStorage,
    InsufficientHugepages,
    TooManyPods,
    OsMismatch,
    ArchMismatch,
}

/// Run all node-side admission predicates in upstream order.
pub fn admit(pod: &PodAdmission, node: &NodeInfo) -> AdmissionVerdict {
    if let Some(name) = &pod.node_name {
        if name != &node.name {
            return AdmissionVerdict::Denied(DenyReason::NodeNameMismatch);
        }
    }
    if node.unschedulable && !pod.critical {
        // node has Unschedulable=true; only critical pods bypass.
        // Treat as the implicit `node.kubernetes.io/unschedulable:NoSchedule`
        // taint — admission denies non-tolerating pods.
        let implicit = Taint {
            key: "node.kubernetes.io/unschedulable".into(),
            value: "".into(),
            effect: TaintEffect::NoSchedule,
        };
        if !pod.tolerations.iter().any(|t| t.matches(&implicit)) {
            return AdmissionVerdict::Denied(DenyReason::Unschedulable);
        }
    }
    if !pod.node_selector.is_empty() {
        let matches = pod
            .node_selector
            .iter()
            .all(|(k, v)| node.labels.get(k) == Some(v));
        if !matches {
            return AdmissionVerdict::Denied(DenyReason::NodeSelectorMismatch);
        }
    }
    if let Some(aff) = &pod.node_affinity {
        if !aff.required.is_empty() {
            let any = aff.required.iter().any(|term| term_matches(term, node));
            if !any {
                return AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch);
            }
        }
    }
    // Taints with NoSchedule / NoExecute must be tolerated (PreferNoSchedule
    // is advisory and ignored at admission).
    for t in &node.taints {
        if matches!(t.effect, TaintEffect::PreferNoSchedule) {
            continue;
        }
        if !pod.tolerations.iter().any(|tol| tol.matches(t)) {
            return AdmissionVerdict::Denied(DenyReason::UntoleratedTaint);
        }
    }
    // Resource fit.
    if pod.cpu_request_milli > 0
        && node.allocated.cpu_milli + pod.cpu_request_milli > node.allocatable.cpu_milli
    {
        return AdmissionVerdict::Denied(DenyReason::InsufficientCpu);
    }
    if pod.memory_request_bytes > 0
        && node.allocated.memory_bytes + pod.memory_request_bytes > node.allocatable.memory_bytes
    {
        return AdmissionVerdict::Denied(DenyReason::InsufficientMemory);
    }
    if pod.ephemeral_storage_request_bytes > 0
        && node.allocated.ephemeral_storage_bytes + pod.ephemeral_storage_request_bytes
            > node.allocatable.ephemeral_storage_bytes
    {
        return AdmissionVerdict::Denied(DenyReason::InsufficientEphemeralStorage);
    }
    if pod.hugepages_2mi_request > 0
        && node.allocated.hugepages_2mi + pod.hugepages_2mi_request > node.allocatable.hugepages_2mi
    {
        return AdmissionVerdict::Denied(DenyReason::InsufficientHugepages);
    }
    if pod.hugepages_1gi_request > 0
        && node.allocated.hugepages_1gi + pod.hugepages_1gi_request > node.allocatable.hugepages_1gi
    {
        return AdmissionVerdict::Denied(DenyReason::InsufficientHugepages);
    }
    if node.current_pods >= node.max_pods {
        return AdmissionVerdict::Denied(DenyReason::TooManyPods);
    }
    if let Some(o) = &pod.os {
        if o != &node.os {
            return AdmissionVerdict::Denied(DenyReason::OsMismatch);
        }
    }
    if let Some(a) = &pod.arch {
        if a != &node.arch {
            return AdmissionVerdict::Denied(DenyReason::ArchMismatch);
        }
    }
    AdmissionVerdict::Allowed
}

fn term_matches(term: &NodeSelectorTerm, node: &NodeInfo) -> bool {
    for r in &term.match_expressions {
        if !requirement_matches(r, &node.labels) {
            return false;
        }
    }
    // matchFields: kubelet supports `metadata.name` only.
    for r in &term.match_fields {
        if r.key == "metadata.name" {
            let mut node_field = BTreeMap::new();
            node_field.insert("metadata.name".into(), node.name.clone());
            if !requirement_matches(r, &node_field) {
                return false;
            }
        } else if !requirement_matches(r, &node.labels) {
            return false;
        }
    }
    !(term.match_expressions.is_empty() && term.match_fields.is_empty())
}

fn requirement_matches(
    r: &NodeSelectorRequirement,
    labels: &BTreeMap<String, String>,
) -> bool {
    let val = labels.get(&r.key);
    match r.operator {
        NodeSelectorOperator::In => match val {
            None => false,
            Some(v) => r.values.iter().any(|x| x == v),
        },
        NodeSelectorOperator::NotIn => match val {
            None => true,
            Some(v) => !r.values.iter().any(|x| x == v),
        },
        NodeSelectorOperator::Exists => val.is_some(),
        NodeSelectorOperator::DoesNotExist => val.is_none(),
        NodeSelectorOperator::Gt => match val.and_then(|v| v.parse::<i64>().ok()) {
            Some(actual) => r
                .values
                .first()
                .and_then(|s| s.parse::<i64>().ok())
                .map(|threshold| actual > threshold)
                .unwrap_or(false),
            None => false,
        },
        NodeSelectorOperator::Lt => match val.and_then(|v| v.parse::<i64>().ok()) {
            Some(actual) => r
                .values
                .first()
                .and_then(|s| s.parse::<i64>().ok())
                .map(|threshold| actual < threshold)
                .unwrap_or(false),
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str) -> NodeInfo {
        NodeInfo {
            name: name.into(),
            labels: BTreeMap::new(),
            taints: vec![],
            allocatable: NodeResources {
                cpu_milli: 8000,
                memory_bytes: 32 * 1024 * 1024 * 1024,
                ephemeral_storage_bytes: 100 * 1024 * 1024 * 1024,
                hugepages_2mi: 0,
                hugepages_1gi: 0,
            },
            allocated: NodeResources::default(),
            max_pods: 110,
            current_pods: 0,
            os: "linux".into(),
            arch: "amd64".into(),
            unschedulable: false,
        }
    }

    fn pod(name: &str) -> PodAdmission {
        PodAdmission {
            name: name.into(),
            namespace: "default".into(),
            uid: "u".into(),
            node_name: None,
            node_selector: BTreeMap::new(),
            node_affinity: None,
            tolerations: vec![],
            cpu_request_milli: 0,
            memory_request_bytes: 0,
            ephemeral_storage_request_bytes: 0,
            hugepages_2mi_request: 0,
            hugepages_1gi_request: 0,
            host_network: false,
            priority: 0,
            critical: false,
            os: None,
            arch: None,
        }
    }

    #[test]
    fn allows_simple_pod_on_empty_node() {
        let p = pod("p");
        let n = node("n");
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn denies_pod_when_node_name_mismatch() {
        let mut p = pod("p");
        p.node_name = Some("other".into());
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeNameMismatch)
        );
    }

    #[test]
    fn allows_pod_when_node_name_matches() {
        let mut p = pod("p");
        p.node_name = Some("n".into());
        let n = node("n");
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn denies_pod_on_unschedulable_node_without_toleration() {
        let p = pod("p");
        let mut n = node("n");
        n.unschedulable = true;
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::Unschedulable)
        );
    }

    #[test]
    fn allows_critical_pod_on_unschedulable_node() {
        let mut p = pod("p");
        p.critical = true;
        let mut n = node("n");
        n.unschedulable = true;
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn allows_pod_with_unschedulable_toleration() {
        let mut p = pod("p");
        p.tolerations = vec![Toleration {
            key: Some("node.kubernetes.io/unschedulable".into()),
            operator: TolerationOperator::Exists,
            value: None,
            effect: Some(TaintEffect::NoSchedule),
            toleration_seconds: None,
        }];
        let mut n = node("n");
        n.unschedulable = true;
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn node_selector_matches_label() {
        let mut p = pod("p");
        p.node_selector.insert("zone".into(), "us-west".into());
        let mut n = node("n");
        n.labels.insert("zone".into(), "us-west".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn node_selector_mismatch_denied() {
        let mut p = pod("p");
        p.node_selector.insert("zone".into(), "us-west".into());
        let mut n = node("n");
        n.labels.insert("zone".into(), "us-east".into());
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeSelectorMismatch)
        );
    }

    #[test]
    fn node_selector_label_missing_denied() {
        let mut p = pod("p");
        p.node_selector.insert("zone".into(), "us-west".into());
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeSelectorMismatch)
        );
    }

    #[test]
    fn taint_no_schedule_blocks_when_no_toleration() {
        let p = pod("p");
        let mut n = node("n");
        n.taints.push(Taint {
            key: "dedicated".into(),
            value: "gpu".into(),
            effect: TaintEffect::NoSchedule,
        });
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::UntoleratedTaint)
        );
    }

    #[test]
    fn taint_no_schedule_tolerated_with_equal() {
        let mut p = pod("p");
        p.tolerations = vec![Toleration {
            key: Some("dedicated".into()),
            operator: TolerationOperator::Equal,
            value: Some("gpu".into()),
            effect: Some(TaintEffect::NoSchedule),
            toleration_seconds: None,
        }];
        let mut n = node("n");
        n.taints.push(Taint {
            key: "dedicated".into(),
            value: "gpu".into(),
            effect: TaintEffect::NoSchedule,
        });
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn taint_no_schedule_tolerated_with_exists_no_key() {
        let mut p = pod("p");
        p.tolerations = vec![Toleration {
            key: None,
            operator: TolerationOperator::Exists,
            value: None,
            effect: None,
            toleration_seconds: None,
        }];
        let mut n = node("n");
        n.taints.push(Taint {
            key: "anything".into(),
            value: "v".into(),
            effect: TaintEffect::NoSchedule,
        });
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn taint_prefer_no_schedule_does_not_block_admission() {
        let p = pod("p");
        let mut n = node("n");
        n.taints.push(Taint {
            key: "soft".into(),
            value: "".into(),
            effect: TaintEffect::PreferNoSchedule,
        });
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn taint_no_execute_must_be_tolerated() {
        let p = pod("p");
        let mut n = node("n");
        n.taints.push(Taint {
            key: "evict".into(),
            value: "".into(),
            effect: TaintEffect::NoExecute,
        });
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::UntoleratedTaint)
        );
    }

    #[test]
    fn toleration_effect_mismatch_does_not_match() {
        let t = Toleration {
            key: Some("k".into()),
            operator: TolerationOperator::Equal,
            value: Some("v".into()),
            effect: Some(TaintEffect::NoSchedule),
            toleration_seconds: None,
        };
        let taint = Taint {
            key: "k".into(),
            value: "v".into(),
            effect: TaintEffect::NoExecute,
        };
        assert!(!t.matches(&taint));
    }

    #[test]
    fn toleration_effect_unset_matches_any_effect() {
        let t = Toleration {
            key: Some("k".into()),
            operator: TolerationOperator::Exists,
            value: None,
            effect: None,
            toleration_seconds: None,
        };
        let taint = Taint {
            key: "k".into(),
            value: "".into(),
            effect: TaintEffect::NoExecute,
        };
        assert!(t.matches(&taint));
    }

    #[test]
    fn toleration_equal_value_mismatch_rejects() {
        let t = Toleration {
            key: Some("k".into()),
            operator: TolerationOperator::Equal,
            value: Some("a".into()),
            effect: Some(TaintEffect::NoSchedule),
            toleration_seconds: None,
        };
        let taint = Taint {
            key: "k".into(),
            value: "b".into(),
            effect: TaintEffect::NoSchedule,
        };
        assert!(!t.matches(&taint));
    }

    #[test]
    fn node_affinity_required_in_match() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "tier".into(),
                    operator: NodeSelectorOperator::In,
                    values: vec!["frontend".into(), "edge".into()],
                }],
                match_fields: vec![],
            }],
        });
        let mut n = node("n");
        n.labels.insert("tier".into(), "edge".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn node_affinity_required_in_no_match() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "tier".into(),
                    operator: NodeSelectorOperator::In,
                    values: vec!["frontend".into()],
                }],
                match_fields: vec![],
            }],
        });
        let mut n = node("n");
        n.labels.insert("tier".into(), "backend".into());
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch)
        );
    }

    #[test]
    fn node_affinity_not_in_inverts() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "kind".into(),
                    operator: NodeSelectorOperator::NotIn,
                    values: vec!["spot".into()],
                }],
                match_fields: vec![],
            }],
        });
        let mut n = node("n");
        n.labels.insert("kind".into(), "ondemand".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
        n.labels.insert("kind".into(), "spot".into());
        assert!(matches!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch)
        ));
    }

    #[test]
    fn node_affinity_exists_passes_when_label_present() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "gpu".into(),
                    operator: NodeSelectorOperator::Exists,
                    values: vec![],
                }],
                match_fields: vec![],
            }],
        });
        let mut n = node("n");
        n.labels.insert("gpu".into(), "any".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn node_affinity_does_not_exist_passes_when_label_absent() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "gpu".into(),
                    operator: NodeSelectorOperator::DoesNotExist,
                    values: vec![],
                }],
                match_fields: vec![],
            }],
        });
        let n = node("n");
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn node_affinity_gt_lt_compare_numeric() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![NodeSelectorRequirement {
                    key: "memory-gib".into(),
                    operator: NodeSelectorOperator::Gt,
                    values: vec!["16".into()],
                }],
                match_fields: vec![],
            }],
        });
        let mut n = node("n");
        n.labels.insert("memory-gib".into(), "32".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
        n.labels.insert("memory-gib".into(), "8".into());
        assert!(matches!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch)
        ));
    }

    #[test]
    fn node_affinity_match_fields_metadata_name() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![],
                match_fields: vec![NodeSelectorRequirement {
                    key: "metadata.name".into(),
                    operator: NodeSelectorOperator::In,
                    values: vec!["node-a".into()],
                }],
            }],
        });
        let na = node("node-a");
        let nb = node("node-b");
        assert_eq!(admit(&p, &na), AdmissionVerdict::Allowed);
        assert!(matches!(
            admit(&p, &nb),
            AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch)
        ));
    }

    #[test]
    fn cpu_fit_denies_when_overcommit() {
        let mut p = pod("p");
        p.cpu_request_milli = 10_000;
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::InsufficientCpu)
        );
    }

    #[test]
    fn cpu_fit_includes_existing_allocations() {
        let mut p = pod("p");
        p.cpu_request_milli = 3000;
        let mut n = node("n");
        n.allocated.cpu_milli = 5000;
        // 5000 + 3000 = 8000 ≤ 8000 → fits.
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
        n.allocated.cpu_milli = 5001;
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::InsufficientCpu)
        );
    }

    #[test]
    fn memory_fit_denies_when_overcommit() {
        let mut p = pod("p");
        p.memory_request_bytes = 64 * 1024 * 1024 * 1024;
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::InsufficientMemory)
        );
    }

    #[test]
    fn ephemeral_storage_fit_check() {
        let mut p = pod("p");
        p.ephemeral_storage_request_bytes = 200 * 1024 * 1024 * 1024;
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::InsufficientEphemeralStorage)
        );
    }

    #[test]
    fn hugepages_2mi_fit_check() {
        let mut p = pod("p");
        p.hugepages_2mi_request = 1024;
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::InsufficientHugepages)
        );
    }

    #[test]
    fn hugepages_1gi_fit_check() {
        let mut p = pod("p");
        p.hugepages_1gi_request = 1024 * 1024 * 1024;
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::InsufficientHugepages)
        );
    }

    #[test]
    fn pod_count_denies_when_at_capacity() {
        let p = pod("p");
        let mut n = node("n");
        n.current_pods = 110;
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::TooManyPods)
        );
    }

    #[test]
    fn os_mismatch_denied() {
        let mut p = pod("p");
        p.os = Some("windows".into());
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::OsMismatch)
        );
    }

    #[test]
    fn arch_mismatch_denied() {
        let mut p = pod("p");
        p.arch = Some("arm64".into());
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::ArchMismatch)
        );
    }

    #[test]
    fn os_match_passes() {
        let mut p = pod("p");
        p.os = Some("linux".into());
        let n = node("n");
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn affinity_with_two_terms_or_semantics() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![
                NodeSelectorTerm {
                    match_expressions: vec![NodeSelectorRequirement {
                        key: "tier".into(),
                        operator: NodeSelectorOperator::In,
                        values: vec!["frontend".into()],
                    }],
                    match_fields: vec![],
                },
                NodeSelectorTerm {
                    match_expressions: vec![NodeSelectorRequirement {
                        key: "tier".into(),
                        operator: NodeSelectorOperator::In,
                        values: vec!["edge".into()],
                    }],
                    match_fields: vec![],
                },
            ],
        });
        let mut n = node("n");
        n.labels.insert("tier".into(), "edge".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn affinity_empty_term_does_not_match() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm::default()],
        });
        let n = node("n");
        assert_eq!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch)
        );
    }

    #[test]
    fn affinity_anded_within_term() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity {
            required: vec![NodeSelectorTerm {
                match_expressions: vec![
                    NodeSelectorRequirement {
                        key: "zone".into(),
                        operator: NodeSelectorOperator::In,
                        values: vec!["us-west".into()],
                    },
                    NodeSelectorRequirement {
                        key: "tier".into(),
                        operator: NodeSelectorOperator::In,
                        values: vec!["frontend".into()],
                    },
                ],
                match_fields: vec![],
            }],
        });
        let mut n = node("n");
        n.labels.insert("zone".into(), "us-west".into());
        n.labels.insert("tier".into(), "backend".into());
        assert!(matches!(
            admit(&p, &n),
            AdmissionVerdict::Denied(DenyReason::NodeAffinityMismatch)
        ));
        n.labels.insert("tier".into(), "frontend".into());
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn untoleratable_no_execute_with_correct_toleration_passes() {
        let mut p = pod("p");
        p.tolerations = vec![Toleration {
            key: Some("evict".into()),
            operator: TolerationOperator::Exists,
            value: None,
            effect: Some(TaintEffect::NoExecute),
            toleration_seconds: None,
        }];
        let mut n = node("n");
        n.taints.push(Taint {
            key: "evict".into(),
            value: "".into(),
            effect: TaintEffect::NoExecute,
        });
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn no_node_affinity_field_does_not_block() {
        let mut p = pod("p");
        p.node_affinity = Some(NodeAffinity { required: vec![] });
        let n = node("n");
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn cpu_zero_request_does_not_check_fit() {
        let p = pod("p");
        let mut n = node("n");
        n.allocated.cpu_milli = n.allocatable.cpu_milli;
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }

    #[test]
    fn memory_zero_request_does_not_check_fit() {
        let p = pod("p");
        let mut n = node("n");
        n.allocated.memory_bytes = n.allocatable.memory_bytes;
        assert_eq!(admit(&p, &n), AdmissionVerdict::Allowed);
    }
}
