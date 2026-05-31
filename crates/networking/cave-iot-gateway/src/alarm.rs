// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Alarms — ThingsBoard `Alarm` + `AlarmService` lifecycle.
//!
//! Alarms are deduplicated per `(originator, type)` while active: raising the
//! same alarm type again updates the existing alarm (escalating severity to
//! the more-severe value, advancing `end_ts`) rather than creating a new one.
//! The status is the ThingsBoard cross-product of active/cleared ×
//! ack/unack; clearing preserves the ack bit. Once cleared, the next raise of
//! that type opens a fresh alarm. Clocks are injected.

use crate::{IotError, Result};
use std::collections::HashMap;

/// Alarm severity (`AlarmSeverity`), Critical most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlarmSeverity {
    Critical,
    Major,
    Minor,
    Warning,
    Indeterminate,
}

impl AlarmSeverity {
    /// Lower rank = more severe (Critical = 0).
    pub fn rank(self) -> u8 {
        match self {
            AlarmSeverity::Critical => 0,
            AlarmSeverity::Major => 1,
            AlarmSeverity::Minor => 2,
            AlarmSeverity::Warning => 3,
            AlarmSeverity::Indeterminate => 4,
        }
    }

    /// The more-severe of two severities.
    fn escalate(self, other: AlarmSeverity) -> AlarmSeverity {
        if other.rank() < self.rank() {
            other
        } else {
            self
        }
    }
}

/// Alarm status (`AlarmStatus`): active/cleared × ack/unack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlarmStatus {
    ActiveUnack,
    ActiveAck,
    ClearedUnack,
    ClearedAck,
}

impl AlarmStatus {
    pub fn is_active(self) -> bool {
        matches!(self, AlarmStatus::ActiveUnack | AlarmStatus::ActiveAck)
    }
    pub fn is_ack(self) -> bool {
        matches!(self, AlarmStatus::ActiveAck | AlarmStatus::ClearedAck)
    }
}

/// An alarm instance (`Alarm`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Alarm {
    pub id: String,
    pub device_id: String,
    pub alarm_type: String,
    pub severity: AlarmSeverity,
    pub status: AlarmStatus,
    pub start_ts: i64,
    pub end_ts: i64,
}

/// Alarm store + lifecycle service.
#[derive(Debug, Default)]
pub struct AlarmService {
    alarms: HashMap<String, Alarm>,
    /// (device, type) → id of the currently-active alarm, if any.
    active_index: HashMap<(String, String), String>,
    seq: u64,
}

impl AlarmService {
    pub fn new() -> AlarmService {
        AlarmService::default()
    }

