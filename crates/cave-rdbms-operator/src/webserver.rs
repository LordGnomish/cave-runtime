// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-pod sidecar webserver — CloudNativePG `pkg/management/postgres/webserver` analog.
//!
//! The operator's HTTP surface in `routes.rs` is cluster-wide. CloudNativePG
//! also runs a *per-instance* sidecar webserver inside every Postgres pod
//! that:
//!
//!   * publishes the local instance state (`/pg/status`)
//!   * exposes health/readiness probes (`/pg/healthz`, `/pg/readyz`)
//!   * accepts promote / backup / restart commands from the operator
//!   * records the latest checkpoint LSN for election arbitration
//!
//! This module owns the wire format (JSON request/response shapes), the
//! state struct that the sidecar exposes, and the pure decision functions
//! the sidecar dispatches when each endpoint is hit. The actual axum
//! routing is wired by cave-runtime's per-pod sidecar binary — kept out of
//! this crate so the operator core does not depend on transport.
//!
//! Upstream:
//!   cloudnative-pg/pkg/management/postgres/webserver/local.go
//!   cloudnative-pg/pkg/management/postgres/webserver/{management,backup,promote}.go

use serde::{Deserialize, Serialize};

/// The per-instance state surface emitted by `GET /pg/status` on the
/// sidecar. Mirrors `webserver.LocalStatus` in CNPG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceStatus {
    pub instance_name: String,
    pub namespace: String,
    pub cluster_name: String,
    pub system_id: String,
    pub timeline_id: u32,
    pub is_primary: bool,
    pub is_in_recovery: bool,
    pub is_pg_rewind_running: bool,
    pub current_lsn: String,
    pub received_lsn: Option<String>,
    pub replay_lsn: Option<String>,
    pub replay_paused: bool,
    pub pg_version: String,
    pub pending_restart: bool,
    pub pending_restart_for_decrease: bool,
    pub last_failed_archive_time: Option<String>,
    pub last_archived_wal: Option<String>,
    pub mighty_function: bool,
}

impl Default for InstanceStatus {
    fn default() -> Self {
        Self {
            instance_name: String::new(),
            namespace: "default".into(),
            cluster_name: String::new(),
            system_id: String::new(),
            timeline_id: 1,
            is_primary: false,
            is_in_recovery: true,
            is_pg_rewind_running: false,
            current_lsn: "0/0".into(),
            received_lsn: None,
            replay_lsn: None,
            replay_paused: false,
            pg_version: "16.0".into(),
            pending_restart: false,
            pending_restart_for_decrease: false,
            last_failed_archive_time: None,
            last_archived_wal: None,
            mighty_function: false,
        }
    }
}

/// Health-probe outcome. The sidecar serialises this to `200 OK` + body
/// for the healthy variants, `503` for the failed variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Healthy,
    Starting,
    NotReady,
    Failed,
}

impl ProbeOutcome {
    /// HTTP status code the sidecar should emit.
    pub fn http_status(&self) -> u16 {
        match self {
            ProbeOutcome::Healthy => 200,
            ProbeOutcome::Starting | ProbeOutcome::NotReady => 503,
            ProbeOutcome::Failed => 503,
        }
    }

    /// Whether this outcome should be treated as "alive" by the kubelet
    /// liveness probe. Mirrors CNPG `IsAlive`.
    pub fn is_alive(&self) -> bool {
        matches!(self, ProbeOutcome::Healthy | ProbeOutcome::Starting)
    }

    /// Whether this outcome should be treated as "ready" by the kubelet
    /// readiness probe. Mirrors CNPG `IsReady`.
    pub fn is_ready(&self) -> bool {
        matches!(self, ProbeOutcome::Healthy)
    }
}

/// Decide the liveness-probe outcome given the instance status.
///
/// CNPG considers an instance alive while it is **either** primary or
/// streaming from upstream and **not** in the middle of a pg_rewind run
/// that has wiped local state.
pub fn liveness(status: &InstanceStatus) -> ProbeOutcome {
    if status.is_pg_rewind_running {
        return ProbeOutcome::NotReady;
    }
    if status.is_primary || status.received_lsn.is_some() || status.replay_lsn.is_some() {
        ProbeOutcome::Healthy
    } else {
        ProbeOutcome::Starting
    }
}

