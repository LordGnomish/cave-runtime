//! Cross-platform sandbox runners — Windows + FreeBSD.
//!
//! Cite: containerd `pkg/cri/server/podsandbox/sandbox_run_other.go`
//! (v2.x). The audit doc flagged this as an honest gap because the
//! Charter doesn't restrict cave to Linux but the existing
//! `sandbox.rs` is Linux-namespaces-only.
//!
//! Scope of this port:
//!
//! * **Windows** — sandbox isolation is via "job objects" + the
//!   Host Compute Service (HCS). Cave models a `WindowsSandbox`
//!   with the per-sandbox job-object name + HCS container id and
//!   the lifecycle methods (`create`, `run`, `stop`, `status`).
//! * **FreeBSD** — sandbox isolation is via "jails". We model a
//!   `FreeBsdJail` with the jail id + path + lifecycle methods.
//!
//! Neither platform's *real* IPC is wired here — we don't run
//! `CreateJobObjectW` or `jail_attach` (those are syscalls that
//! belong to the platform-specific CRI runtime). What we ship is
//! the deterministic state machine + path layout + spec validation,
//! so a future platform-specific runtime can drop in as a backend.
//!
//! Tests run on every platform (no `#[cfg(target_os)]` gates) so the
//! state machine is exercised in CI regardless of the build host.

use crate::error::{CriError, CriResult};
use crate::models::{SandboxSpec, SandboxState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[allow(dead_code)]
pub const UPSTREAM_PATH: &str = "pkg/cri/server/podsandbox/sandbox_run_other.go";

/// Which non-Linux platform this sandbox targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    FreeBsd,
}

impl Platform {
    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::FreeBsd => "freebsd",
        }
    }
}

// ── Windows ────────────────────────────────────────────────────────────────

/// A Windows sandbox — job object + HCS container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsSandbox {
    pub sandbox_id: Uuid,
    /// Job-object name — Windows kernel-level grouping that lets the
    /// system terminate all child processes when the sandbox stops.
    pub job_object_name: String,
    /// Host Compute Service container id — looks like a GUID.
    pub hcs_container_id: String,
    /// Sandbox state. Driven by [`WindowsSandbox::transition`].
    pub state: SandboxState,
    pub created_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl WindowsSandbox {
    /// Build a fresh sandbox descriptor — state starts as `Ready`
    /// (matches containerd's `Created → Ready` transition: the
    /// HCS container creation is synchronous and returns Ready).
    pub fn new(spec: &SandboxSpec, now: DateTime<Utc>) -> CriResult<Self> {
        validate_spec(spec, Platform::Windows)?;
        let id = Uuid::new_v4();
        Ok(Self {
            sandbox_id: id,
            job_object_name: format!("cave-sandbox-{}", id.simple()),
            hcs_container_id: id.to_string(),
            state: SandboxState::Ready,
            created_at: now,
            stopped_at: None,
        })
    }

    /// Move to NotReady → stopped. Idempotent: stopping an already-
    /// stopped sandbox is a no-op.
    pub fn stop(&mut self, now: DateTime<Utc>) {
        if self.state != SandboxState::NotReady {
            self.state = SandboxState::NotReady;
            self.stopped_at = Some(now);
        }
    }

    /// Status string consumed by CRI's `PodSandboxStatus` response.
    pub fn status_str(&self) -> &'static str {
        match self.state {
            SandboxState::Ready => "SANDBOX_READY",
            SandboxState::NotReady => "SANDBOX_NOTREADY",
        }
    }
}

// ── FreeBSD ────────────────────────────────────────────────────────────────

/// A FreeBSD jail — `jid` + per-jail rootfs path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeBsdJail {
    pub sandbox_id: Uuid,
    /// jail id — assigned at creation by the kernel. Stored as
    /// `Option<u32>` because the deterministic test path doesn't
    /// actually call `jail_create`; production runtime fills it in.
    pub jid: Option<u32>,
    /// Per-jail rootfs path (`/var/run/cave/jails/<sandbox_id>/root`).
    pub jail_path: PathBuf,
    /// Hostname inside the jail — defaults to the sandbox name.
    pub hostname: String,
    pub state: SandboxState,
    pub created_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl FreeBsdJail {
    pub fn new(spec: &SandboxSpec, now: DateTime<Utc>) -> CriResult<Self> {
        validate_spec(spec, Platform::FreeBsd)?;
        let id = Uuid::new_v4();
        let jail_root = PathBuf::from("/var/run/cave/jails").join(id.to_string());
        Ok(Self {
            sandbox_id: id,
            jid: None,
            jail_path: jail_root.join("root"),
            hostname: spec.name.clone(),
            state: SandboxState::Ready,
            created_at: now,
            stopped_at: None,
        })
    }

    pub fn stop(&mut self, now: DateTime<Utc>) {
        if self.state != SandboxState::NotReady {
            self.state = SandboxState::NotReady;
            self.stopped_at = Some(now);
        }
    }

    pub fn status_str(&self) -> &'static str {
        match self.state {
            SandboxState::Ready => "SANDBOX_READY",
            SandboxState::NotReady => "SANDBOX_NOTREADY",
        }
    }
}

