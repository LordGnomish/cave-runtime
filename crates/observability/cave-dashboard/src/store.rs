// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory store — CRUD for all dashboard entities with versioning.

use crate::models::*;
use chrono::Utc;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use uuid::Uuid;

// ─── Inner state ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct Inner {
    // Sequences
    next_dashboard_id: i64,
    next_folder_id: i64,
    next_ds_id: i64,
    next_user_id: i64,
    next_org_id: i64,
    next_team_id: i64,
    next_annotation_id: i64,
    next_snapshot_id: i64,
    next_playlist_id: i64,
    next_permission_id: i64,
    next_apikey_id: i64,
    next_sa_id: i64,
    next_alert_rule_id: i64,
    next_notification_channel_id: i64,
    next_version_id: i64,

    // Core entities
    dashboards: HashMap<String, Dashboard>, // uid → dashboard
    dashboard_by_id: HashMap<i64, String>,  // id → uid
    dashboard_versions: HashMap<i64, Vec<DashboardVersion>>, // dashboard_id → versions
    starred: HashMap<(i64, String), bool>,  // (user_id, dashboard_uid)

    folders: HashMap<String, Folder>,   // uid → folder
    folder_by_id: HashMap<i64, String>, // id → uid

    datasources: HashMap<String, DataSource>, // uid → datasource
    datasource_by_id: HashMap<i64, String>,   // id → uid
    datasource_by_name: HashMap<(i64, String), String>, // (org_id, name) → uid

    orgs: HashMap<i64, Org>,
    users: HashMap<i64, User>,
    user_by_login: HashMap<String, i64>,
    teams: HashMap<i64, Team>,
    team_members: HashMap<i64, Vec<TeamMember>>, // team_id → members
    api_keys: HashMap<i64, ApiKey>,
    api_key_by_hash: HashMap<String, i64>,
    service_accounts: HashMap<i64, ServiceAccount>,

    annotations: HashMap<i64, Annotation>,
    snapshots: HashMap<String, Snapshot>, // key → snapshot
    snapshot_by_delete_key: HashMap<String, String>, // delete_key → key
    playlists: HashMap<i64, Playlist>,

    // Unified alerting
    alert_rules: HashMap<String, AlertRule>, // uid → rule
    rule_groups: HashMap<(String, String), Vec<String>>, // (folder_uid, group) → rule uids
    contact_points: HashMap<String, ContactPoint>, // uid → contact point
    notification_policy: Option<NotificationPolicy>,
    silences: HashMap<String, Silence>,
    mute_timings: HashMap<String, MuteTiming>,

    // Legacy alerting
    notification_channels: HashMap<i64, AlertNotificationChannel>,

    // Dashboard permissions
    permissions: HashMap<i64, DashboardPermission>,
}

impl Inner {
    fn next_dashboard_id(&mut self) -> i64 {
        self.next_dashboard_id += 1;
        self.next_dashboard_id
    }
    fn next_folder_id(&mut self) -> i64 {
        self.next_folder_id += 1;
        self.next_folder_id
    }
    fn next_ds_id(&mut self) -> i64 {
        self.next_ds_id += 1;
        self.next_ds_id
    }
    fn next_user_id(&mut self) -> i64 {
        self.next_user_id += 1;
        self.next_user_id
    }
    fn next_org_id(&mut self) -> i64 {
        self.next_org_id += 1;
        self.next_org_id
    }
    fn next_team_id(&mut self) -> i64 {
        self.next_team_id += 1;
        self.next_team_id
    }
    fn next_annotation_id(&mut self) -> i64 {
        self.next_annotation_id += 1;
        self.next_annotation_id
    }
    fn next_snapshot_id(&mut self) -> i64 {
        self.next_snapshot_id += 1;
        self.next_snapshot_id
    }
    fn next_playlist_id(&mut self) -> i64 {
        self.next_playlist_id += 1;
        self.next_playlist_id
    }
    fn next_permission_id(&mut self) -> i64 {
        self.next_permission_id += 1;
        self.next_permission_id
    }
    fn next_apikey_id(&mut self) -> i64 {
        self.next_apikey_id += 1;
        self.next_apikey_id
    }
    fn next_sa_id(&mut self) -> i64 {
        self.next_sa_id += 1;
        self.next_sa_id
    }
    fn next_alert_rule_id(&mut self) -> i64 {
        self.next_alert_rule_id += 1;
        self.next_alert_rule_id
    }
    fn next_notification_channel_id(&mut self) -> i64 {
        self.next_notification_channel_id += 1;
        self.next_notification_channel_id
    }
    fn next_version_id(&mut self) -> i64 {
        self.next_version_id += 1;
        self.next_version_id
    }
}

// ─── Store ────────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct DashboardStore {
    inner: Arc<RwLock<Inner>>,
}

/// Result type for store operations
pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("lock poisoned")]
    Lock,
}

