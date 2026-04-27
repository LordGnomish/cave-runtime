//! Extender API — out-of-tree scheduler extensions over HTTP.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/extender.go
//!   pkg/scheduler/apis/config/types.go (Extender, ExtenderConfig)
//!
//! ## Surface
//!
//! Each extender configures up to four verbs:
//! - **Filter** (`POST /<filter_verb>`): receives `(pod, nodes)`; returns
//!   the surviving subset plus per-node failure reasons.
//! - **Prioritize** (`POST /<prioritize_verb>`): receives `(pod, nodes)`;
//!   returns `(node → score)` in `[0, 100]`. The extender has a `weight`
//!   that the scheduler multiplies into the score.
//! - **Preempt** (`POST /<preempt_verb>`): receives `(pod, candidate_node →
//!   victims)`; returns a curated victim map (extender may veto certain
//!   evictions).
//! - **Bind** (`POST /<bind_verb>`): receives `(pod, target_node)`; returns
//!   `Status` (Success / Error). Only one extender binds.
//!
//! ## Ignorable + fallback
//!
//! `ignorable=true` extenders that error out are *skipped* — the scheduler
//! continues with the in-tree result. `ignorable=false` extenders fail the
//! cycle on error.
//!
//! ## ManagedResources
//!
//! Each extender declares which extended resource names it owns. A pod
//! requesting only resources listed in *some* extender's
//! `ignoredByScheduler` set bypasses the in-tree NodeResourcesFit check
//! for those names — upstream invariant we surface as
//! `Extender::owns_resource_for_scheduler_skip`.

use crate::cycle_state::CycleState;
use crate::extension_points::{BindPlugin, NodeToStatusMap, PostFilterPlugin, PostFilterResult};
use crate::framework::{ClusterSnapshot, FilterPlugin, Pod, ScorePlugin, Status};
use crate::models::Node;
use crate::preempt::PreemptionResult;
use std::collections::HashMap;
use std::sync::Arc;

/// One extender's static configuration.
#[derive(Debug, Clone)]
pub struct ExtenderConfig {
    pub name: String,
    pub url_prefix: String,
    pub filter_verb: Option<String>,
    pub prioritize_verb: Option<String>,
    pub preempt_verb: Option<String>,
    pub bind_verb: Option<String>,
    /// Score weight (defaults to 1).
    pub weight: u32,
    /// Errors are tolerable when true; the scheduler skips this extender on
    /// transport / 5xx failures.
    pub ignorable: bool,
    /// Extended resource names owned by this extender. When `ignored_by_scheduler`
    /// is true for a name, the in-tree NodeResourcesFit will skip it.
    pub managed_resources: Vec<ManagedResource>,
    /// Optional HTTP timeout (informational; honored by HTTP impl).
    pub http_timeout_ms: u64,
    /// Whether to use HTTPS.
    pub enable_https: bool,
}

#[derive(Debug, Clone)]
pub struct ManagedResource {
    pub name: String,
    pub ignored_by_scheduler: bool,
}

impl Default for ExtenderConfig {
    fn default() -> Self {
        Self {
            name: "extender".into(),
            url_prefix: String::new(),
            filter_verb: None, prioritize_verb: None,
            preempt_verb: None, bind_verb: None,
            weight: 1, ignorable: false,
            managed_resources: Vec::new(),
            http_timeout_ms: 5_000,
            enable_https: false,
        }
    }
}

impl ExtenderConfig {
    /// True when this extender owns the named resource and asks the
    /// scheduler to skip the in-tree fit check for it.
    pub fn owns_resource_for_scheduler_skip(&self, name: &str) -> bool {
        self.managed_resources.iter()
            .any(|r| r.name == name && r.ignored_by_scheduler)
    }
}

