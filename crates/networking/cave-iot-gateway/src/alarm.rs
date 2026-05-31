// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Alarms. (RED.)

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