impl DashboardStore {
    pub fn new() -> Self {
        let store = Self::default();
        // Seed default org (id=1) and admin user
        {
            let mut inner = store.inner.write().unwrap();
            inner.next_org_id = 1;
            let org = Org {
                id: 1,
                name: "Main Org.".into(),
                address: OrgAddress::default(),
                created: Utc::now(),
                updated: Utc::now(),
            };
            inner.orgs.insert(1, org);
        }
        store
    }

    // ── Dashboard ─────────────────────────────────────────────────────────────

    pub fn upsert_dashboard(
        &self,
        org_id: i64,
        mut dashboard: Dashboard,
        folder_uid: Option<&str>,
        message: &str,
        user: &str,
        overwrite: bool,
    ) -> StoreResult<Dashboard> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;

        let now = Utc::now();

        // If UID exists — update; else create
        let existing_snapshot = inner
            .dashboards
            .get(&dashboard.uid)
            .map(|e| (e.version, e.id, e.revision, e.created));
        if let Some((existing_version, existing_id, existing_revision, existing_created)) =
            existing_snapshot
        {
            if !overwrite && existing_version != dashboard.version {
                return Err(StoreError::Conflict(format!(
                    "version mismatch: expected {existing_version}, got {}",
                    dashboard.version
                )));
            }
            let new_version = existing_version + 1;
            let db_id = existing_id.unwrap_or(0);

            // Save version history
            let ver_id = inner.next_version_id();
            let dv = DashboardVersion {
                id: ver_id,
                dashboard_id: db_id,
                parent_version: existing_version,
                restored_from: 0,
                version: new_version,
                created: now,
                created_by: user.to_string(),
                message: message.to_string(),
                data: None,
            };
            inner.dashboard_versions.entry(db_id).or_default().push(dv);

            dashboard.id = existing_id;
            dashboard.version = new_version;
            dashboard.revision = existing_revision + 1;
            dashboard.org_id = org_id;
            dashboard.created = existing_created;
            dashboard.updated = now;
            dashboard.updated_by = user.to_string();
            dashboard.slug = Dashboard::slug_from_title(&dashboard.title);
            dashboard.url = format!("/d/{}/{}", dashboard.uid, dashboard.slug);
        } else {
            // New dashboard
            let id = inner.next_dashboard_id();
            dashboard.id = Some(id);
            dashboard.org_id = org_id;
            dashboard.version = 1;
            dashboard.revision = 1;
            dashboard.created = now;
            dashboard.updated = now;
            dashboard.created_by = user.to_string();
            dashboard.updated_by = user.to_string();
            dashboard.slug = Dashboard::slug_from_title(&dashboard.title);
            dashboard.url = format!("/d/{}/{}", dashboard.uid, dashboard.slug);
            inner.dashboard_by_id.insert(id, dashboard.uid.clone());

            // First version
            let ver_id = inner.next_version_id();
            let dv = DashboardVersion {
                id: ver_id,
                dashboard_id: id,
                parent_version: 0,
                restored_from: 0,
                version: 1,
                created: now,
                created_by: user.to_string(),
                message: message.to_string(),
                data: None,
            };
            inner.dashboard_versions.entry(id).or_default().push(dv);
        }

        // Apply folder
        if let Some(fuid) = folder_uid {
            if let Some(folder) = inner.folders.get(fuid) {
                dashboard.folder_uid = Some(folder.uid.clone());
                dashboard.folder_id = Some(folder.id);
                dashboard.folder_title = Some(folder.title.clone());
                dashboard.folder_url = Some(folder.url.clone());
            }
        }

