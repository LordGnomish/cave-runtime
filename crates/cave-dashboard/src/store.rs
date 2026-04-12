//! In-memory CRUD store for CAVE Dashboard.

use std::collections::{HashMap, HashSet};

use crate::models::{
    AlertNotificationChannel, Annotation, Dashboard, DataSource, Folder, Playlist, Snapshot,
};

/// Central in-memory store.
/// Wrapped in `Arc<Mutex<DashboardStore>>` by `DashboardState`.
#[derive(Default)]
pub struct DashboardStore {
    // core entities
    pub dashboards: HashMap<String, Dashboard>,       // key = uid
    pub folders: HashMap<String, Folder>,             // key = uid
    pub datasources: HashMap<u32, DataSource>,        // key = id
    pub alert_channels: HashMap<u32, AlertNotificationChannel>, // key = id
    pub snapshots: HashMap<String, Snapshot>,         // key = key
    pub playlists: HashMap<String, Playlist>,         // key = id
    pub annotations: HashMap<u64, Annotation>,        // key = id
    pub starred: HashSet<String>,                     // dashboard uids

    // auto-increment counters
    pub next_dashboard_id: u32,
    pub next_folder_id: u32,
    pub next_datasource_id: u32,
    pub next_channel_id: u32,
    pub next_snapshot_id: u32,
    pub next_annotation_id: u64,
}

impl DashboardStore {
    // ─── Dashboard ──────────────────────────────────────────────────────────

    pub fn upsert_dashboard(&mut self, mut d: Dashboard) -> Dashboard {
        if let Some(existing) = self.dashboards.get(&d.uid) {
            // Update: inherit stable fields, increment version.
            d.version = existing.version + 1;
            d.created_at = existing.created_at;
            if d.id == 0 {
                d.id = existing.id;
            }
        } else {
            // Create: assign a new numeric id.
            self.next_dashboard_id += 1;
            d.id = self.next_dashboard_id;
        }
        d.is_starred = self.starred.contains(&d.uid);
        let uid = d.uid.clone();
        self.dashboards.insert(uid, d.clone());
        d
    }

    pub fn get_dashboard(&self, uid: &str) -> Option<&Dashboard> {
        self.dashboards.get(uid)
    }

    pub fn delete_dashboard(&mut self, uid: &str) -> bool {
        self.starred.remove(uid);
        self.dashboards.remove(uid).is_some()
    }

    pub fn list_dashboards(&self) -> Vec<&Dashboard> {
        self.dashboards.values().collect()
    }