// ── shared validation ──────────────────────────────────────────────────────

fn validate_spec(spec: &SandboxSpec, platform: Platform) -> CriResult<()> {
    if spec.name.trim().is_empty() {
        return Err(CriError::Sandbox(
            "sandbox name must not be empty".to_string(),
        ));
    }
    // The Linux sandbox tolerates arbitrary names; Windows job
    // objects + FreeBSD jails both forbid `/` (path-like names).
    if spec.name.contains('/') {
        return Err(CriError::Sandbox(format!(
            "{} sandbox name '{}' must not contain '/'",
            platform.as_str(),
            spec.name
        )));
    }
    Ok(())
}

// ── unified runner ─────────────────────────────────────────────────────────

/// Outcome of running a non-Linux sandbox — surfaced as a typed
/// enum so callers can pattern-match without runtime introspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OtherSandbox {
    Windows(WindowsSandbox),
    FreeBsd(FreeBsdJail),
}

impl OtherSandbox {
    pub fn sandbox_id(&self) -> Uuid {
        match self {
            OtherSandbox::Windows(w) => w.sandbox_id,
            OtherSandbox::FreeBsd(f) => f.sandbox_id,
        }
    }

    pub fn platform(&self) -> Platform {
        match self {
            OtherSandbox::Windows(_) => Platform::Windows,
            OtherSandbox::FreeBsd(_) => Platform::FreeBsd,
        }
    }

    pub fn state(&self) -> SandboxState {
        match self {
            OtherSandbox::Windows(w) => w.state.clone(),
            OtherSandbox::FreeBsd(f) => f.state.clone(),
        }
    }

    pub fn stop(&mut self, now: DateTime<Utc>) {
        match self {
            OtherSandbox::Windows(w) => w.stop(now),
            OtherSandbox::FreeBsd(f) => f.stop(now),
        }
    }

    pub fn status_str(&self) -> &'static str {
        match self {
            OtherSandbox::Windows(w) => w.status_str(),
            OtherSandbox::FreeBsd(f) => f.status_str(),
        }
    }
}

