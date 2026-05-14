// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Encryption key rotation controller — scheduled SPI roll for IPsec
//! and per-node key cycling for WireGuard.
//!
//! Mirrors `pkg/ipsec/keyrotation.go` (the IPsec rotator) and
//! `pkg/wireguard/agent.go::rotateKeys`. The controller sequences the
//! rotation: install the new key alongside the old, wait for the
//! "drain" window so in-flight packets keep working, then revoke the
//! old key.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotationPhase {
    /// Old key live; nothing scheduled.
    Stable,
    /// New key installed alongside; both accepted.
    Drain,
    /// Old key revoked; only the new key is live.
    Switched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyVersion {
    pub spi: u32,
    pub key: Vec<u8>,
    pub installed_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotationState {
    pub current: KeyVersion,
    pub previous: Option<KeyVersion>,
    pub phase: RotationPhase,
    pub drain_started_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RotationError {
    #[error("rotation already in progress (phase {0:?})")]
    AlreadyRotating(RotationPhase),
    #[error("no rotation in progress")]
    NotRotating,
    #[error("drain window of {0}s has not elapsed")]
    DrainNotElapsed(u64),
    #[error("tenant {tenant} cannot mutate rotation state owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct KeyRotationController {
    pub tenant: TenantId,
    pub drain_seconds: u64,
    /// Per-target rotation state. Target is a stable identifier
    /// (e.g. `node-a` for WireGuard, `(src,dst)` for IPsec SAs).
    states: BTreeMap<String, RotationState>,
    next_spi: u32,
}

impl KeyRotationController {
    pub fn new(tenant: TenantId, drain_seconds: u64, initial_spi: u32) -> Self {
        Self {
            tenant, drain_seconds,
            states: BTreeMap::new(),
            next_spi: initial_spi,
        }
    }

    pub fn install_initial(&mut self, target: impl Into<String>, key: Vec<u8>, now_ns: u64) -> u32 {
        let target = target.into();
        let spi = self.next_spi;
        self.next_spi += 1;
        self.states.insert(target, RotationState {
            current: KeyVersion { spi, key, installed_ns: now_ns },
            previous: None,
            phase: RotationPhase::Stable,
            drain_started_ns: 0,
        });
        spi
    }

    /// Begin rotation: install the new key alongside, move to Drain.
    pub fn begin_rotation(&mut self, target: &str, new_key: Vec<u8>, now_ns: u64) -> Result<u32, RotationError> {
        let state = self.states.get_mut(target).ok_or(RotationError::NotRotating)?;
        if !matches!(state.phase, RotationPhase::Stable) {
            return Err(RotationError::AlreadyRotating(state.phase));
        }
        let new_spi = self.next_spi;
        self.next_spi += 1;
        let new_version = KeyVersion { spi: new_spi, key: new_key, installed_ns: now_ns };
        state.previous = Some(state.current.clone());
        state.current = new_version;
        state.phase = RotationPhase::Drain;
        state.drain_started_ns = now_ns;
        Ok(new_spi)
    }

    /// Complete rotation: revoke the old key. Returns Err until the drain
    /// window has elapsed.
    pub fn complete_rotation(&mut self, target: &str, now_ns: u64) -> Result<(), RotationError> {
        let drain_ns = self.drain_seconds * 1_000_000_000;
        let state = self.states.get_mut(target).ok_or(RotationError::NotRotating)?;
        if !matches!(state.phase, RotationPhase::Drain) {
            return Err(RotationError::NotRotating);
        }
        let elapsed = now_ns.saturating_sub(state.drain_started_ns);
        if elapsed < drain_ns {
            return Err(RotationError::DrainNotElapsed(self.drain_seconds));
        }
        state.previous = None;
        state.phase = RotationPhase::Switched;
        Ok(())
    }

    /// Re-stabilise (mark Switched → Stable so the next rotation can begin).
    pub fn stabilise(&mut self, target: &str) -> Result<(), RotationError> {
        let state = self.states.get_mut(target).ok_or(RotationError::NotRotating)?;
        if !matches!(state.phase, RotationPhase::Switched | RotationPhase::Stable) {
            return Err(RotationError::AlreadyRotating(state.phase));
        }
        state.phase = RotationPhase::Stable;
        state.drain_started_ns = 0;
        Ok(())
    }

    pub fn current_spi(&self, target: &str) -> Option<u32> {
        self.states.get(target).map(|s| s.current.spi)
    }

    pub fn previous_spi(&self, target: &str) -> Option<u32> {
        self.states.get(target).and_then(|s| s.previous.as_ref().map(|p| p.spi))
    }

    pub fn phase(&self, target: &str) -> Option<RotationPhase> {
        self.states.get(target).map(|s| s.phase)
    }

    pub fn target_count(&self) -> usize {
        self.states.len()
    }

    /// Forget a target entirely (e.g. node decommissioned).
    pub fn forget(&mut self, target: &str) -> bool {
        self.states.remove(target).is_some()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ipsec/keyrotation.go", "Rotator");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ctrl(tenant: TenantId) -> KeyRotationController {
        KeyRotationController::new(tenant, 30, 100)
    }

    // ── Install initial ─────────────────────────────────────────────────────

    #[test]
    fn install_initial_records_stable_phase() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Install.Initial", "tenant-kr-init");
        let mut c = ctrl(tenant);
        let spi = c.install_initial("node-a", vec![1, 2, 3], 100);
        assert_eq!(spi, 100);
        assert_eq!(c.phase("node-a"), Some(RotationPhase::Stable));
    }

    #[test]
    fn install_initial_assigns_monotonic_spi() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Install.MonotonicSPI", "tenant-kr-mono");
        let mut c = ctrl(tenant);
        let a = c.install_initial("node-a", vec![1], 100);
        let b = c.install_initial("node-b", vec![2], 100);
        assert_eq!(b, a + 1);
    }

    #[test]
    fn target_count_tracks_installs() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "TargetCount", "tenant-kr-cnt");
        let mut c = ctrl(tenant);
        c.install_initial("a", vec![1], 100);
        c.install_initial("b", vec![2], 100);
        assert_eq!(c.target_count(), 2);
    }

    // ── Begin rotation ──────────────────────────────────────────────────────

    #[test]
    fn begin_rotation_installs_new_key_in_drain_phase() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Begin", "tenant-kr-bg");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1, 2, 3], 100);
        let new_spi = c.begin_rotation("node-a", vec![4, 5, 6], 200).unwrap();
        assert_eq!(c.phase("node-a"), Some(RotationPhase::Drain));
        assert_eq!(c.current_spi("node-a"), Some(new_spi));
        assert_eq!(c.previous_spi("node-a"), Some(100));
    }

    #[test]
    fn begin_rotation_on_unknown_target_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Begin.Unknown", "tenant-kr-bgnf");
        let mut c = ctrl(tenant);
        let err = c.begin_rotation("ghost", vec![1], 100).unwrap_err();
        assert_eq!(err, RotationError::NotRotating);
    }

    #[test]
    fn begin_rotation_while_in_drain_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Begin.AlreadyRotating", "tenant-kr-bgar");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        let err = c.begin_rotation("node-a", vec![3], 300).unwrap_err();
        assert!(matches!(err, RotationError::AlreadyRotating(_)));
    }

    // ── Complete rotation ──────────────────────────────────────────────────

    #[test]
    fn complete_within_drain_window_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Complete.WithinDrain", "tenant-kr-cwd");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        let err = c.complete_rotation("node-a", 200 + 10_000_000_000).unwrap_err();
        assert!(matches!(err, RotationError::DrainNotElapsed(_)));
    }

    #[test]
    fn complete_after_drain_succeeds_and_revokes_old() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Complete.AfterDrain", "tenant-kr-cad");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        c.complete_rotation("node-a", 200 + 31_000_000_000).unwrap();
        assert_eq!(c.phase("node-a"), Some(RotationPhase::Switched));
        assert!(c.previous_spi("node-a").is_none());
    }

    #[test]
    fn complete_when_not_rotating_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Complete.NotRotating", "tenant-kr-cnr");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        let err = c.complete_rotation("node-a", 1000).unwrap_err();
        assert_eq!(err, RotationError::NotRotating);
    }

    #[test]
    fn complete_unknown_target_errors() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Complete.Unknown", "tenant-kr-cu");
        let mut c = ctrl(tenant);
        let err = c.complete_rotation("ghost", 1000).unwrap_err();
        assert_eq!(err, RotationError::NotRotating);
    }

    // ── Stabilise ──────────────────────────────────────────────────────────

    #[test]
    fn stabilise_after_switched_returns_to_stable() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Stabilise", "tenant-kr-stb");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        c.complete_rotation("node-a", 200 + 31_000_000_000).unwrap();
        c.stabilise("node-a").unwrap();
        assert_eq!(c.phase("node-a"), Some(RotationPhase::Stable));
    }

    #[test]
    fn stabilise_during_drain_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Stabilise.Drain", "tenant-kr-stbd");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        let err = c.stabilise("node-a").unwrap_err();
        assert!(matches!(err, RotationError::AlreadyRotating(RotationPhase::Drain)));
    }

    #[test]
    fn stabilise_unknown_returns_not_rotating() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Stabilise.Unknown", "tenant-kr-stbu");
        let mut c = ctrl(tenant);
        let err = c.stabilise("ghost").unwrap_err();
        assert_eq!(err, RotationError::NotRotating);
    }

    // ── Sequential rotations ───────────────────────────────────────────────

    #[test]
    fn second_rotation_after_stabilise_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Sequential", "tenant-kr-seq");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        c.complete_rotation("node-a", 200 + 31_000_000_000).unwrap();
        c.stabilise("node-a").unwrap();
        c.begin_rotation("node-a", vec![3], 100_000_000_000).unwrap();
        assert_eq!(c.phase("node-a"), Some(RotationPhase::Drain));
    }

    // ── SPI accounting ─────────────────────────────────────────────────────

    #[test]
    fn current_spi_advances_on_rotation() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "SPI.Advance", "tenant-kr-spi");
        let mut c = ctrl(tenant);
        let initial = c.install_initial("node-a", vec![1], 100);
        let next = c.begin_rotation("node-a", vec![2], 200).unwrap();
        assert_eq!(next, initial + 1);
        assert_eq!(c.current_spi("node-a"), Some(next));
    }

    #[test]
    fn previous_spi_set_during_drain_cleared_after_complete() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "SPI.Previous", "tenant-kr-prev");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        c.begin_rotation("node-a", vec![2], 200).unwrap();
        assert_eq!(c.previous_spi("node-a"), Some(100));
        c.complete_rotation("node-a", 200 + 31_000_000_000).unwrap();
        assert!(c.previous_spi("node-a").is_none());
    }

    #[test]
    fn current_spi_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "SPI.Current.NotFound", "tenant-kr-snf");
        let c = ctrl(tenant);
        assert!(c.current_spi("ghost").is_none());
    }

    // ── Forget ──────────────────────────────────────────────────────────────

    #[test]
    fn forget_drops_target() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Forget", "tenant-kr-fgt");
        let mut c = ctrl(tenant);
        c.install_initial("node-a", vec![1], 100);
        assert!(c.forget("node-a"));
        assert_eq!(c.target_count(), 0);
    }

    #[test]
    fn forget_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Forget.NotFound", "tenant-kr-fgtnf");
        let mut c = ctrl(tenant);
        assert!(!c.forget("ghost"));
    }

    // ── Multi-target ───────────────────────────────────────────────────────

    #[test]
    fn rotations_per_target_independent() {
        let (_c, tenant) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Multi.Independent", "tenant-kr-mi");
        let mut c = ctrl(tenant);
        c.install_initial("a", vec![1], 100);
        c.install_initial("b", vec![1], 100);
        c.begin_rotation("a", vec![2], 200).unwrap();
        // b is still stable.
        assert_eq!(c.phase("a"), Some(RotationPhase::Drain));
        assert_eq!(c.phase("b"), Some(RotationPhase::Stable));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn rotation_phase_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "Phase.Serde", "tenant-kr-pserde");
        for p in [RotationPhase::Stable, RotationPhase::Drain, RotationPhase::Switched] {
            let s = serde_json::to_string(&p).unwrap();
            let back: RotationPhase = serde_json::from_str(&s).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn key_version_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipsec/keyrotation.go", "KeyVersion.Serde", "tenant-kr-kvserde");
        let k = KeyVersion { spi: 100, key: vec![1, 2, 3], installed_ns: 200 };
        let s = serde_json::to_string(&k).unwrap();
        let back: KeyVersion = serde_json::from_str(&s).unwrap();
        assert_eq!(back, k);
    }
}
