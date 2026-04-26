//! Pod Security admission (PSA) — KEP-2579.
//!
//! Upstream sources (kubernetes/kubernetes v1.31):
//!   * `staging/src/k8s.io/pod-security-admission/api/types.go`
//!     (Level, Mode, version labels).
//!   * `staging/src/k8s.io/pod-security-admission/admission/admission.go`.
//!   * `staging/src/k8s.io/pod-security-admission/policy/check_*.go`
//!     (per-control checks).
//!
//! ## Levels
//!
//! - `privileged`: no checks.
//! - `baseline`: blocks the most-egregious controls (host namespaces,
//!   privileged, hostPath, hostPort, allowPrivilegeEscalation default,
//!   etc.).
//! - `restricted`: baseline + tighter (must drop ALL caps, runAsNonRoot,
//!   seccompProfile, etc.).
//!
//! ## Modes
//!
//! - `enforce`: deny on violation.
//! - `audit`: emit audit annotation; never deny.
//! - `warn`: append warning to response; never deny.
//!
//! ## Tenant invariant
//!
//! Levels are namespace-scoped (label-driven). Two tenants with the same
//! namespace name MUST get different levels because labels are stored on
//! the per-tenant Namespace object. A `NamespaceLevelLister` always
//! resolves with the request's tenant_id.

use crate::admission::{
    AdmissionRequest, AdmissionResponse, Operation, ValidatingWebhook,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Privileged,
    Baseline,
    Restricted,
}