    pub fn len(&self) -> usize {
        self.alarms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.alarms.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Alarm> {
        self.alarms.get(id)
    }

    /// Raise an alarm. If one of the same `(device, type)` is already active,
    /// update it (escalate severity, advance end_ts) and return its id; else
    /// create a fresh `ACTIVE_UNACK` alarm.
    pub fn create_or_update(
        &mut self,
        device_id: &str,
        alarm_type: &str,
        severity: AlarmSeverity,
        now_ms: i64,
    ) -> String {
        let key = (device_id.to_string(), alarm_type.to_string());
        if let Some(existing_id) = self.active_index.get(&key).cloned() {
            if let Some(a) = self.alarms.get_mut(&existing_id) {
                a.severity = a.severity.escalate(severity);
                a.end_ts = now_ms;
                return existing_id;
            }
        }
        self.seq += 1;
        let id = format!("alarm-{}", self.seq);
        self.alarms.insert(
            id.clone(),
            Alarm {
                id: id.clone(),
                device_id: device_id.to_string(),
                alarm_type: alarm_type.to_string(),
                severity,
                status: AlarmStatus::ActiveUnack,
                start_ts: now_ms,
                end_ts: now_ms,
            },
        );
        self.active_index.insert(key, id.clone());
        id
    }

    /// Acknowledge an alarm (Active*Unack → ActiveAck; Cleared*Unack → ClearedAck).
    pub fn ack(&mut self, id: &str, _now_ms: i64) -> Result<()> {
        let a = self
            .alarms
            .get_mut(id)
            .ok_or_else(|| IotError::NotFound(format!("alarm {id}")))?;
        a.status = match a.status {
            AlarmStatus::ActiveUnack => AlarmStatus::ActiveAck,
            AlarmStatus::ClearedUnack => AlarmStatus::ClearedAck,
            other => other,
        };
        Ok(())
    }

    /// Clear an alarm, preserving its ack bit and dropping it from the active
    /// index so the next raise of that type opens a new alarm.
    pub fn clear(&mut self, id: &str, now_ms: i64) -> Result<()> {
        let a = self
            .alarms
            .get_mut(id)
            .ok_or_else(|| IotError::NotFound(format!("alarm {id}")))?;
        a.status = match a.status {
            AlarmStatus::ActiveAck => AlarmStatus::ClearedAck,
            AlarmStatus::ActiveUnack => AlarmStatus::ClearedUnack,
            other => other,
        };
        a.end_ts = now_ms;
        let key = (a.device_id.clone(), a.alarm_type.clone());
        if self.active_index.get(&key) == Some(&id.to_string()) {
            self.active_index.remove(&key);
        }
        Ok(())
    }

    /// All currently-active alarms for a device.
    pub fn active_alarms(&self, device_id: &str) -> Vec<&Alarm> {
        let mut v: Vec<&Alarm> = self
            .alarms
            .values()
            .filter(|a| a.device_id == device_id && a.status.is_active())
            .collect();
        v.sort_by_key(|a| a.start_ts);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ranks_critical_highest() {
        assert!(AlarmSeverity::Critical.rank() < AlarmSeverity::Major.rank());
        assert!(AlarmSeverity::Major.rank() < AlarmSeverity::Warning.rank());
        assert!(AlarmSeverity::Warning.rank() < AlarmSeverity::Indeterminate.rank());
    }

    #[test]
    fn create_yields_active_unack() {
        let mut svc = AlarmService::new();
        let id = svc.create_or_update("d1", "High Temperature", AlarmSeverity::Warning, 100);
        let a = svc.get(&id).unwrap();
        assert_eq!(a.status, AlarmStatus::ActiveUnack);
        assert_eq!(a.start_ts, 100);
        assert!(a.status.is_active());
        assert!(!a.status.is_ack());
    }

    #[test]
    fn duplicate_active_type_updates_existing_and_escalates() {
        let mut svc = AlarmService::new();
        let id1 = svc.create_or_update("d1", "HT", AlarmSeverity::Warning, 100);
        let id2 = svc.create_or_update("d1", "HT", AlarmSeverity::Critical, 200);
        // Same alarm reused (no second alarm created).
        assert_eq!(id1, id2);
        assert_eq!(svc.len(), 1);
        let a = svc.get(&id1).unwrap();
        // Severity escalates to the more-severe value; end ts advances.
        assert_eq!(a.severity, AlarmSeverity::Critical);
        assert_eq!(a.end_ts, 200);
    }

    #[test]
    fn ack_then_clear_tracks_status() {
        let mut svc = AlarmService::new();
        let id = svc.create_or_update("d1", "HT", AlarmSeverity::Major, 100);
        svc.ack(&id, 150).unwrap();
        assert_eq!(svc.get(&id).unwrap().status, AlarmStatus::ActiveAck);
        svc.clear(&id, 300).unwrap();
        assert_eq!(svc.get(&id).unwrap().status, AlarmStatus::ClearedAck);
        assert_eq!(svc.get(&id).unwrap().end_ts, 300);
    }

    #[test]
    fn clearing_unacked_alarm_keeps_unack() {
        let mut svc = AlarmService::new();
        let id = svc.create_or_update("d1", "HT", AlarmSeverity::Minor, 100);
        svc.clear(&id, 200).unwrap();
        assert_eq!(svc.get(&id).unwrap().status, AlarmStatus::ClearedUnack);
    }

    #[test]
    fn new_alarm_created_after_clear() {
        let mut svc = AlarmService::new();
        let id1 = svc.create_or_update("d1", "HT", AlarmSeverity::Warning, 100);
        svc.clear(&id1, 200).unwrap();
        // The type is no longer active → a fresh alarm is created.
        let id2 = svc.create_or_update("d1", "HT", AlarmSeverity::Warning, 300);
        assert_ne!(id1, id2);
        assert_eq!(svc.len(), 2);
    }

    #[test]
    fn active_alarms_filters_cleared() {
        let mut svc = AlarmService::new();
        let a = svc.create_or_update("d1", "A", AlarmSeverity::Major, 100);
        let _b = svc.create_or_update("d1", "B", AlarmSeverity::Minor, 100);
        svc.clear(&a, 200).unwrap();
        let active = svc.active_alarms("d1");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].alarm_type, "B");
    }
}
