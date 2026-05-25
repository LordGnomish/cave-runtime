// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core DNS management: sync, drift detection, validation, change application, health probes.

use crate::models::{ChangeAction, DnsChange, DnsDrift, DnsRecord, DriftType, RecordData, RecordType};
use chrono::Utc;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use uuid::Uuid;

pub struct SyncResult {
    pub zone_id: Uuid,
    pub changes_applied: usize,
    pub errors: Vec<String>,
}

/// Compare desired records against actual and record the necessary changes.
/// When `dry_run` is false, changes are written to `store` and marked applied.
pub fn sync_records(
    zone_id: Uuid,
    desired: &[DnsRecord],
    actual: &[DnsRecord],
    store: &Arc<Mutex<Vec<DnsChange>>>,
    dry_run: bool,
) -> SyncResult {
    let mut new_changes: Vec<DnsChange> = Vec::new();

    // Records to create
    for d in desired {
        if !actual.iter().any(|a| a.name == d.name && a.record_type == d.record_type) {
            info!(name = %d.name, "DNS record needs creation");
            new_changes.push(DnsChange::new(zone_id, ChangeAction::Create, d.clone()));
        }
    }

    // Managed records to delete
    for a in actual.iter().filter(|r| r.managed) {
        if !desired.iter().any(|d| d.name == a.name && d.record_type == a.record_type) {
            info!(name = %a.name, "DNS record needs deletion");
            new_changes.push(DnsChange::new(zone_id, ChangeAction::Delete, a.clone()));
        }
    }

    // Records to update (name+type match, data or TTL differs)
    for d in desired {
        if let Some(a) = actual
            .iter()
            .find(|a| a.name == d.name && a.record_type == d.record_type)
        {
            if a.data != d.data || a.ttl != d.ttl {
                info!(name = %d.name, "DNS record needs update");
                new_changes.push(DnsChange::new(zone_id, ChangeAction::Update, d.clone()));
            }
        }
    }

    let count = new_changes.len();

    if !dry_run {
        let mut locked = store.lock().unwrap();
        for mut change in new_changes {
            change.applied = true;
            change.applied_at = Some(Utc::now());
            locked.push(change);
        }
    }

    SyncResult { zone_id, changes_applied: count, errors: Vec::new() }
}

/// Compare desired vs actual and return every discrepancy.
pub fn detect_drift(desired: &[DnsRecord], actual: &[DnsRecord]) -> Vec<DnsDrift> {
    let mut drifts = Vec::new();

    for d in desired {
        match actual.iter().find(|a| a.name == d.name && a.record_type == d.record_type) {
            None => drifts.push(DnsDrift {
                zone_id: d.zone_id,
                record_name: d.name.clone(),
                record_type: d.record_type.clone(),
                desired: Some(format!("{:?}", d.data)),
                actual: None,
                drift_type: DriftType::Missing,
            }),
            Some(a) if a.data != d.data => drifts.push(DnsDrift {
                zone_id: d.zone_id,
                record_name: d.name.clone(),
                record_type: d.record_type.clone(),
                desired: Some(format!("{:?}", d.data)),
                actual: Some(format!("{:?}", a.data)),
                drift_type: DriftType::Modified,
            }),
            _ => {}
        }
    }

    for a in actual.iter().filter(|r| r.managed) {
        if !desired.iter().any(|d| d.name == a.name && d.record_type == a.record_type) {
            drifts.push(DnsDrift {
                zone_id: a.zone_id,
                record_name: a.name.clone(),
                record_type: a.record_type.clone(),
                desired: None,
                actual: Some(format!("{:?}", a.data)),
                drift_type: DriftType::Extra,
            });
        }
    }

    drifts
}

/// Validate records for logical correctness; returns a list of error messages.
pub fn validate_records(records: &[DnsRecord]) -> Vec<String> {
    let mut errors = Vec::new();

    for r in records {
        if r.name.is_empty() {
            errors.push(format!("record {} has an empty name", r.id));
        }
        if r.ttl == 0 {
            errors.push(format!("record {} has a zero TTL", r.id));
        }
        match &r.data {
            RecordData::A { address } | RecordData::Aaaa { address } => {
                if address.is_empty() {
                    errors.push(format!("record {} has an empty address", r.id));
                }
            }
            RecordData::Cname { target } => {
                if target.is_empty() {
                    errors.push(format!("CNAME record {} has an empty target", r.id));
                }
            }
            RecordData::Mx { mail_server, .. } => {
                if mail_server.is_empty() {
                    errors.push(format!("MX record {} has an empty mail_server", r.id));
                }
            }
            RecordData::Txt { text } => {
                if text.len() > 255 {
                    warn!(id = %r.id, "TXT record exceeds 255 chars; may need splitting");
                }
            }
            RecordData::Srv { target, port, .. } => {
                if target.is_empty() {
                    errors.push(format!("SRV record {} has an empty target", r.id));
                }
                if *port == 0 {
                    errors.push(format!("SRV record {} has a zero port", r.id));
                }
            }
        }
    }

    errors
}