/// Decide the readiness-probe outcome given the instance status.
///
/// Readiness is stricter than liveness: a replica with a non-empty
/// replay_lsn is ready; a primary not paused is ready.
pub fn readiness(status: &InstanceStatus) -> ProbeOutcome {
    if status.is_pg_rewind_running || status.pending_restart_for_decrease {
        return ProbeOutcome::NotReady;
    }
    if status.is_primary {
        return ProbeOutcome::Healthy;
    }
    if status.replay_paused {
        return ProbeOutcome::NotReady;
    }
    if status.replay_lsn.is_some() || status.received_lsn.is_some() {
        ProbeOutcome::Healthy
    } else {
        ProbeOutcome::Starting
    }
}

/// `POST /pg/promote` body. The sidecar reads this to decide between a
/// fast promotion (immediate) or a safe promotion (checkpoint first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromoteRequest {
    pub mode: PromoteMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromoteMode {
    Fast,
    Safe,
}

impl PromoteMode {
    /// Emit the `pg_ctl promote` flag set CNPG sends when this mode is
    /// chosen. Used by the sidecar shell-out.
    pub fn pg_ctl_flags(&self) -> &'static [&'static str] {
        match self {
            PromoteMode::Fast => &["-W"],
            PromoteMode::Safe => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromoteResponse {
    pub promoted: bool,
    pub new_timeline: u32,
    pub message: String,
}

/// `POST /pg/backup` body. Triggers a Barman-style backup on the local
/// instance. The sidecar forwards to `barman.rs::start_backup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRequest {
    pub method: BackupMethod,
    pub immediate_checkpoint: bool,
    pub backup_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupMethod {
    BarmanObjectStore,
    PluginVolumeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupResponse {
    pub accepted: bool,
    pub backup_id: String,
}

/// `POST /pg/pg_data/lsn` body. The operator collects election hints
/// (which replica is most caught-up) by polling each sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsnReport {
    pub instance_name: String,
    pub current_lsn: String,
    pub timeline_id: u32,
}

/// Comparator used by the operator to pick a preferred primary among
/// reporting replicas. Returns `true` iff `a` is at-least-as-caught-up
/// as `b`. Tie-breaks on `instance_name` for determinism.
pub fn lsn_at_least_as_caught_up(a: &LsnReport, b: &LsnReport) -> bool {
    let na = parse_lsn(&a.current_lsn).unwrap_or(0);
    let nb = parse_lsn(&b.current_lsn).unwrap_or(0);
    if na != nb {
        return na >= nb;
    }
    a.instance_name <= b.instance_name
}

/// Parse a Postgres LSN string (`"H/L"` with hex halves) into a single u64.
pub fn parse_lsn(s: &str) -> Option<u64> {
    let (h, l) = s.split_once('/')?;
    let h = u32::from_str_radix(h.trim(), 16).ok()?;
    let l = u32::from_str_radix(l.trim(), 16).ok()?;
    Some(((h as u64) << 32) | l as u64)
}

/// The set of routes the per-pod sidecar exposes. The cave-runtime sidecar
/// binary owns the axum wiring; this list is the canonical surface.
pub const ROUTES: &[(&str, &str)] = &[
    ("GET", "/pg/status"),
    ("GET", "/pg/healthz"),
    ("GET", "/pg/readyz"),
    ("POST", "/pg/promote"),
    ("POST", "/pg/backup"),
    ("POST", "/pg/pg_data/lsn"),
    ("GET", "/pg/preferred-primary"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn primary() -> InstanceStatus {
        InstanceStatus {
            is_primary: true,
            is_in_recovery: false,
            current_lsn: "1/100".into(),
            ..Default::default()
        }
    }

    fn replica_caught_up() -> InstanceStatus {
        InstanceStatus {
            is_primary: false,
            is_in_recovery: true,
            current_lsn: "1/F0".into(),
            received_lsn: Some("1/F0".into()),
            replay_lsn: Some("1/F0".into()),
            ..Default::default()
        }
    }

    fn replica_starting() -> InstanceStatus {
        InstanceStatus {
            is_primary: false,
            ..Default::default()
        }
    }

    #[test]
    fn liveness_primary_is_healthy() {
        assert_eq!(liveness(&primary()), ProbeOutcome::Healthy);
    }

    #[test]
    fn liveness_replica_streaming_is_healthy() {
        assert_eq!(liveness(&replica_caught_up()), ProbeOutcome::Healthy);
    }

    #[test]
    fn liveness_replica_starting_is_starting() {
        assert_eq!(liveness(&replica_starting()), ProbeOutcome::Starting);
    }

    #[test]
    fn liveness_pg_rewind_is_not_ready() {
        let mut s = replica_caught_up();
        s.is_pg_rewind_running = true;
        assert_eq!(liveness(&s), ProbeOutcome::NotReady);
    }

    #[test]
    fn readiness_primary_is_healthy() {
        assert_eq!(readiness(&primary()), ProbeOutcome::Healthy);
    }

    #[test]
    fn readiness_replica_paused_is_not_ready() {
        let mut s = replica_caught_up();
        s.replay_paused = true;
        assert_eq!(readiness(&s), ProbeOutcome::NotReady);
    }

    #[test]
    fn readiness_pending_restart_for_decrease_blocks() {
        let mut s = primary();
        s.pending_restart_for_decrease = true;
        assert_eq!(readiness(&s), ProbeOutcome::NotReady);
    }

    #[test]
    fn probe_outcome_http_status_codes() {
        assert_eq!(ProbeOutcome::Healthy.http_status(), 200);
        assert_eq!(ProbeOutcome::Starting.http_status(), 503);
        assert_eq!(ProbeOutcome::NotReady.http_status(), 503);
        assert_eq!(ProbeOutcome::Failed.http_status(), 503);
    }

    #[test]
    fn probe_outcome_alive_vs_ready() {
        assert!(ProbeOutcome::Healthy.is_alive());
        assert!(ProbeOutcome::Healthy.is_ready());
        assert!(ProbeOutcome::Starting.is_alive());
        assert!(!ProbeOutcome::Starting.is_ready());
        assert!(!ProbeOutcome::NotReady.is_alive());
        assert!(!ProbeOutcome::Failed.is_ready());
    }

    #[test]
    fn promote_mode_pg_ctl_flags() {
        assert_eq!(PromoteMode::Fast.pg_ctl_flags(), &["-W"]);
        assert!(PromoteMode::Safe.pg_ctl_flags().is_empty());
    }

    #[test]
    fn parse_lsn_round_trip() {
        assert_eq!(parse_lsn("0/0"), Some(0));
        assert_eq!(parse_lsn("1/100"), Some((1u64 << 32) | 0x100));
        assert_eq!(parse_lsn("FF/FFFFFFFF"), Some((0xFFu64 << 32) | 0xFFFFFFFF));
        assert_eq!(parse_lsn("bad"), None);
    }

    #[test]
    fn lsn_comparator_picks_furthest_replay() {
        let a = LsnReport {
            instance_name: "a".into(),
            current_lsn: "1/100".into(),
            timeline_id: 1,
        };
        let b = LsnReport {
            instance_name: "b".into(),
            current_lsn: "1/200".into(),
            timeline_id: 1,
        };
        assert!(!lsn_at_least_as_caught_up(&a, &b));
        assert!(lsn_at_least_as_caught_up(&b, &a));
    }

    #[test]
    fn lsn_comparator_breaks_ties_by_name() {
        let a = LsnReport {
            instance_name: "a".into(),
            current_lsn: "1/100".into(),
            timeline_id: 1,
        };
        let b = LsnReport {
            instance_name: "b".into(),
            current_lsn: "1/100".into(),
            timeline_id: 1,
        };
        assert!(lsn_at_least_as_caught_up(&a, &b));
        assert!(!lsn_at_least_as_caught_up(&b, &a));
    }

    #[test]
    fn routes_cover_canonical_surface() {
        let paths: Vec<&str> = ROUTES.iter().map(|(_, p)| *p).collect();
        for required in &[
            "/pg/status",
            "/pg/healthz",
            "/pg/readyz",
            "/pg/promote",
            "/pg/backup",
        ] {
            assert!(paths.contains(required), "missing route {required}");
        }
    }

    #[test]
    fn promote_request_serde_round_trip() {
        let req = PromoteRequest {
            mode: PromoteMode::Fast,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"mode\":\"fast\""));
        let back: PromoteRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn backup_request_serde_round_trip() {
        let req = BackupRequest {
            method: BackupMethod::BarmanObjectStore,
            immediate_checkpoint: true,
            backup_id: "b-1".into(),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"method\":\"barman_object_store\""));
        let back: BackupRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn instance_status_default_is_replica_in_recovery() {
        let s = InstanceStatus::default();
        assert!(!s.is_primary);
        assert!(s.is_in_recovery);
        assert_eq!(s.timeline_id, 1);
    }
}