#[derive(Debug, Clone)]
pub struct ExtenderFilterResponse {
    /// Names of nodes that passed the extender filter.
    pub passing_nodes: Vec<String>,
    /// Per-node failure messages for nodes that did NOT pass.
    pub failed_nodes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ExtenderPrioritizeResponse {
    /// `node_name → raw_score` in `[0, 100]`. The extender's `weight` is
    /// applied later by the [`Extender`] wrapper.
    pub scores: HashMap<String, i64>,
}

/// Extender transport. Implementations call the extender HTTP API; tests
/// supply mocks.
pub trait ExtenderClient: Send + Sync {
    fn filter(&self, pod: &Pod, nodes: &[Node]) -> Result<ExtenderFilterResponse, String>;
    fn prioritize(&self, pod: &Pod, nodes: &[Node]) -> Result<ExtenderPrioritizeResponse, String>;
    fn preempt(
        &self,
        pod: &Pod,
        plan: &PreemptionResult,
    ) -> Result<PreemptionResult, String>;
    fn bind(&self, pod: &Pod, node: &str) -> Result<Status, String>;
}

/// Plugin wrapping an extender as a Filter / Score / PostFilter / Bind.
pub struct Extender {
    pub config: ExtenderConfig,
    pub client: Arc<dyn ExtenderClient>,
}

impl Extender {
    pub fn new(config: ExtenderConfig, client: Arc<dyn ExtenderClient>) -> Self {
        Self { config, client }
    }