    pub fn search_dashboards(
        &self,
        query: Option<&str>,
        tag: Option<&str>,
        folder_uid: Option<&str>,
        starred: Option<bool>,
    ) -> Vec<&Dashboard> {
        self.dashboards
            .values()
            .filter(|d| {
                if let Some(q) = query {
                    if !q.is_empty() && !d.title.to_lowercase().contains(&q.to_lowercase()) {
                        return false;
                    }
                }
                if let Some(t) = tag {
                    if !d.tags.iter().any(|tag| tag == t) {
                        return false;
                    }
                }
                if let Some(fuid) = folder_uid {
                    match &d.folder_uid {
                        Some(f) if f == fuid => {}
                        _ => return false,
                    }
                }
                if let Some(s) = starred {
                    if d.is_starred != s {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    pub fn star_dashboard(&mut self, uid: &str) -> bool {
        if self.dashboards.contains_key(uid) {
            self.starred.insert(uid.to_string());
            if let Some(d) = self.dashboards.get_mut(uid) {
                d.is_starred = true;
            }
            true
        } else {
            false
        }
    }

    pub fn unstar_dashboard(&mut self, uid: &str) -> bool {
        self.starred.remove(uid);
        if let Some(d) = self.dashboards.get_mut(uid) {
            d.is_starred = false;
        }
        true
    }

    // ─── Folder ─────────────────────────────────────────────────────────────

    pub fn create_folder(&mut self, mut folder: Folder) -> Folder {
        self.next_folder_id += 1;
        folder.id = self.next_folder_id;
        let uid = folder.uid.clone();
        self.folders.insert(uid, folder.clone());
        folder
    }

    pub fn get_folder(&self, uid: &str) -> Option<&Folder> {
        self.folders.get(uid)
    }

    pub fn update_folder(&mut self, uid: &str, title: String) -> Option<Folder> {
        let folder = self.folders.get_mut(uid)?;
        folder.title = title;
        folder.url = format!("/dashboards/f/{}/{}", uid, folder.title.to_lowercase().replace(' ', "-"));
        folder.updated_at = chrono::Utc::now();
        Some(folder.clone())
    }

    pub fn delete_folder(&mut self, uid: &str) -> bool {
        self.folders.remove(uid).is_some()
    }

    pub fn list_folders(&self) -> Vec<&Folder> {
        self.folders.values().collect()
    }

    // ─── DataSource ─────────────────────────────────────────────────────────

    pub fn create_datasource(&mut self, mut ds: DataSource) -> DataSource {
        self.next_datasource_id += 1;
        ds.id = self.next_datasource_id;
        let id = ds.id;
        self.datasources.insert(id, ds.clone());
        ds
    }

    pub fn get_datasource(&self, id: u32) -> Option<&DataSource> {
        self.datasources.get(&id)
    }

    pub fn get_datasource_by_uid(&self, uid: &str) -> Option<&DataSource> {
        self.datasources.values().find(|ds| ds.uid == uid)
    }

    pub fn update_datasource(&mut self, id: u32, mut ds: DataSource) -> Option<DataSource> {
        let existing = self.datasources.get(&id)?;
        ds.id = existing.id;
        ds.created_at = existing.created_at;
        ds.updated_at = chrono::Utc::now();
        self.datasources.insert(id, ds.clone());
        Some(ds)
    }

    pub fn delete_datasource(&mut self, id: u32) -> bool {
        self.datasources.remove(&id).is_some()
    }

    pub fn list_datasources(&self) -> Vec<&DataSource> {
        self.datasources.values().collect()
    }

    // ─── Alert Channels ─────────────────────────────────────────────────────

    pub fn create_channel(&mut self, mut ch: AlertNotificationChannel) -> AlertNotificationChannel {
        self.next_channel_id += 1;
        ch.id = self.next_channel_id;
        let id = ch.id;
        self.alert_channels.insert(id, ch.clone());
        ch
    }

    pub fn get_channel(&self, id: u32) -> Option<&AlertNotificationChannel> {
        self.alert_channels.get(&id)
    }

    pub fn update_channel(
        &mut self,
        id: u32,
        mut ch: AlertNotificationChannel,
    ) -> Option<AlertNotificationChannel> {
        let existing = self.alert_channels.get(&id)?;
        ch.id = existing.id;
        ch.created_at = existing.created_at;
        ch.updated_at = chrono::Utc::now();
        self.alert_channels.insert(id, ch.clone());
        Some(ch)
    }

    pub fn delete_channel(&mut self, id: u32) -> bool {
        self.alert_channels.remove(&id).is_some()
    }

    pub fn list_channels(&self) -> Vec<&AlertNotificationChannel> {
        self.alert_channels.values().collect()
    }

    // ─── Snapshots ──────────────────────────────────────────────────────────

    pub fn create_snapshot(&mut self, mut snap: Snapshot) -> Snapshot {
        self.next_snapshot_id += 1;
        snap.id = self.next_snapshot_id;
        let key = snap.key.clone();
        self.snapshots.insert(key, snap.clone());
        snap
    }

    pub fn get_snapshot(&self, key: &str) -> Option<&Snapshot> {
        self.snapshots.get(key)
    }

    pub fn delete_snapshot(&mut self, key: &str) -> bool {
        self.snapshots.remove(key).is_some()
    }

    // ─── Playlists ──────────────────────────────────────────────────────────

    pub fn create_playlist(&mut self, playlist: Playlist) -> Playlist {
        let id = playlist.id.clone();
        self.playlists.insert(id, playlist.clone());
        playlist
    }

    pub fn get_playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.get(id)
    }

    pub fn update_playlist(&mut self, id: &str, mut playlist: Playlist) -> Option<Playlist> {
        let existing = self.playlists.get(id)?;
        playlist.id = existing.id.clone();
        playlist.created_at = existing.created_at;
        playlist.updated_at = chrono::Utc::now();
        self.playlists.insert(id.to_string(), playlist.clone());
        Some(playlist)
    }

    pub fn delete_playlist(&mut self, id: &str) -> bool {
        self.playlists.remove(id).is_some()
    }

    pub fn list_playlists(&self) -> Vec<&Playlist> {
        self.playlists.values().collect()
    }

    // ─── Annotations ────────────────────────────────────────────────────────

    pub fn create_annotation(&mut self, mut ann: Annotation) -> Annotation {
        self.next_annotation_id += 1;
        ann.id = self.next_annotation_id;
        let id = ann.id;
        self.annotations.insert(id, ann.clone());
        ann
    }

    pub fn list_annotations(&self, dashboard_uid: Option<&str>) -> Vec<&Annotation> {
        self.annotations
            .values()
            .filter(|a| {
                if let Some(uid) = dashboard_uid {
                    a.dashboard_uid == uid
                } else {
                    true
                }
            })
            .collect()
    }

    pub fn delete_annotation(&mut self, id: u64) -> bool {
        self.annotations.remove(&id).is_some()
    }

    // ─── Alert rules (from panels) ──────────────────────────────────────────

    pub fn list_alert_rules(&self) -> Vec<crate::models::AlertRule> {
        let mut rules = vec![];
        for d in self.dashboards.values() {
            for p in &d.panels {
                if let Some(rule) = &p.alert {
                    rules.push(rule.clone());
                }
            }
        }
        rules
    }
}
