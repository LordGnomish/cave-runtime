// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge device twin — KubeEdge `edge/pkg/devicetwin`.
//!
//! A device twin holds, per attribute, the **Expected** value (the desired
//! state the cloud wants) and the **Actual** value (what the device last
//! reported), each governed by a `{cloud, edge}` `TwinVersion`
//! (ExpectedVersion / ActualVersion upstream).
//!
//! Conflict resolution follows KubeEdge `dtcommon.DealVersion`:
//!
//!   * an edge-local **actual** update (a device report, the `RestDealType`
//!     path) bumps the edge version;
//!   * a cloud **expected** update (the sync path) is accepted only when the
//!     edge version it acknowledges is at least the edge version we hold —
//!     i.e. the cloud has seen our latest local change. If the edge has moved
//!     ahead, the update is rejected so the unsynced edge change survives.
//!
//! The **twin delta** is every attribute whose expected value differs from
//! its actual value — exactly the payload the twin module publishes toward
//! the device. The **membership** module binds devices to the edge node.

use std::collections::{BTreeMap, BTreeSet};

/// A `{cloud, edge}` version vector for one twin attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TwinVersion {
    pub cloud: u64,
    pub edge: u64,
}

/// One entry of a computed twin delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwinDelta {
    pub attr: String,
    pub expected: String,
}

#[derive(Debug, Clone, Default)]
struct Twin {
    expected: Option<String>,
    actual: Option<String>,
    version: TwinVersion,
}

/// The device-twin store for an edge node.
#[derive(Debug, Clone, Default)]
pub struct DeviceTwin {
    /// Devices bound to this node (membership module).
    members: BTreeSet<String>,
    /// Per-(device, attribute) twin state.
    twins: BTreeMap<(String, String), Twin>,
}

impl DeviceTwin {
    pub fn new() -> Self {
        Self::default()
    }

    // ── membership ──────────────────────────────────────────────────────

    pub fn bind_device(&mut self, device: &str) {
        self.members.insert(device.to_string());
    }

    pub fn unbind_device(&mut self, device: &str) {
        self.members.remove(device);
        self.twins.retain(|(d, _), _| d != device);
    }

    pub fn is_member(&self, device: &str) -> bool {
        self.members.contains(device)
    }

    pub fn members(&self) -> Vec<String> {
        self.members.iter().cloned().collect()
    }

    // ── twin state ──────────────────────────────────────────────────────

    fn entry(&mut self, device: &str, attr: &str) -> &mut Twin {
        self.twins
            .entry((device.to_string(), attr.to_string()))
            .or_default()
    }

    /// Edge-local actual update (device report). Bumps the edge version
    /// (`DealVersion` RestDealType) and is always applied.
    pub fn update_actual(&mut self, device: &str, attr: &str, value: &str) {
        let t = self.entry(device, attr);
        t.actual = Some(value.to_string());
        t.version.edge += 1;
    }

    /// Cloud expected update (sync). Accepted only when `req.edge >=` the
    /// edge version we hold (the cloud has seen our latest change). On accept
    /// the stored version is set to `req`. Returns whether it was applied.
    pub fn update_expected(
        &mut self,
        device: &str,
        attr: &str,
        value: &str,
        req: TwinVersion,
    ) -> bool {
        let t = self.entry(device, attr);
        if t.version.edge > req.edge {
            // Edge has unsynced local changes — reject to avoid losing them.
            return false;
        }
        t.expected = Some(value.to_string());
        t.version = req;
        true
    }

    pub fn expected(&self, device: &str, attr: &str) -> Option<String> {
        self.twins
            .get(&(device.to_string(), attr.to_string()))
            .and_then(|t| t.expected.clone())
    }

    pub fn actual(&self, device: &str, attr: &str) -> Option<String> {
        self.twins
            .get(&(device.to_string(), attr.to_string()))
            .and_then(|t| t.actual.clone())
    }

    pub fn version(&self, device: &str, attr: &str) -> Option<TwinVersion> {
        self.twins
            .get(&(device.to_string(), attr.to_string()))
            .map(|t| t.version)
    }

    /// The twin delta for a device: every attribute with an expected value
    /// that differs from its actual value, sorted by attribute name.
    pub fn delta(&self, device: &str) -> Vec<TwinDelta> {
        self.twins
            .iter()
            .filter(|((d, _), _)| d == device)
            .filter_map(|((_, attr), t)| {
                let expected = t.expected.as_ref()?;
                if t.actual.as_ref() != Some(expected) {
                    Some(TwinDelta {
                        attr: attr.clone(),
                        expected: expected.clone(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}