impl Default for Level {
    fn default() -> Self { Level::Privileged }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Enforce,
    Audit,
    Warn,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LevelTriple {
    pub enforce: Option<Level>,
    pub audit: Option<Level>,
    pub warn: Option<Level>,
}

/// Parse a single namespace label set into a `LevelTriple`.
///
/// Recognised labels (KEP-2579):
///   * `pod-security.kubernetes.io/enforce`
///   * `pod-security.kubernetes.io/audit`
///   * `pod-security.kubernetes.io/warn`
///   * + `*-version` labels (mapped here as no-op, since we treat policy
///     as version-agnostic.)
pub fn parse_namespace_labels(labels: &HashMap<String, String>) -> LevelTriple {
    LevelTriple {
        enforce: labels.get("pod-security.kubernetes.io/enforce").and_then(|s| parse_level(s)),
        audit: labels.get("pod-security.kubernetes.io/audit").and_then(|s| parse_level(s)),
        warn: labels.get("pod-security.kubernetes.io/warn").and_then(|s| parse_level(s)),
    }
}

pub fn parse_level(s: &str) -> Option<Level> {
    match s {
        "privileged" => Some(Level::Privileged),
        "baseline" => Some(Level::Baseline),
        "restricted" => Some(Level::Restricted),
        _ => None,
    }
}

/// Pod-security spec (a flattened subset of `corev1.PodSpec.SecurityContext` +
/// per-container `SecurityContext`). Real implementations populate this from
/// the admitted Pod object; we accept it as a struct so the per-control
/// checks are independent of the resource model.
#[derive(Debug, Clone, Default)]
pub struct PodSecuritySpec {
    pub host_network: bool,
    pub host_pid: bool,
    pub host_ipc: bool,
    pub host_path_volumes: Vec<String>, // hostPath sources
    pub host_ports: Vec<i32>,
    pub privileged_containers: Vec<String>,
    pub allow_privilege_escalation: HashMap<String, bool>, // container → APE
    pub run_as_non_root: HashMap<String, Option<bool>>, // None == unset
    pub run_as_user: HashMap<String, i64>,
    pub seccomp_profile: HashMap<String, String>, // container → profile type
    pub capabilities_add: HashMap<String, Vec<String>>,
    pub capabilities_drop: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub control: String, // e.g. "hostNetwork", "privileged"
    pub message: String,
}

impl Violation {
    fn new(control: impl Into<String>, message: impl Into<String>) -> Self {
        Self { control: control.into(), message: message.into() }
    }
}

/// Run all controls applicable at `level` against `spec`. Returns ALL
/// violations (upstream returns a sorted list to keep messages stable).
pub fn check_level(level: Level, spec: &PodSecuritySpec) -> Vec<Violation> {
    let mut out = vec![];
    if level == Level::Privileged { return out; }
    // Baseline controls
    if spec.host_network {
        out.push(Violation::new("hostNetwork", "host network must not be set"));
    }
    if spec.host_pid {
        out.push(Violation::new("hostPID", "host PID must not be set"));
    }
    if spec.host_ipc {
        out.push(Violation::new("hostIPC", "host IPC must not be set"));
    }
    if !spec.host_path_volumes.is_empty() {
        out.push(Violation::new("hostPathVolumes",
            format!("hostPath volumes are forbidden: {}", spec.host_path_volumes.join(", "))));
    }
    if !spec.host_ports.is_empty() {
        out.push(Violation::new("hostPorts",
            format!("hostPort declarations forbidden: {:?}", spec.host_ports)));
    }
    if !spec.privileged_containers.is_empty() {
        out.push(Violation::new("privileged",
            format!("privileged containers: {}", spec.privileged_containers.join(", "))));
    }
    // Capabilities — baseline forbids adding any except a small allowlist.
    let cap_allowlist: std::collections::HashSet<&str> = [
        "AUDIT_WRITE","CHOWN","DAC_OVERRIDE","FOWNER","FSETID",
        "KILL","MKNOD","NET_BIND_SERVICE","SETFCAP","SETGID",
        "SETPCAP","SETUID","SYS_CHROOT",
    ].into_iter().collect();
    for (c, caps) in &spec.capabilities_add {
        for cap in caps {
            if !cap_allowlist.contains(cap.as_str()) {
                out.push(Violation::new("capabilities",
                    format!("container {c} adds disallowed capability {cap}")));
            }
        }
    }
    if level == Level::Baseline { return out; }
    // Restricted controls (additive on top of baseline)
    for (c, ape) in &spec.allow_privilege_escalation {
        if *ape {
            out.push(Violation::new("allowPrivilegeEscalation",
                format!("container {c} sets allowPrivilegeEscalation=true")));
        }
    }
    for (c, rnru) in &spec.run_as_non_root {
        if rnru.is_none() || rnru == &Some(false) {
            out.push(Violation::new("runAsNonRoot",
                format!("container {c} must set runAsNonRoot=true")));
        }
    }
    for (c, uid) in &spec.run_as_user {
        if *uid == 0 {
            out.push(Violation::new("runAsUser",
                format!("container {c} sets runAsUser=0 (root)")));
        }
    }
    for (c, profile) in &spec.seccomp_profile {
        if !matches!(profile.as_str(), "RuntimeDefault" | "Localhost") {
            out.push(Violation::new("seccompProfile",
                format!("container {c} seccompProfile.type must be RuntimeDefault or Localhost; got {profile}")));
        }
    }
    // restricted requires `capabilities.drop` to include "ALL" on every container.
    let containers_seen: std::collections::HashSet<String> =
        spec.privileged_containers.iter().cloned()
        .chain(spec.allow_privilege_escalation.keys().cloned())
        .chain(spec.capabilities_add.keys().cloned())
        .chain(spec.capabilities_drop.keys().cloned())
        .chain(spec.run_as_user.keys().cloned())
        .collect();
    for c in containers_seen {
        let drops = spec.capabilities_drop.get(&c).cloned().unwrap_or_default();
        if !drops.iter().any(|d| d == "ALL") {
            out.push(Violation::new("capabilities",
                format!("container {c} must drop ALL capabilities")));
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Namespace level lister + plugin
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct InMemoryLevelStore {
    inner: RwLock<HashMap<(String, String), LevelTriple>>, // (tenant, ns)
}

impl InMemoryLevelStore {
    pub fn new() -> Self { Self::default() }
    pub fn set(&self, tenant: &str, namespace: &str, t: LevelTriple) {
        self.inner.write().unwrap().insert((tenant.into(), namespace.into()), t);
    }
    pub fn get(&self, tenant: &str, namespace: &str) -> LevelTriple {
        self.inner.read().unwrap().get(&(tenant.into(), namespace.into()))
            .cloned().unwrap_or_default()
    }
}

pub trait PodSpecExtractor: Send + Sync {
    /// Map an AdmissionRequest object → PodSecuritySpec. Returns None when
    /// the request isn't pod-shaped.
    fn extract(&self, req: &AdmissionRequest) -> Option<PodSecuritySpec>;
}

pub struct PodSecurityPlugin {
    pub levels: Arc<InMemoryLevelStore>,
    pub extractor: Arc<dyn PodSpecExtractor>,
}

impl ValidatingWebhook for PodSecurityPlugin {
    fn name(&self) -> &str { "PodSecurity" }
    fn validate(&self, req: &AdmissionRequest) -> AdmissionResponse {
        if req.kind != "Pod" { return AdmissionResponse::allow(req); }
        if !matches!(req.operation, Operation::Create | Operation::Update) {
            return AdmissionResponse::allow(req);
        }
        let Some(spec) = self.extractor.extract(req) else {
            return AdmissionResponse::allow(req);
        };
        let triple = self.levels.get(&req.tenant_id, &req.namespace);
        let mut warnings = Vec::new();
        // enforce
        if let Some(level) = triple.enforce {
            let v = check_level(level, &spec);
            if !v.is_empty() {
                let msg = v.iter().map(|x| format!("{}: {}", x.control, x.message))
                    .collect::<Vec<_>>().join("; ");
                let mut r = AdmissionResponse::deny(req, 403, msg);
                r.warnings = warnings;
                return r;
            }
        }
        // warn
        if let Some(level) = triple.warn {
            let v = check_level(level, &spec);
            for x in v {
                warnings.push(format!("[psa.warn] {}: {}", x.control, x.message));
            }
        }
        // audit (audit annotations would be emitted via M4 audit framework)
        // No-op here.
        let _ = triple.audit;
        let mut r = AdmissionResponse::allow(req);
        r.warnings = warnings;
        r
    }
}

#[cfg(test)]
mod tests;