/// Mark all unapplied changes in the store as applied.
pub fn apply_changes(store: &Arc<Mutex<Vec<DnsChange>>>) -> usize {
    let mut locked = store.lock().unwrap();
    let mut count = 0;
    for change in locked.iter_mut().filter(|c| !c.applied) {
        change.applied = true;
        change.applied_at = Some(Utc::now());
        count += 1;
        info!(
            record = %change.record.name,
            action = ?change.action,
            "Applied DNS change"
        );
    }
    count
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EndpointHealth {
    pub name: String,
    pub address: String,
    pub healthy: bool,
    pub checked_at: chrono::DateTime<Utc>,
}

/// TCP-probe A records on port 80 to determine reachability.
pub async fn health_check_endpoints(records: &[DnsRecord]) -> Vec<EndpointHealth> {
    let mut results = Vec::new();

    for record in records.iter().filter(|r| r.record_type == RecordType::A) {
        let address = match &record.data {
            RecordData::A { address } => address.clone(),
            _ => continue,
        };

        let healthy = tokio::net::TcpStream::connect(format!("{address}:80"))
            .await
            .is_ok();

        results.push(EndpointHealth {
            name: record.name.clone(),
            address,
            healthy,
            checked_at: Utc::now(),
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DnsProvider, RecordData, RecordType};

    fn make_a(zone_id: Uuid, name: &str, addr: &str) -> DnsRecord {
        DnsRecord::new(
            zone_id,
            name.into(),
            RecordType::A,
            300,
            RecordData::A { address: addr.into() },
        )
    }

    #[test]
    fn test_detect_drift_missing() {
        let zone_id = Uuid::new_v4();
        let desired = vec![make_a(zone_id, "api", "1.2.3.4")];
        let drifts = detect_drift(&desired, &[]);
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].drift_type, DriftType::Missing);
    }

    #[test]
    fn test_detect_drift_modified() {
        let zone_id = Uuid::new_v4();
        let desired = vec![make_a(zone_id, "api", "1.2.3.4")];
        let actual = vec![make_a(zone_id, "api", "9.9.9.9")];
        let drifts = detect_drift(&desired, &actual);
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].drift_type, DriftType::Modified);
    }

    #[test]
    fn test_detect_drift_clean() {
        let zone_id = Uuid::new_v4();
        let r = make_a(zone_id, "api", "1.2.3.4");
        assert!(detect_drift(&[r.clone()], &[r]).is_empty());
    }

    #[test]
    fn test_detect_drift_extra() {
        let zone_id = Uuid::new_v4();
        let actual = vec![make_a(zone_id, "extra", "5.5.5.5")];
        let drifts = detect_drift(&[], &actual);
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].drift_type, DriftType::Extra);
    }

    #[test]
    fn test_validate_empty_name() {
        let zone_id = Uuid::new_v4();
        let r = make_a(zone_id, "", "1.2.3.4");
        let errors = validate_records(&[r]);
        assert!(!errors.is_empty());
        assert!(errors[0].contains("empty name"));
    }

    #[test]
    fn test_validate_zero_ttl() {
        let zone_id = Uuid::new_v4();
        let r = DnsRecord::new(
            zone_id,
            "api".into(),
            RecordType::A,
            0,
            RecordData::A { address: "1.1.1.1".into() },
        );
        let errors = validate_records(&[r]);
        assert!(errors.iter().any(|e| e.contains("zero TTL")));
    }

    #[test]
    fn test_sync_creates_change() {
        let zone_id = Uuid::new_v4();
        let desired = vec![make_a(zone_id, "api", "1.2.3.4")];
        let store = Arc::new(Mutex::new(Vec::new()));
        let result = sync_records(zone_id, &desired, &[], &store, false);
        assert_eq!(result.changes_applied, 1);
        let locked = store.lock().unwrap();
        assert_eq!(locked[0].action, ChangeAction::Create);
    }

    #[test]
    fn test_sync_dry_run_no_store() {
        let zone_id = Uuid::new_v4();
        let desired = vec![make_a(zone_id, "api", "1.2.3.4")];
        let store = Arc::new(Mutex::new(Vec::new()));
        let result = sync_records(zone_id, &desired, &[], &store, true);
        assert_eq!(result.changes_applied, 1);
        // dry_run: store must remain empty
        assert!(store.lock().unwrap().is_empty());
    }

    #[test]
    fn test_apply_changes() {
        let zone_id = Uuid::new_v4();
        let r = make_a(zone_id, "x", "1.1.1.1");
        let mut change = DnsChange::new(zone_id, ChangeAction::Create, r);
        change.applied = false;
        let store = Arc::new(Mutex::new(vec![change]));
        let applied = apply_changes(&store);
        assert_eq!(applied, 1);
        assert!(store.lock().unwrap()[0].applied);
    }
}
