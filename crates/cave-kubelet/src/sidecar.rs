// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Native sidecar containers — KEP-753 (`SidecarContainers` GA in 1.29).
//!
//! Mirrors `pkg/kubelet/kuberuntime` and `pkg/api/v1/pod` semantics for
//! init containers with `restartPolicy: Always`: they are sidecars whose
//! lifetime spans the full pod lifecycle, started during the init phase
//! and terminated together with the main containers (in reverse start
//! order). This module captures the start-order graph, readiness gating,
//! restart-policy validation, and the termination ordering rules.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerKind {
    /// Init container that completes before the next container starts.
    Init,
    /// Native sidecar — init container with `restartPolicy: Always`.
    /// Started during init phase, runs alongside main containers.
    Sidecar,
    /// Regular workload container.
    Main,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerRestartPolicy {
    /// Default for init containers (must complete).
    Never,
    /// Sidecar marker — init container with this policy is treated as sidecar.
    Always,
    OnFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub name: String,
    pub kind: ContainerKind,
    /// Only set for init containers; `Some(Always)` ⇒ sidecar.
    pub restart_policy: Option<ContainerRestartPolicy>,
    /// Whether the container declares a `startupProbe`.
    pub has_startup_probe: bool,
    /// Whether the container declares a `readinessProbe`.
    pub has_readiness_probe: bool,
    /// `terminationGracePeriodSeconds` override at the container level
    /// (Kubernetes 1.25+ `pod.spec.terminationGracePeriodSeconds` is
    /// pod-level; per-probe-grace at container probe is separate). We
    /// keep the field for parity with upstream's container-level grace.
    pub termination_grace_period_seconds: Option<u32>,
}

impl ContainerSpec {
    /// Whether this container is a *sidecar* per KEP-753 — i.e. init container
    /// with `restartPolicy: Always`.
    pub fn is_sidecar(&self) -> bool {
        self.kind == ContainerKind::Init
            && self.restart_policy == Some(ContainerRestartPolicy::Always)
    }

