// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory vulnerability store.  Keyed by `(source, vuln_id)`.

use crate::error::{Error, Result};
use crate::models::{Vulnerability, VulnSource};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct VulnStore {
    by_key: RwLock<HashMap<(VulnSource, String), Vulnerability>>,
    by_uuid: RwLock<HashMap<Uuid, (VulnSource, String)>>,
}

impl VulnStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.by_key.read().unwrap().len()
    }

    pub fn upsert(&self, v: Vulnerability) -> Vulnerability {
        let key = (v.source, v.vuln_id.clone());
        let mut by_key = self.by_key.write().unwrap();
        let mut by_uuid = self.by_uuid.write().unwrap();
        if let Some(existing) = by_key.get(&key).cloned() {
            // Preserve the existing UUID for stable cross-references.
            let merged = Vulnerability {
                uuid: existing.uuid,
                ..v
            };
            by_key.insert(key.clone(), merged.clone());
            by_uuid.insert(merged.uuid, key);
            merged
        } else {
            by_key.insert(key.clone(), v.clone());
            by_uuid.insert(v.uuid, key);
            v
        }
    }

    pub fn get_by_id(&self, source: VulnSource, id: &str) -> Option<Vulnerability> {
        self.by_key
            .read()
            .unwrap()
            .get(&(source, id.to_string()))
            .cloned()
    }

    pub fn get_by_uuid(&self, uuid: Uuid) -> Result<Vulnerability> {
        let by_uuid = self.by_uuid.read().unwrap();
        let key = by_uuid
            .get(&uuid)
            .ok_or_else(|| Error::NotFound(format!("vuln uuid {}", uuid)))?
            .clone();
        drop(by_uuid);
        self.get_by_id(key.0, &key.1)
            .ok_or_else(|| Error::NotFound(format!("vuln uuid {}", uuid)))
    }

    pub fn list(&self) -> Vec<Vulnerability> {
        let mut v: Vec<_> = self.by_key.read().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.vuln_id.cmp(&b.vuln_id));
        v
    }

    pub fn list_by_source(&self, source: VulnSource) -> Vec<Vulnerability> {
        self.list().into_iter().filter(|v| v.source == source).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Severity;

    fn v(src: VulnSource, id: &str, sev: Severity) -> Vulnerability {
        let mut x = Vulnerability::new(id, src);
        x.severity = sev;
        x
    }

    #[test]
    fn upsert_then_get() {
        let s = VulnStore::new();
        let stored = s.upsert(v(VulnSource::Nvd, "CVE-2026-0001", Severity::High));
        let back = s.get_by_id(VulnSource::Nvd, "CVE-2026-0001").unwrap();
        assert_eq!(back.uuid, stored.uuid);
        assert_eq!(back.severity, Severity::High);
    }

    #[test]
    fn upsert_preserves_uuid_on_update() {
        let s = VulnStore::new();
        let first = s.upsert(v(VulnSource::Nvd, "CVE-1", Severity::Low));
        let second = s.upsert(v(VulnSource::Nvd, "CVE-1", Severity::Critical));
        assert_eq!(first.uuid, second.uuid);
        let back = s.get_by_id(VulnSource::Nvd, "CVE-1").unwrap();
        assert_eq!(back.severity, Severity::Critical);
    }

    #[test]
    fn different_sources_dont_collide() {
        let s = VulnStore::new();
        s.upsert(v(VulnSource::Nvd, "CVE-X", Severity::Low));
        s.upsert(v(VulnSource::Github, "CVE-X", Severity::High));
        assert_eq!(s.count(), 2);
        assert_eq!(s.list_by_source(VulnSource::Nvd).len(), 1);
        assert_eq!(s.list_by_source(VulnSource::Github).len(), 1);
    }

    #[test]
    fn get_by_uuid_works() {
        let s = VulnStore::new();
        let stored = s.upsert(v(VulnSource::Osv, "GHSA-1", Severity::Medium));
        assert_eq!(s.get_by_uuid(stored.uuid).unwrap().vuln_id, "GHSA-1");
    }

    #[test]
    fn list_sorted_by_id() {
        let s = VulnStore::new();
        s.upsert(v(VulnSource::Nvd, "CVE-2", Severity::Low));
        s.upsert(v(VulnSource::Nvd, "CVE-1", Severity::Low));
        let l = s.list();
        assert_eq!(l[0].vuln_id, "CVE-1");
        assert_eq!(l[1].vuln_id, "CVE-2");
    }
}
