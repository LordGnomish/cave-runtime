// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Device twin / attribute sync. (RED.)

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
