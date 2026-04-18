use crate::models::{
    ChangeEntry, ChangelogStats, Release, ReleaseStatus,
};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct ChangelogStore {
    entries: RwLock<HashMap<Uuid, ChangeEntry>>,
    releases: RwLock<HashMap<Uuid, Release>>,
}

impl ChangelogStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Entries ───────────────────────────────────────────────────────────────

    pub fn create_entry(&self, entry: ChangeEntry) -> ChangeEntry {
        let mut entries = self.entries.write().unwrap();
        let e = entry.clone();
        entries.insert(entry.id, entry);
        e
    }

    pub fn get_entry(&self, id: &Uuid) -> Option<ChangeEntry> {
        self.entries.read().unwrap().get(id).cloned()
    }

    pub fn delete_entry(&self, id: &Uuid) -> Option<ChangeEntry> {
        self.entries.write().unwrap().remove(id)
    }

    pub fn list_entries(&self) -> Vec<ChangeEntry> {
        let mut entries: Vec<ChangeEntry> =
            self.entries.read().unwrap().values().cloned().collect();
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        entries
    }

    pub fn entries_for_release(&self, release_id: &Uuid) -> Vec<ChangeEntry> {
        let releases = self.releases.read().unwrap();
        if let Some(release) = releases.get(release_id) {
            let entry_ids = release.changes.clone();
            drop(releases);
            let entries = self.entries.read().unwrap();
            entry_ids
                .iter()
                .filter_map(|id| entries.get(id).cloned())
                .collect()
        } else {
            vec![]
        }
    }

    pub fn recent_entries(&self, limit: usize) -> Vec<ChangeEntry> {
        let mut entries = self.list_entries();
        entries.truncate(limit);
        entries
    }

    pub fn search_entries(&self, query: &str) -> Vec<ChangeEntry> {
        let lower = query.to_lowercase();
        self.entries
            .read()
            .unwrap()
            .values()
            .filter(|e| {
                e.title.to_lowercase().contains(&lower)
                    || e.description.to_lowercase().contains(&lower)
            })
            .cloned()
            .collect()
    }

    // ── Releases ──────────────────────────────────────────────────────────────

    pub fn create_release(&self, release: Release) -> Release {
        let mut releases = self.releases.write().unwrap();
        let r = release.clone();
        releases.insert(release.id, release);
        r
    }

    pub fn get_release(&self, id: &Uuid) -> Option<Release> {
        self.releases.read().unwrap().get(id).cloned()
    }

    pub fn update_release(&self, id: &Uuid, title: Option<String>, description: Option<String>, changes: Option<Vec<Uuid>>, pre_release: Option<bool>) -> Option<Release> {
        let mut releases = self.releases.write().unwrap();
        if let Some(r) = releases.get_mut(id) {
            if let Some(t) = title {
                r.title = t;
            }
            if let Some(d) = description {
                r.description = d;
            }
            if let Some(c) = changes {
                r.changes = c;
            }
            if let Some(p) = pre_release {
                r.pre_release = p;
            }
            return Some(r.clone());
        }
        None
    }

    pub fn delete_release(&self, id: &Uuid) -> Option<Release> {
        self.releases.write().unwrap().remove(id)
    }

    pub fn list_releases(&self) -> Vec<Release> {
        let mut releases: Vec<Release> =
            self.releases.read().unwrap().values().cloned().collect();
        releases.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        releases
    }

    pub fn publish_release(&self, id: &Uuid) -> Option<Release> {
        let mut releases = self.releases.write().unwrap();
        if let Some(r) = releases.get_mut(id) {
            r.status = ReleaseStatus::Published;
            r.release_date = Some(Utc::now());
            return Some(r.clone());
        }
        None
    }

    // ── Stats ─────────────────────────────────────────────────────────────────

    pub fn compute_stats(&self) -> ChangelogStats {
        let entries = self.entries.read().unwrap();
        let releases = self.releases.read().unwrap();

        let now = Utc::now();
        let seven_days_ago = now - chrono::Duration::days(7);

        let mut by_type: HashMap<String, u64> = HashMap::new();
        let mut by_service: HashMap<String, u64> = HashMap::new();
        let mut recent_breaking = 0u64;

        for e in entries.values() {
            let type_key = format!("{:?}", e.change_type);
            *by_type.entry(type_key).or_default() += 1;
            *by_service.entry(e.service.clone()).or_default() += 1;
            if e.breaking && e.created_at >= seven_days_ago {
                recent_breaking += 1;
            }
        }

        ChangelogStats {
            total_entries: entries.len() as u64,
            total_releases: releases.len() as u64,
            by_type,
            by_service,
            recent_breaking,
        }
    }
}
