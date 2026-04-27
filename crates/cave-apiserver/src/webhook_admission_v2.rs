//! Webhook admission v2 — deeper coverage of TLS dial state machine,
//! CABundle chain validation skeleton, deadline-driven timeout, match
//! condition evaluator wiring, reinvocation tracking.
//!
//! Layered atop M2 `webhook_admission`.
//!
//! Upstream sources (kubernetes/kubernetes v1.31+):
//!   * `staging/src/k8s.io/apiserver/pkg/util/webhook/webhook.go`
//!     (`Authentic.NewClient` — TLS dial + per-request deadline).
//!   * `staging/src/k8s.io/apiserver/pkg/util/webhook/serviceresolver.go`
//!     (Service ↔ URL resolution).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/matchconditions/matcher.go`
//!     (CEL match-conditions matcher).
//!   * `staging/src/k8s.io/apiserver/pkg/admission/plugin/webhook/mutating/plugin.go`
//!     (reinvocation policy state across mutation rounds).

use crate::admission::AdmissionRequest;
use crate::vap_advanced::{CelEvaluator, CelActivation, CelValue, CelError};
use crate::webhook_admission::{
    FailurePolicyType, MatchCondition, ServiceReference, WebhookClientConfig,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// TLS dial state machine — a tiny FSM the connection layer steps through.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialState {
    /// Initial — about to resolve.
    Pending,
    /// DNS / Service resolved to address; about to connect.
    Resolved,
    /// TCP connected; about to handshake.
    Connected,
    /// TLS handshake in progress.
    Handshaking,
    /// Handshake completed, sending request.
    Established,
    /// Failed at one of the steps.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialEvent {
    Resolve,
    Connect,
    Handshake,
    Complete,
    Fail(String),
}

pub struct DialFsm {
    pub state: DialState,
    /// Recorded transitions for testing.
    pub transitions: Vec<(DialState, DialEvent, DialState)>,
}

impl DialFsm {
    pub fn new() -> Self {
        Self { state: DialState::Pending, transitions: vec![] }
    }
    pub fn step(&mut self, event: DialEvent) -> Result<(), String> {
        let next = match (self.state, &event) {
            (DialState::Pending, DialEvent::Resolve) => DialState::Resolved,
            (DialState::Resolved, DialEvent::Connect) => DialState::Connected,
            (DialState::Connected, DialEvent::Handshake) => DialState::Handshaking,
            (DialState::Handshaking, DialEvent::Complete) => DialState::Established,
            (_, DialEvent::Fail(_)) => DialState::Failed,
            (s, e) => return Err(format!("invalid transition {s:?} → {e:?}")),
        };
        self.transitions.push((self.state, event, next));
        self.state = next;
        Ok(())
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self.state, DialState::Established | DialState::Failed)
    }
}

impl Default for DialFsm { fn default() -> Self { Self::new() } }

// ─────────────────────────────────────────────────────────────────────────────
// Service resolver — `serviceresolver.go::ResolveEndpoint`. Maps a
// ServiceReference to a https://<host>:<port><path> URL using the cluster
// service domain.
// ─────────────────────────────────────────────────────────────────────────────

pub trait ServiceResolver: Send + Sync {
    fn resolve(&self, service: &ServiceReference) -> Result<String, String>;
}

pub struct ClusterServiceResolver {
    pub cluster_domain: String, // typically "cluster.local"
}

impl ServiceResolver for ClusterServiceResolver {
    fn resolve(&self, service: &ServiceReference) -> Result<String, String> {
        if service.name.is_empty() {
            return Err("service.name empty".into());
        }
        if service.namespace.is_empty() {
            return Err("service.namespace empty".into());
        }
        let port = service.port.unwrap_or(443);
        if !(1..=65535).contains(&port) {
            return Err(format!("invalid port {port}"));
        }
        let path = service.path.as_deref().unwrap_or("");
        let path = if path.is_empty() || path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        Ok(format!(
            "https://{}.{}.svc.{}:{}{}",
            service.name, service.namespace, self.cluster_domain, port, path,
        ))
    }
}

pub fn resolve_endpoint(
    resolver: &dyn ServiceResolver, cfg: &WebhookClientConfig,
) -> Result<String, String> {
    if let Some(url) = &cfg.url { return Ok(url.clone()); }
    let Some(svc) = &cfg.service else {
        return Err("clientConfig has neither url nor service".into());
    };
    resolver.resolve(svc)
}

// ─────────────────────────────────────────────────────────────────────────────
// CABundle chain validation — model layer. We accept a list of (parent,
// child) edges and compute whether `subject` chains up to a trusted root.
// Real X.509 verification lands in a follow-up; the model captures the
// algorithmic frame.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CertNode {
    pub subject: String,
    pub issuer: String,
    pub is_root: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainOutcome {
    Trusted { depth: usize },
    Untrusted(String),
    Cycle,
}

