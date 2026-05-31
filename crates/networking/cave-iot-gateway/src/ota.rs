// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! OTA firmware/software campaigns.
//!
//! Ports the ThingsBoard `OtaPackage` entity + `OtaPackageUpdateStatus`
//! state machine and the per-device rollout tracking of a firmware campaign.
//! The download itself (HTTP range serving, device-side flash) is a runtime
//! data-plane concern; this owns the package metadata, the legal status
//! transitions and the campaign progress roll-up.

use crate::{IotError, Result};
use std::collections::HashMap;

/// OTA package kind (`OtaPackageType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OtaType {
    Firmware,
    Software,
}

/// An OTA package (`OtaPackage`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OtaPackage {
    pub id: String,
    pub title: String,
    pub version: String,
    pub package_type: OtaType,
    pub checksum: String,
    pub checksum_algorithm: String,
    pub size: u64,
}

impl OtaPackage {
    /// Verify a device-reported checksum against the package's expected one.
    pub fn verify(&self, reported_checksum: &str) -> bool {
        !self.checksum.is_empty() && self.checksum == reported_checksum
    }
}

/// Per-device update status (`OtaPackageUpdateStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OtaStatus {
    Queued,
    Initiated,
    Downloading,
    Downloaded,
    Verified,
    Updating,
    Updated,
    Failed,
}

impl OtaStatus {
    /// The states reachable in one legal step from `self`. Any non-terminal
    /// state may also fail. `Updated` / `Failed` are terminal.
    fn allowed_next(self) -> &'static [OtaStatus] {
        use OtaStatus::*;
        match self {
            Queued => &[Initiated, Failed],
            Initiated => &[Downloading, Failed],
            Downloading => &[Downloaded, Failed],
            Downloaded => &[Verified, Failed],
            Verified => &[Updating, Failed],
            Updating => &[Updated, Failed],
            Updated => &[],
            Failed => &[],
        }
    }

    fn is_terminal(self) -> bool {
        matches!(self, OtaStatus::Updated | OtaStatus::Failed)
    }
}

/// Roll-up of campaign progress.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OtaProgress {
    pub total: usize,
    pub queued: usize,
    pub in_progress: usize,
    pub updated: usize,
    pub failed: usize,
}

/// An OTA campaign assigning a package to a fixed device set.
#[derive(Debug, Clone)]
pub struct OtaCampaign {
    pub id: String,
    pub package: OtaPackage,
    statuses: HashMap<String, OtaStatus>,
}

impl OtaCampaign {
    pub fn new(id: &str, package: OtaPackage, device_ids: &[String]) -> OtaCampaign {
        let statuses = device_ids
            .iter()
            .map(|d| (d.clone(), OtaStatus::Queued))
            .collect();
        OtaCampaign { id: id.to_string(), package, statuses }
    }

    pub fn status(&self, device_id: &str) -> Option<OtaStatus> {
        self.statuses.get(device_id).copied()
    }

    /// Advance a device to a new status, enforcing the legal transition graph.
    pub fn advance(&mut self, device_id: &str, next: OtaStatus) -> Result<()> {
        let current = self
            .statuses
            .get(device_id)
            .copied()
            .ok_or_else(|| IotError::NotFound(format!("device {device_id} not in campaign")))?;
        if !current.allowed_next().contains(&next) {
            return Err(IotError::IllegalTransition(format!(
                "{current:?} → {next:?}"
            )));
        }
        self.statuses.insert(device_id.to_string(), next);
        Ok(())
    }

    pub fn progress(&self) -> OtaProgress {
        let mut p = OtaProgress {
            total: self.statuses.len(),
            queued: 0,
            in_progress: 0,
            updated: 0,
            failed: 0,
        };
        for s in self.statuses.values() {
            match s {
                OtaStatus::Queued => p.queued += 1,
                OtaStatus::Updated => p.updated += 1,
                OtaStatus::Failed => p.failed += 1,
                _ => p.in_progress += 1,
            }
        }
        p
    }

    /// True when every device has reached a terminal status.
    pub fn is_complete(&self) -> bool {
        self.statuses.values().all(|s| s.is_terminal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> OtaPackage {
        OtaPackage {
            id: "p1".into(),
            title: "fw".into(),
            version: "2.0.0".into(),
            package_type: OtaType::Firmware,
            checksum: "abc123".into(),
            checksum_algorithm: "SHA256".into(),
            size: 1024,
        }
    }

    #[test]
    fn new_campaign_starts_all_queued() {
        let c = OtaCampaign::new("c1", pkg(), &["d1".into(), "d2".into()]);
        let p = c.progress();
        assert_eq!(p.total, 2);
        assert_eq!(p.queued, 2);
        assert_eq!(p.updated, 0);
        assert_eq!(c.status("d1"), Some(OtaStatus::Queued));
    }

    #[test]
    fn happy_path_transitions_succeed() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        for s in [
            OtaStatus::Initiated,
            OtaStatus::Downloading,
            OtaStatus::Downloaded,
            OtaStatus::Verified,
            OtaStatus::Updating,
            OtaStatus::Updated,
        ] {
            assert!(c.advance("d1", s).is_ok(), "transition to {s:?} should be allowed");
        }
        assert_eq!(c.status("d1"), Some(OtaStatus::Updated));
    }

    #[test]
    fn skipping_states_is_rejected() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        // Queued cannot jump straight to Updated.
        assert!(c.advance("d1", OtaStatus::Updated).is_err());
    }

    #[test]
    fn any_state_can_fail() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        c.advance("d1", OtaStatus::Initiated).unwrap();
        c.advance("d1", OtaStatus::Downloading).unwrap();
        assert!(c.advance("d1", OtaStatus::Failed).is_ok());
        // Failed is terminal — no resurrection.
        assert!(c.advance("d1", OtaStatus::Downloading).is_err());
    }

    #[test]
    fn progress_and_completion() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into(), "d2".into()]);
        // d1 → Updated, d2 → Failed.
        for s in [
            OtaStatus::Initiated,
            OtaStatus::Downloading,
            OtaStatus::Downloaded,
            OtaStatus::Verified,
            OtaStatus::Updating,
            OtaStatus::Updated,
        ] {
            c.advance("d1", s).unwrap();
        }
        c.advance("d2", OtaStatus::Failed).unwrap();
        let p = c.progress();
        assert_eq!(p.updated, 1);
        assert_eq!(p.failed, 1);
        assert!(c.is_complete());
    }

    #[test]
    fn checksum_verification() {
        let p = pkg();
        assert!(p.verify("abc123"));
        assert!(!p.verify("deadbeef"));
    }

    #[test]
    fn advance_unknown_device_errors() {
        let mut c = OtaCampaign::new("c1", pkg(), &["d1".into()]);
        assert!(c.advance("ghost", OtaStatus::Initiated).is_err());
    }
}
