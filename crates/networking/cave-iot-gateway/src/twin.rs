// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Device twin / attribute sync.
//!
//! Ports two related ThingsBoard concepts:
//! - the **attribute store** with its three scopes (`CLIENT_SCOPE`,
//!   `SERVER_SCOPE`, `SHARED_SCOPE`) keyed per device + scope;
//! - a **device twin** with desired (server-set shared) vs reported
//!   (device-set client) values, a monotonic version, and a [`DeviceTwin::delta`]
//!   that yields the shared-attribute changes still pending delivery to the
//!   device — the basis of ThingsBoard's shared-attribute update push.

use crate::{KvMap, KvValue};
use std::collections::HashMap;

/// Attribute scope (`AttributeScope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttributeScope {
    /// Written by the device (reported state).
    Client,
    /// Written by the platform, invisible to the device.
    Server,
    /// Written by the platform, pushed to the device (desired state).
    Shared,
}

/// Per-(device, scope) attribute store.
#[derive(Debug, Default)]
pub struct AttributeStore {
    map: HashMap<(String, AttributeScope), KvMap>,
}

impl AttributeStore {
    pub fn new() -> AttributeStore {
        AttributeStore::default()
    }

    pub fn save(&mut self, device_id: &str, scope: AttributeScope, key: &str, value: KvValue) {
        self.map
            .entry((device_id.to_string(), scope))
            .or_default()
            .insert(key.to_string(), value);
    }

    pub fn get(&self, device_id: &str, scope: AttributeScope, key: &str) -> Option<&KvValue> {
        self.map
            .get(&(device_id.to_string(), scope))
            .and_then(|m| m.get(key))
    }

    pub fn delete(&mut self, device_id: &str, scope: AttributeScope, key: &str) -> bool {
        self.map
            .get_mut(&(device_id.to_string(), scope))
            .map(|m| m.remove(key).is_some())
            .unwrap_or(false)
    }

    /// All attributes in one scope for a device (empty if none).
    pub fn scope(&self, device_id: &str, scope: AttributeScope) -> KvMap {
        self.map
            .get(&(device_id.to_string(), scope))
            .cloned()
            .unwrap_or_default()
    }
}

/// A device twin: desired (shared) vs reported (client) attribute values.
#[derive(Debug, Clone)]
pub struct DeviceTwin {
    pub device_id: String,
    pub version: u64,
    desired: KvMap,
    reported: KvMap,
}

impl DeviceTwin {
    pub fn new(device_id: &str) -> DeviceTwin {
        DeviceTwin {
            device_id: device_id.to_string(),
            version: 0,
            desired: KvMap::new(),
            reported: KvMap::new(),
        }
    }

    /// Server sets a desired (shared) attribute. Bumps the version only when
    /// the value actually changes.
    pub fn update_desired(&mut self, key: &str, value: KvValue) {
        let changed = self.desired.get(key) != Some(&value);
        self.desired.insert(key.to_string(), value);
        if changed {
            self.version += 1;
        }
    }

    /// Device reports a current (client) attribute value.
    pub fn report(&mut self, key: &str, value: KvValue) {
        self.reported.insert(key.to_string(), value);
    }

    /// The desired attributes whose reported value does not yet match — the
    /// set of changes still pending delivery to the device.
    pub fn delta(&self) -> KvMap {
        self.desired
            .iter()
            .filter(|(k, v)| self.reported.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// True when every desired attribute has been confirmed by the device.
    pub fn is_synced(&self) -> bool {
        self.delta().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KvValue;

    #[test]
    fn attribute_store_scopes_are_isolated() {
        let mut store = AttributeStore::new();
        store.save("dev", AttributeScope::Server, "fw", KvValue::Str("1.0".into()));
        store.save("dev", AttributeScope::Shared, "target", KvValue::Long(5));
        assert_eq!(
            store.get("dev", AttributeScope::Server, "fw"),
            Some(&KvValue::Str("1.0".into()))
        );
        // Same key absent in a different scope.
        assert!(store.get("dev", AttributeScope::Client, "fw").is_none());
        assert_eq!(store.scope("dev", AttributeScope::Shared).len(), 1);
    }

    #[test]
    fn deleting_attribute_removes_it() {
        let mut store = AttributeStore::new();
        store.save("d", AttributeScope::Shared, "k", KvValue::Long(1));
        assert!(store.delete("d", AttributeScope::Shared, "k"));
        assert!(store.get("d", AttributeScope::Shared, "k").is_none());
        assert!(!store.delete("d", AttributeScope::Shared, "k"));
    }

    #[test]
    fn updating_desired_bumps_version_and_creates_delta() {
        let mut twin = DeviceTwin::new("dev");
        assert_eq!(twin.version, 0);
        twin.update_desired("fan_speed", KvValue::Long(3));
        assert_eq!(twin.version, 1);
        // The device has not reported yet → the change is pending.
        let delta = twin.delta();
        assert_eq!(delta.get("fan_speed"), Some(&KvValue::Long(3)));
        assert!(!twin.is_synced());
    }

    #[test]
    fn reporting_matching_value_clears_delta() {
        let mut twin = DeviceTwin::new("dev");
        twin.update_desired("fan_speed", KvValue::Long(3));
        twin.report("fan_speed", KvValue::Long(3));
        assert!(twin.delta().is_empty());
        assert!(twin.is_synced());
    }

    #[test]
    fn reporting_stale_value_keeps_delta() {
        let mut twin = DeviceTwin::new("dev");
        twin.update_desired("fan_speed", KvValue::Long(3));
        twin.report("fan_speed", KvValue::Long(1)); // device still at old value
        assert_eq!(twin.delta().get("fan_speed"), Some(&KvValue::Long(3)));
        assert!(!twin.is_synced());
    }

    #[test]
    fn partial_sync_reports_only_remaining_delta() {
        let mut twin = DeviceTwin::new("dev");
        twin.update_desired("a", KvValue::Long(1));
        twin.update_desired("b", KvValue::Long(2));
        twin.report("a", KvValue::Long(1));
        let delta = twin.delta();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta.get("b"), Some(&KvValue::Long(2)));
    }
}