    fn handle_err(&self, plugin_method: &str, err: String) -> Status {
        if self.config.ignorable {
            Status::skip(&self.config.name)
        } else {
            Status::error(&self.config.name, format!("{}: {}", plugin_method, err))
        }
    }
}

impl FilterPlugin for Extender {
    fn name(&self) -> &str { &self.config.name }
    fn filter(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> Status {
        if self.config.filter_verb.is_none() {
            return Status::skip(&self.config.name);
        }
        // Bulk-call once per (pod, snapshot.nodes) and reuse the result for
        // every node in the cycle. We approximate by per-node calls (one-shot
        // schedulers in tests don't notice the chattiness).
        match self.client.filter(pod, &snap.nodes) {
            Ok(resp) => {
                if resp.passing_nodes.iter().any(|n| n == &node.name) {
                    Status::success(&self.config.name)
                } else if let Some(reason) = resp.failed_nodes.get(&node.name) {
                    Status::unschedulable(&self.config.name, reason.clone())
                } else {
                    Status::unschedulable(&self.config.name, "extender did not pass node")
                }
            }
            Err(e) => self.handle_err("filter", e),
        }
    }
}

impl ScorePlugin for Extender {
    fn name(&self) -> &str { &self.config.name }
    fn score(&self, pod: &Pod, node: &Node, snap: &ClusterSnapshot) -> i64 {
        if self.config.prioritize_verb.is_none() {
            return 0;
        }
        match self.client.prioritize(pod, &snap.nodes) {
            Ok(resp) => {
                let raw = resp.scores.get(&node.name).copied().unwrap_or(0);
                raw * self.config.weight as i64
            }
            Err(_) => 0,
        }
    }
}

impl PostFilterPlugin for Extender {
    fn name(&self) -> &str { &self.config.name }
    fn post_filter(
        &self,
        pod: &Pod,
        snapshot: &ClusterSnapshot,
        _filtered: &NodeToStatusMap,
        _state: &CycleState,
    ) -> (PostFilterResult, Status) {
        if self.config.preempt_verb.is_none() {
            return (PostFilterResult::default(), Status::unschedulable(&self.config.name, "extender has no preempt verb"));
        }
        // Build a no-op base plan and let the extender curate it. Real impl
        // would compute victims first via in-tree preempt; mocks pass through.
        let base = PreemptionResult {
            nominated_node_name: snapshot.nodes.first().map(|n| n.name.clone()).unwrap_or_default(),
            victims: vec![],
            pdb_violations: 0,
        };
        match self.client.preempt(pod, &base) {
            Ok(curated) => {
                if curated.nominated_node_name.is_empty() {
                    (PostFilterResult::default(), Status::unschedulable(&self.config.name, "extender vetoed preemption"))
                } else {
                    (
                        PostFilterResult::nominate(curated.nominated_node_name.clone()),
                        Status::success(&self.config.name),
                    )
                }
            }
            Err(e) => (PostFilterResult::default(), self.handle_err("preempt", e)),
        }
    }
}

impl BindPlugin for Extender {
    fn name(&self) -> &str { &self.config.name }
    fn bind(&self, pod: &Pod, node: &str, _: &CycleState) -> Status {
        if self.config.bind_verb.is_none() {
            return Status::skip(&self.config.name);
        }
        match self.client.bind(pod, node) {
            Ok(s) => s,
            Err(e) => self.handle_err("bind", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Pod;
    use crate::models::{NodeStatus, ResourceCapacity};
    use chrono::Utc;
    use std::sync::Mutex;
    use uuid::Uuid;

    fn ready_node(name: &str) -> Node {
        Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity::default(),
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    fn snap(nodes: Vec<Node>) -> ClusterSnapshot {
        ClusterSnapshot { nodes, pods_by_node: HashMap::new() }
    }

    /// Programmable mock — every method returns whatever was last `set_*`'d.
    struct MockClient {
        filter_resp: Mutex<Result<ExtenderFilterResponse, String>>,
        prio_resp: Mutex<Result<ExtenderPrioritizeResponse, String>>,
        preempt_resp: Mutex<Result<PreemptionResult, String>>,
        bind_resp: Mutex<Result<Status, String>>,
        bind_calls: Mutex<u32>,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                filter_resp: Mutex::new(Ok(ExtenderFilterResponse {
                    passing_nodes: vec![], failed_nodes: HashMap::new(),
                })),
                prio_resp: Mutex::new(Ok(ExtenderPrioritizeResponse { scores: HashMap::new() })),
                preempt_resp: Mutex::new(Ok(PreemptionResult {
                    nominated_node_name: String::new(), victims: vec![], pdb_violations: 0,
                })),
                bind_resp: Mutex::new(Ok(Status::success("mock"))),
                bind_calls: Mutex::new(0),
            }
        }
    }

    impl ExtenderClient for MockClient {
        fn filter(&self, _: &Pod, _: &[Node]) -> Result<ExtenderFilterResponse, String> {
            self.filter_resp.lock().unwrap().clone()
        }
        fn prioritize(&self, _: &Pod, _: &[Node]) -> Result<ExtenderPrioritizeResponse, String> {
            self.prio_resp.lock().unwrap().clone()
        }
        fn preempt(&self, _: &Pod, _: &PreemptionResult) -> Result<PreemptionResult, String> {
            self.preempt_resp.lock().unwrap().clone()
        }
        fn bind(&self, _: &Pod, _: &str) -> Result<Status, String> {
            *self.bind_calls.lock().unwrap() += 1;
            self.bind_resp.lock().unwrap().clone()
        }
    }

    fn cfg(name: &str) -> ExtenderConfig {
        ExtenderConfig { name: name.into(), filter_verb: Some("filter".into()),
            prioritize_verb: Some("prioritize".into()), preempt_verb: Some("preempt".into()),
            bind_verb: Some("bind".into()), ..Default::default() }
    }

    // ── ExtenderConfig ────────────────────────────────────────────────────

    #[test]
    fn managed_resource_skip_flag() {
        let mut c = ExtenderConfig::default();
        c.managed_resources = vec![
            ManagedResource { name: "nvidia.com/gpu".into(), ignored_by_scheduler: true },
            ManagedResource { name: "smarter-devices/fuse".into(), ignored_by_scheduler: false },
        ];
        assert!(c.owns_resource_for_scheduler_skip("nvidia.com/gpu"));
        assert!(!c.owns_resource_for_scheduler_skip("smarter-devices/fuse"));
        assert!(!c.owns_resource_for_scheduler_skip("ghost"));
    }

    #[test]
    fn config_defaults_match_upstream() {
        let c = ExtenderConfig::default();
        assert_eq!(c.weight, 1);
        assert!(!c.ignorable);
        assert!(c.filter_verb.is_none());
        assert_eq!(c.http_timeout_ms, 5_000);
        assert!(!c.enable_https);
    }

    // ── Filter ────────────────────────────────────────────────────────────

    #[test]
    fn filter_passes_when_extender_passes() {
        let mock = Arc::new(MockClient::new());
        *mock.filter_resp.lock().unwrap() = Ok(ExtenderFilterResponse {
            passing_nodes: vec!["a".into(), "b".into()], failed_nodes: HashMap::new(),
        });
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        let s = snap(vec![ready_node("a"), ready_node("b")]);
        assert!(ext.filter(&p, &ready_node("a"), &s).is_success());
    }

    #[test]
    fn filter_rejects_with_extender_reason() {
        let mock = Arc::new(MockClient::new());
        let mut failed = HashMap::new();
        failed.insert("a".into(), "wrong-zone".into());
        *mock.filter_resp.lock().unwrap() = Ok(ExtenderFilterResponse {
            passing_nodes: vec![], failed_nodes: failed,
        });
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        let s = ext.filter(&p, &ready_node("a"), &snap(vec![ready_node("a")]));
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("wrong-zone"));
    }

    #[test]
    fn filter_no_verb_is_skip() {
        let mock = Arc::new(MockClient::new());
        let mut c = cfg("ext");
        c.filter_verb = None;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let s = ext.filter(&p, &ready_node("a"), &snap(vec![]));
        assert!(s.is_skip());
    }

    #[test]
    fn filter_error_with_ignorable_returns_skip() {
        let mock = Arc::new(MockClient::new());
        *mock.filter_resp.lock().unwrap() = Err("transport".into());
        let mut c = cfg("ext");
        c.ignorable = true;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let s = ext.filter(&p, &ready_node("a"), &snap(vec![]));
        assert!(s.is_skip());
    }

    #[test]
    fn filter_error_without_ignorable_returns_error() {
        let mock = Arc::new(MockClient::new());
        *mock.filter_resp.lock().unwrap() = Err("network".into());
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        let s = ext.filter(&p, &ready_node("a"), &snap(vec![]));
        assert!(s.is_error());
        assert!(s.reasons[0].contains("network"));
    }

    // ── Score ─────────────────────────────────────────────────────────────

    #[test]
    fn prioritize_multiplies_by_weight() {
        let mock = Arc::new(MockClient::new());
        let mut scores = HashMap::new();
        scores.insert("a".into(), 50);
        *mock.prio_resp.lock().unwrap() = Ok(ExtenderPrioritizeResponse { scores });
        let mut c = cfg("ext");
        c.weight = 3;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let s = snap(vec![ready_node("a")]);
        assert_eq!(ext.score(&p, &ready_node("a"), &s), 150);
    }

    #[test]
    fn prioritize_no_verb_returns_zero() {
        let mock = Arc::new(MockClient::new());
        let mut c = cfg("ext");
        c.prioritize_verb = None;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        assert_eq!(ext.score(&p, &ready_node("a"), &snap(vec![])), 0);
    }

    #[test]
    fn prioritize_unknown_node_returns_zero() {
        let mock = Arc::new(MockClient::new());
        *mock.prio_resp.lock().unwrap() = Ok(ExtenderPrioritizeResponse { scores: HashMap::new() });
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        assert_eq!(ext.score(&p, &ready_node("a"), &snap(vec![])), 0);
    }

    #[test]
    fn prioritize_error_returns_zero_silently() {
        let mock = Arc::new(MockClient::new());
        *mock.prio_resp.lock().unwrap() = Err("boom".into());
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        // Score plugin can't return Status; errors silently produce 0.
        assert_eq!(ext.score(&p, &ready_node("a"), &snap(vec![])), 0);
    }

    // ── Preempt (PostFilter) ──────────────────────────────────────────────

    #[test]
    fn preempt_returns_extender_curated_plan() {
        let mock = Arc::new(MockClient::new());
        *mock.preempt_resp.lock().unwrap() = Ok(PreemptionResult {
            nominated_node_name: "node-x".into(),
            victims: vec![], pdb_violations: 0,
        });
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        let (res, st) = ext.post_filter(&p, &snap(vec![ready_node("a")]), &NodeToStatusMap::new(), &cs);
        assert!(st.is_success());
        assert_eq!(res.nominating_info.unwrap().nominated_node_name, "node-x");
    }

    #[test]
    fn preempt_veto_is_unschedulable() {
        let mock = Arc::new(MockClient::new());
        *mock.preempt_resp.lock().unwrap() = Ok(PreemptionResult {
            nominated_node_name: String::new(),
            victims: vec![], pdb_violations: 0,
        });
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        let (res, st) = ext.post_filter(&p, &snap(vec![ready_node("a")]), &NodeToStatusMap::new(), &cs);
        assert!(res.nominating_info.is_none());
        assert!(st.is_rejected());
    }

    #[test]
    fn preempt_no_verb_is_unschedulable() {
        let mock = Arc::new(MockClient::new());
        let mut c = cfg("ext");
        c.preempt_verb = None;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        let (_, st) = ext.post_filter(&p, &snap(vec![]), &NodeToStatusMap::new(), &cs);
        assert!(st.is_rejected());
    }

    #[test]
    fn preempt_error_with_ignorable_returns_skip() {
        let mock = Arc::new(MockClient::new());
        *mock.preempt_resp.lock().unwrap() = Err("net".into());
        let mut c = cfg("ext"); c.ignorable = true;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        let (_, st) = ext.post_filter(&p, &snap(vec![]), &NodeToStatusMap::new(), &cs);
        assert!(st.is_skip());
    }

    // ── Bind ──────────────────────────────────────────────────────────────

    #[test]
    fn bind_returns_extender_status() {
        let mock = Arc::new(MockClient::new());
        *mock.bind_resp.lock().unwrap() = Ok(Status::success("ok"));
        let ext = Extender::new(cfg("ext"), mock.clone());
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        assert!(ext.bind(&p, "n1", &cs).is_success());
        assert_eq!(*mock.bind_calls.lock().unwrap(), 1);
    }

    #[test]
    fn bind_no_verb_is_skip() {
        let mock = Arc::new(MockClient::new());
        let mut c = cfg("ext"); c.bind_verb = None;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        assert!(ext.bind(&p, "n", &cs).is_skip());
    }

    #[test]
    fn bind_error_ignorable_skip() {
        let mock = Arc::new(MockClient::new());
        *mock.bind_resp.lock().unwrap() = Err("transport".into());
        let mut c = cfg("ext"); c.ignorable = true;
        let ext = Extender::new(c, mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        assert!(ext.bind(&p, "n", &cs).is_skip());
    }

    #[test]
    fn bind_error_non_ignorable_propagates_error() {
        let mock = Arc::new(MockClient::new());
        *mock.bind_resp.lock().unwrap() = Err("transport".into());
        let ext = Extender::new(cfg("ext"), mock);
        let p = Pod::new("t", "ns", "p");
        let cs = CycleState::new();
        let s = ext.bind(&p, "n", &cs);
        assert!(s.is_error());
        assert!(s.reasons[0].contains("transport"));
    }

    // ── Framework integration ────────────────────────────────────────────

    #[test]
    fn extender_in_framework_filter_chain_short_circuits() {
        use crate::framework::Framework;
        let mock = Arc::new(MockClient::new());
        let mut failed = HashMap::new();
        failed.insert("a".into(), "ext-reject".into());
        *mock.filter_resp.lock().unwrap() = Ok(ExtenderFilterResponse {
            passing_nodes: vec![], failed_nodes: failed,
        });
        let ext = Extender::new(cfg("ext"), mock);
        let fw = Framework::new().with_filter(Box::new(ext));
        let s = fw.run_filters(&Pod::new("t", "ns", "p"), &snap(vec![ready_node("a")]));
        let v = s.get("a").unwrap();
        assert!(v.is_some());
        assert_eq!(v.as_ref().unwrap().plugin, "ext");
    }

    #[test]
    fn fallback_when_extender_ignorable_filter_errs() {
        use crate::framework::Framework;
        let mock = Arc::new(MockClient::new());
        *mock.filter_resp.lock().unwrap() = Err("net".into());
        let mut c = cfg("ext"); c.ignorable = true;
        let ext = Extender::new(c, mock);
        let fw = Framework::new().with_filter(Box::new(ext));
        // Skip-result is treated as success in run_filters → node passes.
        let s = fw.run_filters(&Pod::new("t", "ns", "p"), &snap(vec![ready_node("a")]));
        assert!(s.get("a").unwrap().is_none(), "fallback: ignorable extender error → node still passes");
    }

    #[test]
    fn extender_score_in_framework_uses_weight() {
        use crate::framework::Framework;
        let mock = Arc::new(MockClient::new());
        let mut scores = HashMap::new();
        scores.insert("a".into(), 50);
        *mock.prio_resp.lock().unwrap() = Ok(ExtenderPrioritizeResponse { scores });
        let mut c = cfg("ext"); c.weight = 4;
        let ext = Extender::new(c, mock);
        // Wrap inside a struct so we don't move out of the unique-ownership ext.
        let fw = Framework::new().with_score(Box::new(ext));
        let scores = fw.run_scores(&Pod::new("t", "ns", "p"), &["a".into()], &snap(vec![ready_node("a")]));
        // Note: framework.run_scores clamps each per-plugin score to MAX_NODE_SCORE
        // before applying ScoringWeights; here weight is in the extender, so
        // raw is 200 → clamped to 100.
        let s = *scores.get("a").unwrap();
        assert_eq!(s, 100, "extender weight folded into raw, then framework clamps");
    }
}