    /// Whether this container blocks subsequent containers from starting.
    /// Init containers (non-sidecar) block; sidecars do not.
    pub fn is_blocking_init(&self) -> bool {
        self.kind == ContainerKind::Init && !self.is_sidecar()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerLifecycleState {
    NotStarted,
    Starting,
    /// Sidecar/main: running and ready (or no readiness probe).
    Running,
    /// Init container only: completed successfully (Exit 0).
    Completed,
    /// Sidecar/main: terminating (preStop / SIGTERM in progress).
    Terminating,
    Terminated,
    Failed,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SidecarError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("ordering violation: {0}")]
    Ordering(String),
}

pub type SidecarResult<T> = Result<T, SidecarError>;

/// Validate that a pod's container list (init + sidecar + main) is consistent
/// per upstream's `pkg/api/pod/util.go` rules:
///   1. Container names unique pod-wide.
///   2. Only init containers may have a restart policy; main containers must not.
///   3. Init containers' restart policy ∈ {Never, Always, OnFailure}, with
///      Always meaning "sidecar".
///   4. Pod-level `restartPolicy: Never` is incompatible with init-container
///      `restartPolicy: Always` (sidecar) ONLY when GA gate is off — in 1.29 GA
///      this constraint was lifted, so we accept any combination.
pub fn validate_pod_containers(
    init_containers: &[ContainerSpec],
    main_containers: &[ContainerSpec],
) -> SidecarResult<()> {
    if main_containers.is_empty() {
        return Err(SidecarError::Invalid(
            "pod must declare at least one main container".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for c in init_containers.iter().chain(main_containers.iter()) {
        if c.name.is_empty() {
            return Err(SidecarError::Invalid("container name empty".into()));
        }
        if !names.insert(&c.name) {
            return Err(SidecarError::Invalid(format!(
                "duplicate container name {}",
                c.name
            )));
        }
    }
    for c in init_containers {
        if c.kind != ContainerKind::Init {
            return Err(SidecarError::Invalid(format!(
                "init container {} must have kind=Init",
                c.name
            )));
        }
    }
    for c in main_containers {
        if c.kind != ContainerKind::Main {
            return Err(SidecarError::Invalid(format!(
                "main container {} must have kind=Main",
                c.name
            )));
        }
        if c.restart_policy.is_some() {
            return Err(SidecarError::Invalid(format!(
                "main container {} must not set restartPolicy",
                c.name
            )));
        }
    }
    Ok(())
}

/// Compute the **start order** — list of (name, kind) in the order kubelet
/// should call `StartContainer` for them. Rules (per `kuberuntime_manager.go`
/// `computePodActions` + KEP-753):
///   1. Init containers (sidecars OR blocking) start in declaration order.
///   2. A blocking init must complete before the next container starts.
///   3. Sidecars only need to be **ready** (or started, if no readinessProbe),
///      then the next container starts.
///   4. Main containers start in declaration order, AFTER all init phase has
///      completed all blocking + sidecars-ready transitions.
pub fn compute_start_order(
    init_containers: &[ContainerSpec],
    main_containers: &[ContainerSpec],
) -> Vec<String> {
    let mut order = Vec::with_capacity(init_containers.len() + main_containers.len());
    for c in init_containers {
        order.push(c.name.clone());
    }
    for c in main_containers {
        order.push(c.name.clone());
    }
    order
}

/// Compute the **termination order** — reverse of start order, but with one
/// extra rule: sidecars are torn down only AFTER all main containers have
/// terminated, so they are NOT interleaved with mains in reverse — they go
/// last (in reverse declaration order among themselves).
pub fn compute_termination_order(
    init_containers: &[ContainerSpec],
    main_containers: &[ContainerSpec],
) -> Vec<String> {
    let mut out = Vec::with_capacity(init_containers.len() + main_containers.len());
    // Mains first, in reverse declaration order.
    for c in main_containers.iter().rev() {
        out.push(c.name.clone());
    }
    // Sidecars last, in reverse declaration order. Blocking init containers
    // by definition already finished and aren't part of teardown.
    for c in init_containers.iter().rev() {
        if c.is_sidecar() {
            out.push(c.name.clone());
        }
    }
    out
}

/// Whether the pod can transition out of the init phase given a per-container
/// state map. Rules (per upstream):
///   - All blocking init containers must be `Completed`.
///   - All sidecars must be `Running` (and ready if they have a readiness probe).
pub fn init_phase_complete(
    init_containers: &[ContainerSpec],
    state: &BTreeMap<String, ContainerLifecycleState>,
    sidecar_ready: &BTreeMap<String, bool>,
) -> bool {
    for c in init_containers {
        let cs = state.get(&c.name).copied().unwrap_or(ContainerLifecycleState::NotStarted);
        if c.is_blocking_init() {
            if cs != ContainerLifecycleState::Completed {
                return false;
            }
        } else {
            // Sidecar.
            if cs != ContainerLifecycleState::Running {
                return false;
            }
            if c.has_readiness_probe {
                if !sidecar_ready.get(&c.name).copied().unwrap_or(false) {
                    return false;
                }
            }
        }
    }
    true
}

/// Decide which container the kubelet should start NEXT given current state.
/// Returns None if everything that needs to be running is running.
pub fn next_container_to_start(
    init_containers: &[ContainerSpec],
    main_containers: &[ContainerSpec],
    state: &BTreeMap<String, ContainerLifecycleState>,
    sidecar_ready: &BTreeMap<String, bool>,
) -> Option<String> {
    // Walk init containers in declaration order.
    for c in init_containers {
        let cs = state.get(&c.name).copied().unwrap_or(ContainerLifecycleState::NotStarted);
        match cs {
            ContainerLifecycleState::NotStarted => return Some(c.name.clone()),
            ContainerLifecycleState::Starting => return None, // wait
            // Blocking init must reach Completed before we move on.
            ContainerLifecycleState::Running if c.is_blocking_init() => return None,
            // Sidecar: if it's not yet ready (when readinessProbe declared),
            // we wait before starting subsequent containers.
            ContainerLifecycleState::Running if c.is_sidecar() => {
                if c.has_readiness_probe
                    && !sidecar_ready.get(&c.name).copied().unwrap_or(false)
                {
                    return None;
                }
            }
            ContainerLifecycleState::Completed => continue,
            ContainerLifecycleState::Failed => return None,
            _ => continue,
        }
    }
    // Then main containers in declaration order.
    for c in main_containers {
        let cs = state.get(&c.name).copied().unwrap_or(ContainerLifecycleState::NotStarted);
        if cs == ContainerLifecycleState::NotStarted {
            return Some(c.name.clone());
        }
    }
    None
}

/// Determine whether starting `target` violates the start-order invariant
/// given current state. Used for admission-side checks.
pub fn validate_start_request(
    target: &str,
    init_containers: &[ContainerSpec],
    main_containers: &[ContainerSpec],
    state: &BTreeMap<String, ContainerLifecycleState>,
    sidecar_ready: &BTreeMap<String, bool>,
) -> SidecarResult<()> {
    let next = next_container_to_start(init_containers, main_containers, state, sidecar_ready);
    match next {
        Some(name) if name == target => Ok(()),
        Some(name) => Err(SidecarError::Ordering(format!(
            "must start {} before {}",
            name, target
        ))),
        None => Err(SidecarError::Ordering(format!(
            "no container needs starting; refusing to start {}",
            target
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init(name: &str, policy: Option<ContainerRestartPolicy>) -> ContainerSpec {
        ContainerSpec {
            name: name.into(),
            kind: ContainerKind::Init,
            restart_policy: policy,
            has_startup_probe: false,
            has_readiness_probe: false,
            termination_grace_period_seconds: None,
        }
    }

    fn sidecar(name: &str) -> ContainerSpec {
        init(name, Some(ContainerRestartPolicy::Always))
    }

    fn main_c(name: &str) -> ContainerSpec {
        ContainerSpec {
            name: name.into(),
            kind: ContainerKind::Main,
            restart_policy: None,
            has_startup_probe: false,
            has_readiness_probe: false,
            termination_grace_period_seconds: None,
        }
    }

    #[test]
    fn is_sidecar_true_for_init_with_always() {
        assert!(sidecar("s").is_sidecar());
    }

    #[test]
    fn is_sidecar_false_for_init_without_always() {
        assert!(!init("i", None).is_sidecar());
        assert!(!init("i", Some(ContainerRestartPolicy::Never)).is_sidecar());
        assert!(!init("i", Some(ContainerRestartPolicy::OnFailure)).is_sidecar());
    }

    #[test]
    fn is_sidecar_false_for_main_container() {
        assert!(!main_c("m").is_sidecar());
    }

    #[test]
    fn is_blocking_init_excludes_sidecars() {
        assert!(init("i", None).is_blocking_init());
        assert!(init("i", Some(ContainerRestartPolicy::OnFailure)).is_blocking_init());
        assert!(!sidecar("s").is_blocking_init());
        assert!(!main_c("m").is_blocking_init());
    }

    #[test]
    fn validate_rejects_no_main_containers() {
        assert!(validate_pod_containers(&[init("i", None)], &[]).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_names() {
        assert!(validate_pod_containers(
            &[init("dup", None)],
            &[main_c("dup")]
        )
        .is_err());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut bad = main_c("ok");
        bad.name = "".into();
        assert!(validate_pod_containers(&[], &[bad]).is_err());
    }

    #[test]
    fn validate_rejects_main_with_restart_policy() {
        let mut bad = main_c("m");
        bad.restart_policy = Some(ContainerRestartPolicy::Always);
        assert!(validate_pod_containers(&[], &[bad]).is_err());
    }

    #[test]
    fn validate_rejects_init_with_main_kind() {
        let mut bad = init("i", None);
        bad.kind = ContainerKind::Main;
        assert!(validate_pod_containers(&[bad], &[main_c("m")]).is_err());
    }

    #[test]
    fn validate_accepts_init_sidecar_main_combo() {
        validate_pod_containers(
            &[init("i", None), sidecar("s")],
            &[main_c("a"), main_c("b")],
        )
        .unwrap();
    }

    #[test]
    fn start_order_inits_then_mains() {
        let order = compute_start_order(
            &[init("i1", None), sidecar("s1")],
            &[main_c("m1"), main_c("m2")],
        );
        assert_eq!(order, vec!["i1", "s1", "m1", "m2"]);
    }

    #[test]
    fn termination_order_mains_first_reversed_then_sidecars_reversed() {
        let order = compute_termination_order(
            &[init("i1", None), sidecar("s1"), sidecar("s2")],
            &[main_c("m1"), main_c("m2"), main_c("m3")],
        );
        assert_eq!(order, vec!["m3", "m2", "m1", "s2", "s1"]);
    }

    #[test]
    fn termination_excludes_blocking_inits() {
        let order = compute_termination_order(&[init("i1", None)], &[main_c("m1")]);
        assert_eq!(order, vec!["m1"]);
    }

    #[test]
    fn termination_with_no_sidecars() {
        let order = compute_termination_order(
            &[init("i1", None)],
            &[main_c("m1"), main_c("m2")],
        );
        assert_eq!(order, vec!["m2", "m1"]);
    }

    #[test]
    fn init_phase_complete_requires_blocking_completed() {
        let mut state = BTreeMap::new();
        state.insert("i1".into(), ContainerLifecycleState::Running);
        assert!(!init_phase_complete(&[init("i1", None)], &state, &BTreeMap::new()));
        state.insert("i1".into(), ContainerLifecycleState::Completed);
        assert!(init_phase_complete(&[init("i1", None)], &state, &BTreeMap::new()));
    }

    #[test]
    fn init_phase_requires_sidecar_running() {
        let mut state = BTreeMap::new();
        state.insert("s1".into(), ContainerLifecycleState::Starting);
        assert!(!init_phase_complete(&[sidecar("s1")], &state, &BTreeMap::new()));
        state.insert("s1".into(), ContainerLifecycleState::Running);
        assert!(init_phase_complete(&[sidecar("s1")], &state, &BTreeMap::new()));
    }

    #[test]
    fn init_phase_requires_sidecar_ready_when_readiness_probe_present() {
        let mut s = sidecar("s1");
        s.has_readiness_probe = true;
        let mut state = BTreeMap::new();
        state.insert("s1".into(), ContainerLifecycleState::Running);
        let mut ready = BTreeMap::new();
        assert!(!init_phase_complete(&[s.clone()], &state, &ready));
        ready.insert("s1".into(), true);
        assert!(init_phase_complete(&[s], &state, &ready));
    }

    #[test]
    fn init_phase_no_init_containers_is_complete() {
        assert!(init_phase_complete(&[], &BTreeMap::new(), &BTreeMap::new()));
    }

    #[test]
    fn next_to_start_picks_first_unstarted_init() {
        let s = BTreeMap::new();
        let r = BTreeMap::new();
        let next = next_container_to_start(
            &[init("i1", None), init("i2", None)],
            &[main_c("m1")],
            &s,
            &r,
        );
        assert_eq!(next, Some("i1".into()));
    }

    #[test]
    fn next_to_start_waits_for_blocking_init_completion() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Running);
        let r = BTreeMap::new();
        let next = next_container_to_start(
            &[init("i1", None)],
            &[main_c("m1")],
            &s,
            &r,
        );
        assert_eq!(next, None);
    }

    #[test]
    fn next_to_start_advances_after_blocking_completed() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Completed);
        let next = next_container_to_start(
            &[init("i1", None)],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, Some("m1".into()));
    }

    #[test]
    fn next_to_start_after_sidecar_running_proceeds_to_main() {
        let mut s = BTreeMap::new();
        s.insert("s1".into(), ContainerLifecycleState::Running);
        let next = next_container_to_start(
            &[sidecar("s1")],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, Some("m1".into()));
    }

    #[test]
    fn next_to_start_waits_for_sidecar_readiness_when_probe_declared() {
        let mut sc = sidecar("s1");
        sc.has_readiness_probe = true;
        let mut s = BTreeMap::new();
        s.insert("s1".into(), ContainerLifecycleState::Running);
        let r = BTreeMap::new();
        let next = next_container_to_start(&[sc.clone()], &[main_c("m1")], &s, &r);
        assert_eq!(next, None);
        let mut r2 = r.clone();
        r2.insert("s1".into(), true);
        let next2 = next_container_to_start(&[sc], &[main_c("m1")], &s, &r2);
        assert_eq!(next2, Some("m1".into()));
    }

    #[test]
    fn next_to_start_returns_none_when_all_running() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Completed);
        s.insert("m1".into(), ContainerLifecycleState::Running);
        s.insert("m2".into(), ContainerLifecycleState::Running);
        let next = next_container_to_start(
            &[init("i1", None)],
            &[main_c("m1"), main_c("m2")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, None);
    }

    #[test]
    fn next_to_start_starting_blocks_progress() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Starting);
        let next = next_container_to_start(
            &[init("i1", None)],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, None);
    }

    #[test]
    fn next_to_start_failed_blocking_init_blocks_progress() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Failed);
        let next = next_container_to_start(
            &[init("i1", None)],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, None);
    }

    #[test]
    fn validate_start_request_accepts_correct_target() {
        validate_start_request(
            "i1",
            &[init("i1", None)],
            &[main_c("m1")],
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
    }

    #[test]
    fn validate_start_request_rejects_skipped_target() {
        let err = validate_start_request(
            "m1",
            &[init("i1", None)],
            &[main_c("m1")],
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(matches!(err, SidecarError::Ordering(_)));
    }

    #[test]
    fn validate_start_request_rejects_when_nothing_to_start() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Completed);
        s.insert("m1".into(), ContainerLifecycleState::Running);
        let err = validate_start_request(
            "m1",
            &[init("i1", None)],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(matches!(err, SidecarError::Ordering(_)));
    }

    #[test]
    fn sidecar_with_blocking_init_before_it_runs_first_init() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Running);
        let next = next_container_to_start(
            &[init("i1", None), sidecar("s1")],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, None);
    }