/// Top-level `run_pod_sandbox` for the non-Linux family. Picks the
/// concrete sandbox flavour from `platform` and constructs the
/// state machine accordingly.
pub fn run_pod_sandbox_other(
    spec: SandboxSpec,
    platform: Platform,
    now: DateTime<Utc>,
) -> CriResult<OtherSandbox> {
    Ok(match platform {
        Platform::Windows => OtherSandbox::Windows(WindowsSandbox::new(&spec, now)?),
        Platform::FreeBsd => OtherSandbox::FreeBsd(FreeBsdJail::new(&spec, now)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-05-13T14:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn spec(name: &str) -> SandboxSpec {
        SandboxSpec {
            name: name.into(),
            namespace: "default".into(),
            labels: Default::default(),
            annotations: Default::default(),
            hostname: None,
            dns_config: None,
            port_mappings: vec![],
            log_directory: None,
            cgroup_parent: None,
            runtime_handler: None,
            user_namespace_mode: crate::models::UserNamespaceMode::Host,
        }
    }

    // ── Windows ────────────────────────────────────────────────

    #[test]
    fn windows_new_starts_in_ready_state() {
        let w = WindowsSandbox::new(&spec("nginx"), now()).unwrap();
        assert_eq!(w.state, SandboxState::Ready);
        assert!(w.job_object_name.starts_with("cave-sandbox-"));
        assert_eq!(w.status_str(), "SANDBOX_READY");
        assert!(w.stopped_at.is_none());
    }

    #[test]
    fn windows_stop_transitions_to_notready_and_records_stopped_at() {
        let mut w = WindowsSandbox::new(&spec("nginx"), now()).unwrap();
        let later = now() + chrono::Duration::seconds(10);
        w.stop(later);
        assert_eq!(w.state, SandboxState::NotReady);
        assert_eq!(w.stopped_at, Some(later));
        assert_eq!(w.status_str(), "SANDBOX_NOTREADY");
    }

    #[test]
    fn windows_stop_is_idempotent() {
        let mut w = WindowsSandbox::new(&spec("nginx"), now()).unwrap();
        let first = now() + chrono::Duration::seconds(10);
        let second = now() + chrono::Duration::seconds(20);
        w.stop(first);
        w.stop(second);
        // stopped_at stays at first call.
        assert_eq!(w.stopped_at, Some(first));
    }

    #[test]
    fn windows_rejects_empty_name() {
        let err = WindowsSandbox::new(&spec(""), now()).unwrap_err();
        assert!(matches!(err, CriError::Sandbox(_)));
    }

    #[test]
    fn windows_rejects_slash_in_name() {
        let err = WindowsSandbox::new(&spec("foo/bar"), now()).unwrap_err();
        match err {
            CriError::Sandbox(msg) => assert!(msg.contains("windows")),
            other => panic!("got {other:?}"),
        }
    }

    // ── FreeBSD ────────────────────────────────────────────────

    #[test]
    fn freebsd_new_starts_in_ready_state_with_jail_path() {
        let f = FreeBsdJail::new(&spec("nginx"), now()).unwrap();
        assert_eq!(f.state, SandboxState::Ready);
        assert!(f.jail_path.to_string_lossy().contains(&f.sandbox_id.to_string()));
        assert_eq!(f.hostname, "nginx");
        assert!(f.jid.is_none()); // jid filled by real syscall path
    }

    #[test]
    fn freebsd_stop_transitions_to_notready() {
        let mut f = FreeBsdJail::new(&spec("nginx"), now()).unwrap();
        f.stop(now() + chrono::Duration::seconds(5));
        assert_eq!(f.state, SandboxState::NotReady);
    }

    #[test]
    fn freebsd_rejects_slash_in_name() {
        let err = FreeBsdJail::new(&spec("a/b"), now()).unwrap_err();
        match err {
            CriError::Sandbox(msg) => assert!(msg.contains("freebsd")),
            other => panic!("got {other:?}"),
        }
    }

    // ── unified runner ─────────────────────────────────────────

    #[test]
    fn run_pod_sandbox_other_picks_windows_for_windows_platform() {
        let s = run_pod_sandbox_other(spec("a"), Platform::Windows, now()).unwrap();
        assert_eq!(s.platform(), Platform::Windows);
        assert!(matches!(s, OtherSandbox::Windows(_)));
    }

    #[test]
    fn run_pod_sandbox_other_picks_freebsd_for_freebsd_platform() {
        let s = run_pod_sandbox_other(spec("a"), Platform::FreeBsd, now()).unwrap();
        assert_eq!(s.platform(), Platform::FreeBsd);
        assert!(matches!(s, OtherSandbox::FreeBsd(_)));
    }

    #[test]
    fn other_sandbox_state_propagates_through_enum() {
        let mut s = run_pod_sandbox_other(spec("a"), Platform::Windows, now()).unwrap();
        assert_eq!(s.state(), SandboxState::Ready);
        s.stop(now() + chrono::Duration::seconds(1));
        assert_eq!(s.state(), SandboxState::NotReady);
    }

    #[test]
    fn other_sandbox_status_str_round_trips() {
        let s = run_pod_sandbox_other(spec("a"), Platform::FreeBsd, now()).unwrap();
        assert_eq!(s.status_str(), "SANDBOX_READY");
    }

    #[test]
    fn other_sandbox_serde_round_trip_windows() {
        let s = run_pod_sandbox_other(spec("a"), Platform::Windows, now()).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: OtherSandbox = serde_json::from_str(&json).unwrap();
        assert_eq!(s.sandbox_id(), back.sandbox_id());
    }

    #[test]
    fn other_sandbox_serde_round_trip_freebsd() {
        let s = run_pod_sandbox_other(spec("a"), Platform::FreeBsd, now()).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        let back: OtherSandbox = serde_json::from_str(&json).unwrap();
        assert_eq!(s.sandbox_id(), back.sandbox_id());
    }

    #[test]
    fn platform_as_str_returns_canonical_lowercase() {
        assert_eq!(Platform::Windows.as_str(), "windows");
        assert_eq!(Platform::FreeBsd.as_str(), "freebsd");
    }
}