        inner
            .dashboards
            .insert(dashboard.uid.clone(), dashboard.clone());
        Ok(dashboard)
    }

    pub fn get_dashboard_by_uid(&self, uid: &str) -> StoreResult<Dashboard> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .dashboards
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("dashboard {uid}")))
    }

    pub fn get_dashboard_by_id(&self, id: i64) -> StoreResult<Dashboard> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let uid = inner
            .dashboard_by_id
            .get(&id)
            .ok_or_else(|| StoreError::NotFound(format!("dashboard id={id}")))?;
        inner
            .dashboards
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("dashboard {uid}")))
    }

    pub fn delete_dashboard(&self, uid: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let d = inner
            .dashboards
            .remove(uid)
            .ok_or_else(|| StoreError::NotFound(format!("dashboard {uid}")))?;
        if let Some(id) = d.id {
            inner.dashboard_by_id.remove(&id);
            inner.dashboard_versions.remove(&id);
        }
        Ok(())
    }

    pub fn list_dashboards(&self, org_id: i64) -> StoreResult<Vec<Dashboard>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .dashboards
            .values()
            .filter(|d| d.org_id == org_id)
            .cloned()
            .collect())
    }

    pub fn search_dashboards(&self, q: &SearchQuery) -> StoreResult<Vec<SearchResult>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let org_id = q.org_id.unwrap_or(1);
        let mut results: Vec<SearchResult> = inner
            .dashboards
            .values()
            .filter(|d| d.org_id == org_id)
            .filter(|d| {
                if let Some(ref qstr) = q.query {
                    if !qstr.is_empty() && !d.title.to_lowercase().contains(&qstr.to_lowercase()) {
                        return false;
                    }
                }
                if !q.tag.is_empty() {
                    if !q.tag.iter().all(|t| d.tags.contains(t)) {
                        return false;
                    }
                }
                if let Some(starred) = q.starred {
                    // In a real impl we'd check by user; simplified here
                    if starred != d.is_starred {
                        return false;
                    }
                }
                if !q.folder_uids.is_empty() {
                    let fuid = d.folder_uid.as_deref().unwrap_or("");
                    if !q.folder_uids.iter().any(|f| f == fuid) {
                        return false;
                    }
                }
                true
            })
            .map(|d| SearchResult {
                id: d.id.unwrap_or(0),
                uid: d.uid.clone(),
                title: d.title.clone(),
                uri: format!("db/{}", d.slug),
                url: d.url.clone(),
                slug: d.slug.clone(),
                r#type: "dash-db".into(),
                tags: d.tags.clone(),
                is_starred: d.is_starred,
                folder_id: d.folder_id,
                folder_uid: d.folder_uid.clone(),
                folder_title: d.folder_title.clone(),
                folder_url: d.folder_url.clone(),
                sort_meta: 0,
                sort_meta_name: String::new(),
            })
            .collect();

        // Also include folder results if no type filter
        if q.result_type.as_deref() != Some("dash-db") {
            for folder in inner.folders.values().filter(|f| f.org_id == org_id) {
                if let Some(ref qstr) = q.query {
                    if !qstr.is_empty()
                        && !folder.title.to_lowercase().contains(&qstr.to_lowercase())
                    {
                        continue;
                    }
                }
                results.push(SearchResult {
                    id: folder.id,
                    uid: folder.uid.clone(),
                    title: folder.title.clone(),
                    uri: format!(
                        "f/{}/{}",
                        folder.uid,
                        Dashboard::slug_from_title(&folder.title)
                    ),
                    url: folder.url.clone(),
                    slug: Dashboard::slug_from_title(&folder.title),
                    r#type: "dash-folder".into(),
                    tags: vec![],
                    is_starred: false,
                    folder_id: None,
                    folder_uid: None,
                    folder_title: None,
                    folder_url: None,
                    sort_meta: 0,
                    sort_meta_name: String::new(),
                });
            }
        }

        let limit = q.limit.unwrap_or(1000) as usize;
        results.truncate(limit);
        Ok(results)
    }

    pub fn star_dashboard(&self, user_id: i64, uid: &str, star: bool) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let d = inner
            .dashboards
            .get_mut(uid)
            .ok_or_else(|| StoreError::NotFound(uid.to_string()))?;
        d.is_starred = star;
        if star {
            inner.starred.insert((user_id, uid.to_string()), true);
        } else {
            inner.starred.remove(&(user_id, uid.to_string()));
        }
        Ok(())
    }

    pub fn get_dashboard_versions(&self, dashboard_id: i64) -> StoreResult<Vec<DashboardVersion>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .dashboard_versions
            .get(&dashboard_id)
            .cloned()
            .unwrap_or_default())
    }

    pub fn restore_dashboard_version(
        &self,
        dashboard_id: i64,
        version: i64,
        user: &str,
    ) -> StoreResult<Dashboard> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let uid = inner
            .dashboard_by_id
            .get(&dashboard_id)
            .ok_or_else(|| StoreError::NotFound(format!("dashboard id={dashboard_id}")))?
            .clone();
        let _versions = inner.dashboard_versions.get(&dashboard_id).ok_or_else(|| {
            StoreError::NotFound(format!("versions for dashboard {dashboard_id}"))
        })?;
        // In a full impl, we'd restore the dashboard JSON from that version.
        // Here we just bump the version number.
        let d = inner
            .dashboards
            .get_mut(&uid)
            .ok_or_else(|| StoreError::NotFound(uid.clone()))?;
        let new_version = d.version + 1;
        d.version = new_version;
        d.updated = Utc::now();
        d.updated_by = user.to_string();
        let restored = d.clone();

        let ver_id = inner.next_version_id();
        let dv = DashboardVersion {
            id: ver_id,
            dashboard_id,
            parent_version: new_version - 1,
            restored_from: version,
            version: new_version,
            created: Utc::now(),
            created_by: user.to_string(),
            message: format!("Restored from version {version}"),
            data: None,
        };
        inner
            .dashboard_versions
            .entry(dashboard_id)
            .or_default()
            .push(dv);
        Ok(restored)
    }

    // ── Permissions ───────────────────────────────────────────────────────────

    pub fn set_dashboard_permissions(
        &self,
        dashboard_id: i64,
        perms: Vec<DashboardPermission>,
    ) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        // Remove old permissions for this dashboard
        inner
            .permissions
            .retain(|_, v| v.dashboard_id != dashboard_id);
        for mut p in perms {
            let id = inner.next_permission_id();
            p.id = id;
            inner.permissions.insert(id, p);
        }
        Ok(())
    }

    pub fn get_dashboard_permissions(
        &self,
        dashboard_id: i64,
    ) -> StoreResult<Vec<DashboardPermission>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .permissions
            .values()
            .filter(|p| p.dashboard_id == dashboard_id)
            .cloned()
            .collect())
    }

    // ── Folder ────────────────────────────────────────────────────────────────

    pub fn create_folder(
        &self,
        org_id: i64,
        uid: Option<&str>,
        title: &str,
        parent_uid: Option<&str>,
    ) -> StoreResult<Folder> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_folder_id();
        let uid = uid
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', "")[..9].to_string());

        if inner.folders.contains_key(&uid) {
            return Err(StoreError::Conflict(format!(
                "folder uid {uid} already exists"
            )));
        }

        let mut folder = Folder::new(id, org_id, &uid, title);
        folder.parent_uid = parent_uid.map(str::to_string);
        inner.folders.insert(uid.clone(), folder.clone());
        inner.folder_by_id.insert(id, uid);
        Ok(folder)
    }

    pub fn get_folder_by_uid(&self, uid: &str) -> StoreResult<Folder> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .folders
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("folder {uid}")))
    }

    pub fn get_folder_by_id(&self, id: i64) -> StoreResult<Folder> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let uid = inner
            .folder_by_id
            .get(&id)
            .ok_or_else(|| StoreError::NotFound(format!("folder id={id}")))?;
        inner
            .folders
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("folder {uid}")))
    }

    pub fn update_folder(&self, uid: &str, title: &str) -> StoreResult<Folder> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let folder = inner
            .folders
            .get_mut(uid)
            .ok_or_else(|| StoreError::NotFound(format!("folder {uid}")))?;
        folder.title = title.to_string();
        folder.version += 1;
        folder.updated = Utc::now();
        Ok(folder.clone())
    }

    /// Reparent a folder (used by the nested-folder move operation).
    /// `None` moves it to the root.
    pub fn set_folder_parent(&self, uid: &str, parent_uid: Option<&str>) -> StoreResult<Folder> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let folder = inner
            .folders
            .get_mut(uid)
            .ok_or_else(|| StoreError::NotFound(format!("folder {uid}")))?;
        folder.parent_uid = parent_uid.map(str::to_string);
        folder.version += 1;
        folder.updated = Utc::now();
        Ok(folder.clone())
    }

    pub fn delete_folder(&self, uid: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let f = inner
            .folders
            .remove(uid)
            .ok_or_else(|| StoreError::NotFound(format!("folder {uid}")))?;
        inner.folder_by_id.remove(&f.id);
        Ok(())
    }

    pub fn list_folders(&self, org_id: i64) -> StoreResult<Vec<Folder>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .folders
            .values()
            .filter(|f| f.org_id == org_id)
            .cloned()
            .collect())
    }

    // ── DataSource ────────────────────────────────────────────────────────────

    pub fn create_datasource(
        &self,
        req: crate::models::CreateDataSourceRequest,
        org_id: i64,
    ) -> StoreResult<DataSource> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_ds_id();
        let uid = req
            .uid
            .unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', "")[..9].to_string());

        if inner
            .datasource_by_name
            .contains_key(&(org_id, req.name.clone()))
        {
            return Err(StoreError::Conflict(format!(
                "datasource name '{}' already exists",
                req.name
            )));
        }

        let mut ds = DataSource::new(id, org_id, &uid, &req.name, req.ds_type, &req.url);
        ds.access = req.access;
        ds.is_default = req.is_default;
        ds.json_data = req.json_data;
        ds.basic_auth = req.basic_auth;
        ds.basic_auth_user = req.basic_auth_user;
        ds.user = req.user;
        ds.database = req.database;

        if req.is_default {
            // Clear existing default for same org
            for existing in inner
                .datasources
                .values_mut()
                .filter(|d| d.org_id == org_id)
            {
                existing.is_default = false;
            }
        }

        inner.datasources.insert(uid.clone(), ds.clone());
        inner.datasource_by_id.insert(id, uid.clone());
        inner.datasource_by_name.insert((org_id, req.name), uid);
        Ok(ds)
    }

    pub fn get_datasource_by_uid(&self, uid: &str) -> StoreResult<DataSource> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .datasources
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("datasource {uid}")))
    }

    pub fn get_datasource_by_id(&self, id: i64) -> StoreResult<DataSource> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let uid = inner
            .datasource_by_id
            .get(&id)
            .ok_or_else(|| StoreError::NotFound(format!("datasource id={id}")))?;
        inner
            .datasources
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(uid.as_str().to_string()))
    }

    pub fn update_datasource(
        &self,
        uid: &str,
        req: CreateDataSourceRequest,
    ) -> StoreResult<DataSource> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let ds = inner
            .datasources
            .get_mut(uid)
            .ok_or_else(|| StoreError::NotFound(format!("datasource {uid}")))?;
        ds.name = req.name;
        ds.ds_type = req.ds_type;
        ds.url = req.url;
        ds.access = req.access;
        ds.is_default = req.is_default;
        ds.json_data = req.json_data;
        ds.basic_auth = req.basic_auth;
        ds.basic_auth_user = req.basic_auth_user;
        ds.user = req.user;
        ds.database = req.database;
        ds.version += 1;
        ds.updated = Utc::now();
        Ok(ds.clone())
    }

    pub fn delete_datasource(&self, uid: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let ds = inner
            .datasources
            .remove(uid)
            .ok_or_else(|| StoreError::NotFound(format!("datasource {uid}")))?;
        inner.datasource_by_id.remove(&ds.id);
        inner.datasource_by_name.remove(&(ds.org_id, ds.name));
        Ok(())
    }

    pub fn list_datasources(&self, org_id: i64) -> StoreResult<Vec<DataSource>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .datasources
            .values()
            .filter(|d| d.org_id == org_id)
            .cloned()
            .collect())
    }

    // ── Annotation ────────────────────────────────────────────────────────────

    pub fn create_annotation(
        &self,
        req: CreateAnnotationRequest,
        user_id: i64,
        org_id: i64,
    ) -> StoreResult<Annotation> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_annotation_id();
        let now = Utc::now();
        let annotation = Annotation {
            id,
            alert_id: 0,
            alert_name: String::new(),
            dashboard_id: 0,
            dashboard_uid: req.dashboard_uid.unwrap_or_default(),
            panel_id: req.panel_id.unwrap_or(0),
            user_id,
            user_name: String::new(),
            new_state: String::new(),
            prev_state: String::new(),
            time: req.time,
            time_end: req.time_end.unwrap_or(req.time),
            text: req.text,
            tags: req.tags,
            data: None,
            login: String::new(),
            email: String::new(),
            avatar_url: String::new(),
            created: now,
            updated: now,
        };
        inner.annotations.insert(id, annotation.clone());
        Ok(annotation)
    }

    pub fn list_annotations(
        &self,
        dashboard_uid: Option<&str>,
        org_id: i64,
    ) -> StoreResult<Vec<Annotation>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let anns: Vec<Annotation> = inner
            .annotations
            .values()
            .filter(|a| {
                if let Some(uid) = dashboard_uid {
                    a.dashboard_uid == uid
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        Ok(anns)
    }

    pub fn delete_annotation(&self, id: i64) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner
            .annotations
            .remove(&id)
            .ok_or_else(|| StoreError::NotFound(format!("annotation {id}")))?;
        Ok(())
    }

    // ── Snapshot ──────────────────────────────────────────────────────────────

    pub fn create_snapshot(
        &self,
        req: CreateSnapshotRequest,
        org_id: i64,
        user_id: i64,
    ) -> StoreResult<Snapshot> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_snapshot_id();
        let key = req
            .key
            .unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', ""));
        let delete_key = req
            .delete_key
            .unwrap_or_else(|| Uuid::new_v4().to_string().replace('-', ""));
        let now = Utc::now();
        let expires_secs = req.expires.unwrap_or(0);
        let expires = if expires_secs > 0 {
            now + chrono::Duration::seconds(expires_secs)
        } else {
            now + chrono::Duration::days(365 * 100) // "never"
        };
        let name = req.name.unwrap_or_else(|| {
            req.dashboard
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("Snapshot")
                .to_string()
        });
        let snapshot = Snapshot {
            id,
            uid: Uuid::new_v4().to_string().replace('-', "")[..9].to_string(),
            name,
            org_id,
            key: key.clone(),
            delete_key: delete_key.clone(),
            url: format!("/dashboard/snapshot/{key}"),
            external: req.external,
            external_url: String::new(),
            expires,
            created: now,
            updated: now,
            dashboard: req.dashboard,
        };
        inner.snapshots.insert(key.clone(), snapshot.clone());
        inner.snapshot_by_delete_key.insert(delete_key, key);
        Ok(snapshot)
    }

    pub fn get_snapshot(&self, key: &str) -> StoreResult<Snapshot> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let snap = inner
            .snapshots
            .get(key)
            .ok_or_else(|| StoreError::NotFound(format!("snapshot {key}")))?;
        if snap.expires < Utc::now() {
            return Err(StoreError::NotFound("snapshot expired".into()));
        }
        Ok(snap.clone())
    }

    pub fn delete_snapshot_by_delete_key(&self, delete_key: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let key = inner
            .snapshot_by_delete_key
            .remove(delete_key)
            .ok_or_else(|| StoreError::NotFound(format!("snapshot delete_key {delete_key}")))?;
        inner.snapshots.remove(&key);
        Ok(())
    }

    // ── Playlist ──────────────────────────────────────────────────────────────

    pub fn create_playlist(
        &self,
        org_id: i64,
        name: &str,
        interval: &str,
        items: Vec<PlaylistItem>,
    ) -> StoreResult<Playlist> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_playlist_id();
        let now = Utc::now();
        let p = Playlist {
            id,
            uid: Uuid::new_v4().to_string().replace('-', "")[..9].to_string(),
            name: name.to_string(),
            interval: interval.to_string(),
            org_id,
            items,
            created: now,
            updated: now,
        };
        inner.playlists.insert(id, p.clone());
        Ok(p)
    }

    pub fn get_playlist(&self, id: i64) -> StoreResult<Playlist> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .playlists
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("playlist {id}")))
    }

    pub fn update_playlist(
        &self,
        id: i64,
        name: &str,
        interval: &str,
        items: Vec<PlaylistItem>,
    ) -> StoreResult<Playlist> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let p = inner
            .playlists
            .get_mut(&id)
            .ok_or_else(|| StoreError::NotFound(format!("playlist {id}")))?;
        p.name = name.to_string();
        p.interval = interval.to_string();
        p.items = items;
        p.updated = Utc::now();
        Ok(p.clone())
    }

    pub fn delete_playlist(&self, id: i64) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner
            .playlists
            .remove(&id)
            .ok_or_else(|| StoreError::NotFound(format!("playlist {id}")))?;
        Ok(())
    }

    pub fn list_playlists(&self, org_id: i64) -> StoreResult<Vec<Playlist>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .playlists
            .values()
            .filter(|p| p.org_id == org_id)
            .cloned()
            .collect())
    }

    // ── Alert Rules (Unified Alerting) ────────────────────────────────────────

    pub fn upsert_alert_rule(&self, mut rule: AlertRule) -> StoreResult<AlertRule> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        if inner.alert_rules.contains_key(&rule.uid) {
            let existing = &inner.alert_rules[&rule.uid];
            rule.id = existing.id;
            rule.created = existing.created;
            rule.updated = Utc::now();
        } else {
            rule.id = inner.next_alert_rule_id();
            rule.created = Utc::now();
            rule.updated = Utc::now();
        }
        inner
            .rule_groups
            .entry((rule.folder_uid.clone(), rule.rule_group.clone()))
            .or_default()
            .push(rule.uid.clone());
        inner.alert_rules.insert(rule.uid.clone(), rule.clone());
        Ok(rule)
    }

    pub fn get_alert_rule(&self, uid: &str) -> StoreResult<AlertRule> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .alert_rules
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("alert rule {uid}")))
    }

    pub fn delete_alert_rule(&self, uid: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let rule = inner
            .alert_rules
            .remove(uid)
            .ok_or_else(|| StoreError::NotFound(uid.to_string()))?;
        if let Some(group) = inner
            .rule_groups
            .get_mut(&(rule.folder_uid.clone(), rule.rule_group))
        {
            group.retain(|u| u != uid);
        }
        Ok(())
    }

    pub fn list_alert_rules(&self, org_id: i64) -> StoreResult<Vec<AlertRule>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .alert_rules
            .values()
            .filter(|r| r.org_id == org_id)
            .cloned()
            .collect())
    }

    pub fn list_rule_groups(&self, org_id: i64) -> StoreResult<Vec<RuleGroup>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let mut groups: HashMap<(String, String), Vec<AlertRule>> = HashMap::new();
        for rule in inner.alert_rules.values().filter(|r| r.org_id == org_id) {
            groups
                .entry((rule.folder_uid.clone(), rule.rule_group.clone()))
                .or_default()
                .push(rule.clone());
        }
        Ok(groups
            .into_iter()
            .map(|((folder_uid, name), rules)| RuleGroup {
                name,
                folder_uid,
                interval: 60,
                rules,
            })
            .collect())
    }

    // ── Contact Points ────────────────────────────────────────────────────────

    pub fn upsert_contact_point(&self, cp: ContactPoint) -> StoreResult<ContactPoint> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner.contact_points.insert(cp.uid.clone(), cp.clone());
        Ok(cp)
    }

    pub fn get_contact_point(&self, uid: &str) -> StoreResult<ContactPoint> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .contact_points
            .get(uid)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("contact point {uid}")))
    }

    pub fn delete_contact_point(&self, uid: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner
            .contact_points
            .remove(uid)
            .ok_or_else(|| StoreError::NotFound(format!("contact point {uid}")))?;
        Ok(())
    }

    pub fn list_contact_points(&self) -> StoreResult<Vec<ContactPoint>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner.contact_points.values().cloned().collect())
    }

    // ── Notification Policy ───────────────────────────────────────────────────

    pub fn set_notification_policy(&self, policy: NotificationPolicy) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner.notification_policy = Some(policy);
        Ok(())
    }

    pub fn get_notification_policy(&self) -> StoreResult<NotificationPolicy> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .notification_policy
            .clone()
            .ok_or_else(|| StoreError::NotFound("notification policy".into()))
    }

    // ── Silences ─────────────────────────────────────────────────────────────

    pub fn create_silence(&self, mut silence: Silence) -> StoreResult<Silence> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        if silence.id.is_empty() {
            silence.id = Uuid::new_v4().to_string();
        }
        inner.silences.insert(silence.id.clone(), silence.clone());
        Ok(silence)
    }

    pub fn get_silence(&self, id: &str) -> StoreResult<Silence> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .silences
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("silence {id}")))
    }

    pub fn delete_silence(&self, id: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let silence = inner
            .silences
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(format!("silence {id}")))?;
        silence.status.state = "expired".into();
        silence.ends_at = Utc::now();
        Ok(())
    }

    pub fn list_silences(&self) -> StoreResult<Vec<Silence>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner.silences.values().cloned().collect())
    }

    // ── Mute Timings ─────────────────────────────────────────────────────────

    pub fn upsert_mute_timing(&self, mt: MuteTiming) -> StoreResult<MuteTiming> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner.mute_timings.insert(mt.name.clone(), mt.clone());
        Ok(mt)
    }

    pub fn delete_mute_timing(&self, name: &str) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner
            .mute_timings
            .remove(name)
            .ok_or_else(|| StoreError::NotFound(format!("mute timing {name}")))?;
        Ok(())
    }

    pub fn list_mute_timings(&self) -> StoreResult<Vec<MuteTiming>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner.mute_timings.values().cloned().collect())
    }

    // ── Legacy Notification Channels ─────────────────────────────────────────

    pub fn create_notification_channel(
        &self,
        mut ch: AlertNotificationChannel,
    ) -> StoreResult<AlertNotificationChannel> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        ch.id = inner.next_notification_channel_id();
        inner.notification_channels.insert(ch.id, ch.clone());
        Ok(ch)
    }

    pub fn get_notification_channel(&self, id: i64) -> StoreResult<AlertNotificationChannel> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .notification_channels
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("channel {id}")))
    }

    pub fn update_notification_channel(
        &self,
        id: i64,
        mut ch: AlertNotificationChannel,
    ) -> StoreResult<AlertNotificationChannel> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let existing = inner
            .notification_channels
            .get(&id)
            .ok_or_else(|| StoreError::NotFound(format!("channel {id}")))?;
        ch.id = existing.id;
        ch.created = existing.created;
        ch.updated = Utc::now();
        inner.notification_channels.insert(id, ch.clone());
        Ok(ch)
    }

    pub fn delete_notification_channel(&self, id: i64) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        inner
            .notification_channels
            .remove(&id)
            .ok_or_else(|| StoreError::NotFound(format!("channel {id}")))?;
        Ok(())
    }

    pub fn list_notification_channels(
        &self,
        org_id: i64,
    ) -> StoreResult<Vec<AlertNotificationChannel>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .notification_channels
            .values()
            .filter(|c| c.org_id == org_id)
            .cloned()
            .collect())
    }

    // ── Org ───────────────────────────────────────────────────────────────────

    pub fn create_org(&self, name: &str) -> StoreResult<Org> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_org_id();
        let now = Utc::now();
        let org = Org {
            id,
            name: name.to_string(),
            address: OrgAddress::default(),
            created: now,
            updated: now,
        };
        inner.orgs.insert(id, org.clone());
        Ok(org)
    }

    pub fn get_org(&self, id: i64) -> StoreResult<Org> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .orgs
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("org {id}")))
    }

    pub fn list_orgs(&self) -> StoreResult<Vec<Org>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner.orgs.values().cloned().collect())
    }

    // ── User ──────────────────────────────────────────────────────────────────

    pub fn create_user(&self, req: CreateUserRequest) -> StoreResult<User> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        if inner.user_by_login.contains_key(&req.login) {
            return Err(StoreError::Conflict(format!(
                "login '{}' already exists",
                req.login
            )));
        }
        let id = inner.next_user_id();
        let now = Utc::now();
        let user = User {
            id,
            uid: Uuid::new_v4().to_string().replace('-', "")[..9].to_string(),
            name: req.name,
            email: req.email,
            login: req.login.clone(),
            org_id: req.org_id.unwrap_or(1),
            org_role: OrgRole::Viewer,
            is_admin: false,
            is_disabled: false,
            created: now,
            updated: now,
            last_seen_at: None,
            avatar_url: String::new(),
            theme: "dark".into(),
        };
        inner.user_by_login.insert(req.login, id);
        inner.users.insert(id, user.clone());
        Ok(user)
    }

    pub fn get_user(&self, id: i64) -> StoreResult<User> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .users
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("user {id}")))
    }

    pub fn list_users(&self) -> StoreResult<Vec<User>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner.users.values().cloned().collect())
    }

    // ── Team ──────────────────────────────────────────────────────────────────

    pub fn create_team(&self, org_id: i64, name: &str, email: &str) -> StoreResult<Team> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_team_id();
        let now = Utc::now();
        let team = Team {
            id,
            uid: Uuid::new_v4().to_string().replace('-', "")[..9].to_string(),
            org_id,
            name: name.to_string(),
            email: email.to_string(),
            avatar_url: String::new(),
            member_count: 0,
            created: now,
            updated: now,
        };
        inner.teams.insert(id, team.clone());
        Ok(team)
    }

    pub fn get_team(&self, id: i64) -> StoreResult<Team> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .teams
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("team {id}")))
    }

    pub fn list_teams(&self, org_id: i64) -> StoreResult<Vec<Team>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .teams
            .values()
            .filter(|t| t.org_id == org_id)
            .cloned()
            .collect())
    }

    pub fn add_team_member(&self, team_id: i64, member: TeamMember) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let members = inner.team_members.entry(team_id).or_default();
        if members.iter().any(|m| m.user_id == member.user_id) {
            return Err(StoreError::Conflict(format!(
                "user {} already in team {team_id}",
                member.user_id
            )));
        }
        members.push(member);
        if let Some(team) = inner.teams.get_mut(&team_id) {
            team.member_count += 1;
        }
        Ok(())
    }

    pub fn list_team_members(&self, team_id: i64) -> StoreResult<Vec<TeamMember>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .team_members
            .get(&team_id)
            .cloned()
            .unwrap_or_default())
    }

    // ── API Keys ─────────────────────────────────────────────────────────────

    pub fn create_api_key(
        &self,
        org_id: i64,
        name: &str,
        role: OrgRole,
        seconds_to_live: Option<i64>,
        key_hash: &str,
        key: &str,
    ) -> StoreResult<ApiKey> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_apikey_id();
        let now = Utc::now();
        let expires = seconds_to_live.map(|s| now + chrono::Duration::seconds(s));
        let api_key = ApiKey {
            id,
            org_id,
            name: name.to_string(),
            role,
            created: now,
            updated: now,
            expires,
            key_hash: key_hash.to_string(),
            key: Some(key.to_string()),
        };
        inner.api_keys.insert(id, api_key.clone());
        inner.api_key_by_hash.insert(key_hash.to_string(), id);
        Ok(api_key)
    }

    pub fn delete_api_key(&self, id: i64) -> StoreResult<()> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let key = inner
            .api_keys
            .remove(&id)
            .ok_or_else(|| StoreError::NotFound(format!("api key {id}")))?;
        inner.api_key_by_hash.remove(&key.key_hash);
        Ok(())
    }

    pub fn list_api_keys(&self, org_id: i64) -> StoreResult<Vec<ApiKey>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .api_keys
            .values()
            .filter(|k| k.org_id == org_id)
            .map(|k| {
                let mut k = k.clone();
                k.key = None; // Never return key after creation
                k
            })
            .collect())
    }

    pub fn lookup_api_key(&self, key_hash: &str) -> StoreResult<ApiKey> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        let id = inner
            .api_key_by_hash
            .get(key_hash)
            .ok_or_else(|| StoreError::NotFound("api key".into()))?;
        inner
            .api_keys
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound("api key".into()))
    }

    // ── Service Accounts ─────────────────────────────────────────────────────

    pub fn create_service_account(
        &self,
        org_id: i64,
        name: &str,
        role: OrgRole,
    ) -> StoreResult<ServiceAccount> {
        let mut inner = self.inner.write().map_err(|_| StoreError::Lock)?;
        let id = inner.next_sa_id();
        let now = Utc::now();
        let sa = ServiceAccount {
            id,
            uid: Uuid::new_v4().to_string().replace('-', "")[..9].to_string(),
            org_id,
            name: name.to_string(),
            login: format!("sa-{}", name.to_lowercase().replace(' ', "-")),
            role,
            is_disabled: false,
            created: now,
            updated: now,
            avatar_url: String::new(),
            tokens: 0,
        };
        inner.service_accounts.insert(id, sa.clone());
        Ok(sa)
    }

    pub fn get_service_account(&self, id: i64) -> StoreResult<ServiceAccount> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        inner
            .service_accounts
            .get(&id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("service account {id}")))
    }

    pub fn list_service_accounts(&self, org_id: i64) -> StoreResult<Vec<ServiceAccount>> {
        let inner = self.inner.read().map_err(|_| StoreError::Lock)?;
        Ok(inner
            .service_accounts
            .values()
            .filter(|s| s.org_id == org_id)
            .cloned()
            .collect())
    }
}