    #[test]
    fn after_blocking_init_done_sidecar_starts_next() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Completed);
        let next = next_container_to_start(
            &[init("i1", None), sidecar("s1")],
            &[main_c("m1")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, Some("s1".into()));
    }

    #[test]
    fn termination_order_reversed_among_sidecars_keeps_relative_order() {
        let order = compute_termination_order(
            &[sidecar("s1"), sidecar("s2"), sidecar("s3")],
            &[main_c("m1")],
        );
        assert_eq!(order, vec!["m1", "s3", "s2", "s1"]);
    }

    #[test]
    fn empty_init_phase_with_main_only_pod() {
        let order = compute_start_order(&[], &[main_c("m1")]);
        assert_eq!(order, vec!["m1"]);
    }

    #[test]
    fn validate_pod_with_only_sidecars_no_main_rejected() {
        // Sidecar alone is invalid — pod must have a main.
        assert!(validate_pod_containers(&[sidecar("s")], &[]).is_err());
    }

    #[test]
    fn restart_policy_never_acceptable_for_init() {
        validate_pod_containers(
            &[init("i", Some(ContainerRestartPolicy::Never))],
            &[main_c("m")],
        )
        .unwrap();
    }

    #[test]
    fn restart_policy_on_failure_acceptable_for_init() {
        validate_pod_containers(
            &[init("i", Some(ContainerRestartPolicy::OnFailure))],
            &[main_c("m")],
        )
        .unwrap();
    }

    #[test]
    fn validate_main_kind_must_be_main() {
        let mut bad = main_c("m");
        bad.kind = ContainerKind::Init;
        assert!(validate_pod_containers(&[], &[bad]).is_err());
    }

    #[test]
    fn next_to_start_picks_main_after_all_inits_done() {
        let mut s = BTreeMap::new();
        s.insert("i1".into(), ContainerLifecycleState::Completed);
        s.insert("i2".into(), ContainerLifecycleState::Completed);
        s.insert("s1".into(), ContainerLifecycleState::Running);
        let mut sc = sidecar("s1");
        sc.has_readiness_probe = false;
        let next = next_container_to_start(
            &[init("i1", None), init("i2", None), sc],
            &[main_c("m1"), main_c("m2")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, Some("m1".into()));
    }

    #[test]
    fn next_to_start_proceeds_through_main_sequence() {
        let mut s = BTreeMap::new();
        s.insert("m1".into(), ContainerLifecycleState::Running);
        let next = next_container_to_start(
            &[],
            &[main_c("m1"), main_c("m2")],
            &s,
            &BTreeMap::new(),
        );
        assert_eq!(next, Some("m2".into()));
    }
}