pub fn verify_chain(
    leaf_subject: &str, certs: &[CertNode], trusted_roots: &[String],
) -> ChainOutcome {
    let by_subject: HashMap<&str, &CertNode> =
        certs.iter().map(|c| (c.subject.as_str(), c)).collect();
    let trusted: std::collections::HashSet<&str> =
        trusted_roots.iter().map(|s| s.as_str()).collect();
    let mut current = leaf_subject;
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut depth = 0usize;
    while !trusted.contains(current) {
        if !visited.insert(current) { return ChainOutcome::Cycle; }
        let Some(node) = by_subject.get(current) else {
            return ChainOutcome::Untrusted(format!("missing intermediate for {current}"));
        };
        if node.is_root && !trusted.contains(node.subject.as_str()) {
            return ChainOutcome::Untrusted(format!("untrusted root {}", node.subject));
        }
        if node.subject == node.issuer { break; }
        current = node.issuer.as_str();
        depth += 1;
        if depth > 16 { return ChainOutcome::Untrusted("chain too long".into()); }
    }
    if trusted.contains(current) {
        ChainOutcome::Trusted { depth }
    } else {
        ChainOutcome::Untrusted("did not reach a trusted root".into())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Deadline — converts `timeoutSeconds` into an `Instant` and exposes
// `remaining`. Used by callers driving real async IO.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Deadline {
    pub deadline: Instant,
}

impl Deadline {
    pub fn from_seconds(s: i32) -> Self {
        let s = s.max(1) as u64;
        Self { deadline: Instant::now() + Duration::from_secs(s) }
    }
    pub fn from_millis(ms: u64) -> Self {
        Self { deadline: Instant::now() + Duration::from_millis(ms) }
    }
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline.checked_duration_since(Instant::now())
    }
    pub fn expired(&self) -> bool { self.remaining().is_none() }
}

// ─────────────────────────────────────────────────────────────────────────────
// MatchConditions matcher — drives the per-webhook CEL match. Mirrors
// `matchconditions/matcher.go::Match`.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchOutcome {
    Matched,
    /// Any condition returned false → skip the webhook.
    NoMatch,
    /// Any condition returned non-bool, or evaluator errored AND failure
    /// policy is Fail. Carries the offending condition name + reason.
    Fail { name: String, reason: String },
    /// Errored under FailurePolicy=Ignore — webhook is treated as
    /// not-matched but the apiserver records this for audit.
    SilencedError { name: String },
}

pub struct MatchConditionsMatcher {
    pub evaluator: Arc<dyn CelEvaluator>,
}

impl MatchConditionsMatcher {
    pub fn match_(
        &self,
        conditions: &[MatchCondition],
        failure_policy: &FailurePolicyType,
        req: &AdmissionRequest,
    ) -> MatchOutcome {
        let act = CelActivation {
            request: serde_json::to_value(req).ok(),
            object: req.object.as_ref().and_then(|o| serde_json::to_value(o).ok()),
            old_object: req.old_object.as_ref().and_then(|o| serde_json::to_value(o).ok()),
            params: None, namespace_object: None,
            variables: HashMap::new(),
        };
        for c in conditions {
            match self.evaluator.evaluate(&c.expression, &act) {
                Ok(CelValue::Bool(true)) => continue,
                Ok(CelValue::Bool(false)) => return MatchOutcome::NoMatch,
                Ok(other) => return MatchOutcome::Fail {
                    name: c.name.clone(),
                    reason: format!("non-bool result {:?}", other),
                },
                Err(e) => {
                    let reason = match &e {
                        CelError::Compile(m) => m.clone(),
                        CelError::Runtime(m) => m.clone(),
                        CelError::Type { expected, got } =>
                            format!("type error: expected {expected}, got {got}"),
                    };
                    return match failure_policy {
                        FailurePolicyType::Fail => MatchOutcome::Fail {
                            name: c.name.clone(), reason,
                        },
                        FailurePolicyType::Ignore => MatchOutcome::SilencedError {
                            name: c.name.clone(),
                        },
                    };
                }
            }
        }
        MatchOutcome::Matched
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Reinvocation tracker — `mutating/plugin.go::reinvocationContext`. Tracks
// the set of webhooks already invoked in a single dispatch, plus whether
// the object was mutated by a later webhook (which marks every prior
// IfNeeded webhook for re-invocation).
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ReinvocationContext {
    inner: Mutex<ReinvocationInner>,
}

#[derive(Default)]
struct ReinvocationInner {
    invoked: Vec<String>,
    object_modified_after: HashMap<String, bool>,
}

impl ReinvocationContext {
    pub fn new() -> Self { Self::default() }
    /// Mark a webhook as invoked. Returns true if this is a re-invocation.
    pub fn mark_invoked(&self, name: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        let already = g.invoked.contains(&name.to_string());
        if !already { g.invoked.push(name.to_string()); }
        already
    }
    /// Record that the object was modified after this webhook.
    pub fn mark_modified_after(&self, after: &str) {
        let g = self.inner.lock().unwrap();
        let names: Vec<String> = g.invoked.clone();
        drop(g);
        let mut g = self.inner.lock().unwrap();
        for n in names {
            if n == after { break; }
            g.object_modified_after.insert(n, true);
        }
    }
    /// `true` if the named webhook should run a second time.
    pub fn should_reinvoke(&self, name: &str) -> bool {
        self.inner.lock().unwrap().object_modified_after.get(name).copied().unwrap_or(false)
    }
    pub fn invoked_count(&self) -> usize {
        self.inner.lock().unwrap().invoked.len()
    }
}

#[cfg(test)]
mod tests;
