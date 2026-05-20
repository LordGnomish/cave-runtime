// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SCIM 2.0 provisioning — RFC 7643 / RFC 7644.
//!
//! Provides /Users and /Groups CRUD, list/filter, and bulk operations.
//! Okta calls this endpoint to sync users and groups from Okta to CAVE.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub const SCIM_SCHEMA_USER: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
pub const SCIM_SCHEMA_GROUP: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
pub const SCIM_SCHEMA_LIST: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
pub const SCIM_SCHEMA_ERROR: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
pub const SCIM_SCHEMA_BULK: &str = "urn:ietf:params:scim:api:messages:2.0:BulkRequest";

/// SCIM user name (RFC 7643 §4.1.1).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScimName {
    pub formatted: Option<String>,
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    pub middle_name: Option<String>,
}

/// SCIM multi-valued attribute (email, phone, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimMultiValue {
    pub value: String,
    #[serde(rename = "type")]
    pub value_type: Option<String>,
    pub primary: Option<bool>,
    pub display: Option<String>,
}

/// Reference to a group (embedded in user's groups list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroupRef {
    pub value: String,
    #[serde(rename = "$ref")]
    pub ref_: Option<String>,
    pub display: Option<String>,
}

/// Reference to a member (embedded in group's members list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimMember {
    pub value: String,
    #[serde(rename = "$ref")]
    pub ref_: Option<String>,
    pub display: Option<String>,
    #[serde(rename = "type")]
    pub member_type: Option<String>, // "User" or "Group"
}

/// SCIM resource metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimMeta {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    pub created: DateTime<Utc>,
    #[serde(rename = "lastModified")]
    pub last_modified: DateTime<Utc>,
    pub location: Option<String>,
    pub version: Option<String>,
}

impl ScimMeta {
    pub fn new(resource_type: &str) -> Self {
        let now = Utc::now();
        Self {
            resource_type: resource_type.to_string(),
            created: now,
            last_modified: now,
            location: None,
            version: Some(format!("W/\"{}\"-1", Uuid::new_v4())),
        }
    }
}

/// SCIM User resource (RFC 7643 §4.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimUser {
    pub id: String,
    pub schemas: Vec<String>,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(rename = "userName")]
    pub user_name: String,
    pub name: Option<ScimName>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub emails: Vec<ScimMultiValue>,
    #[serde(rename = "phoneNumbers")]
    pub phone_numbers: Vec<ScimMultiValue>,
    pub active: bool,
    pub groups: Vec<ScimGroupRef>,
    pub meta: ScimMeta,
}

impl ScimUser {
    pub fn new(user_name: &str, email: Option<&str>) -> Self {
        let id = Uuid::new_v4().to_string();
        Self {
            id: id,
            schemas: vec![SCIM_SCHEMA_USER.to_string()],
            external_id: None,
            user_name: user_name.to_string(),
            name: None,
            display_name: None,
            emails: email
                .map(|e| {
                    vec![ScimMultiValue {
                        value: e.to_string(),
                        value_type: Some("work".to_string()),
                        primary: Some(true),
                        display: None,
                    }]
                })
                .unwrap_or_default(),
            phone_numbers: vec![],
            active: true,
            groups: vec![],
            meta: ScimMeta::new("User"),
        }
    }

    pub fn primary_email(&self) -> Option<&str> {
        self.emails
            .iter()
            .find(|e| e.primary == Some(true))
            .or_else(|| self.emails.first())
            .map(|e| e.value.as_str())
    }
}

/// SCIM Group resource (RFC 7643 §4.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimGroup {
    pub id: String,
    pub schemas: Vec<String>,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub members: Vec<ScimMember>,
    pub meta: ScimMeta,
}

impl ScimGroup {
    pub fn new(display_name: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            schemas: vec![SCIM_SCHEMA_GROUP.to_string()],
            external_id: None,
            display_name: display_name.to_string(),
            members: vec![],
            meta: ScimMeta::new("Group"),
        }
    }
}

/// SCIM ListResponse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScimListResponse<T> {
    pub schemas: Vec<String>,
    #[serde(rename = "totalResults")]
    pub total_results: usize,
    #[serde(rename = "startIndex")]
    pub start_index: usize,
    #[serde(rename = "itemsPerPage")]
    pub items_per_page: usize,
    #[serde(rename = "Resources")]
    pub resources: Vec<T>,
}

impl<T> ScimListResponse<T> {
    pub fn new(resources: Vec<T>, start_index: usize, count: usize) -> Self {
        let total = resources.len();
        Self {
            schemas: vec![SCIM_SCHEMA_LIST.to_string()],
            total_results: total,
            start_index,
            items_per_page: count,
            resources,
        }
    }
}

/// Bulk operation (RFC 7644 §3.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkOperation {
    pub method: String, // "POST", "PUT", "PATCH", "DELETE"
    pub path: String,
    pub bulk_id: Option<String>,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkRequest {
    pub schemas: Vec<String>,
    #[serde(rename = "failOnErrors")]
    pub fail_on_errors: Option<usize>,
    pub operations: Vec<BulkOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkResult {
    pub bulk_id: Option<String>,
    pub method: String,
    pub location: Option<String>,
    pub status: u16,
    pub response: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkResponse {
    pub schemas: Vec<String>,
    pub operations: Vec<BulkResult>,
}

/// Error returned from SCIM operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ScimError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Already exists: {0}")]
    Conflict(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
}

/// SCIM service — in-memory store for CAVE users and groups.
#[derive(Clone)]
pub struct ScimService {
    users: Arc<RwLock<HashMap<String, ScimUser>>>,
    groups: Arc<RwLock<HashMap<String, ScimGroup>>>,
    base_url: String,
}

impl ScimService {
    pub fn new(base_url: &str) -> Self {
        Self {
            users: Arc::new(RwLock::new(HashMap::new())),
            groups: Arc::new(RwLock::new(HashMap::new())),
            base_url: base_url.to_string(),
        }
    }

    // ── Users ──────────────────────────────────────────────────────────────

    pub async fn create_user(&self, mut user: ScimUser) -> Result<ScimUser, ScimError> {
        let mut users = self.users.write().await;
        // Check username uniqueness
        if users.values().any(|u| u.user_name == user.user_name) {
            return Err(ScimError::Conflict(format!(
                "User '{}' already exists",
                user.user_name
            )));
        }
        user.meta.location = Some(format!("{}/scim/v2/Users/{}", self.base_url, user.id));
        users.insert(user.id.clone(), user.clone());
        Ok(user)
    }

    pub async fn get_user(&self, id: &str) -> Result<ScimUser, ScimError> {
        self.users
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ScimError::NotFound(format!("User {id}")))
    }

    pub async fn update_user(&self, id: &str, mut user: ScimUser) -> Result<ScimUser, ScimError> {
        let mut users = self.users.write().await;
        if !users.contains_key(id) {
            return Err(ScimError::NotFound(format!("User {id}")));
        }
        user.id = id.to_string();
        user.meta.last_modified = Utc::now();
        user.meta.location = Some(format!("{}/scim/v2/Users/{id}", self.base_url));
        users.insert(id.to_string(), user.clone());
        Ok(user)
    }

    pub async fn delete_user(&self, id: &str) -> Result<(), ScimError> {
        let mut users = self.users.write().await;
        if users.remove(id).is_none() {
            return Err(ScimError::NotFound(format!("User {id}")));
        }
        Ok(())
    }

    pub async fn list_users(&self, start: usize, count: usize) -> ScimListResponse<ScimUser> {
        let users: Vec<ScimUser> = self
            .users
            .read()
            .await
            .values()
            .skip(start.saturating_sub(1))
            .take(count)
            .cloned()
            .collect();
        let total = self.users.read().await.len();
        ScimListResponse {
            schemas: vec![SCIM_SCHEMA_LIST.to_string()],
            total_results: total,
            start_index: start,
            items_per_page: users.len(),
            resources: users,
        }
    }

    // ── Groups ─────────────────────────────────────────────────────────────

    pub async fn create_group(&self, mut group: ScimGroup) -> Result<ScimGroup, ScimError> {
        let mut groups = self.groups.write().await;
        if groups
            .values()
            .any(|g| g.display_name == group.display_name)
        {
            return Err(ScimError::Conflict(format!(
                "Group '{}' already exists",
                group.display_name
            )));
        }
        group.meta.location = Some(format!("{}/scim/v2/Groups/{}", self.base_url, group.id));
        groups.insert(group.id.clone(), group.clone());
        Ok(group)
    }

    pub async fn get_group(&self, id: &str) -> Result<ScimGroup, ScimError> {
        self.groups
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| ScimError::NotFound(format!("Group {id}")))
    }

    pub async fn update_group(
        &self,
        id: &str,
        mut group: ScimGroup,
    ) -> Result<ScimGroup, ScimError> {
        let mut groups = self.groups.write().await;
        if !groups.contains_key(id) {
            return Err(ScimError::NotFound(format!("Group {id}")));
        }
        group.id = id.to_string();
        group.meta.last_modified = Utc::now();
        groups.insert(id.to_string(), group.clone());
        Ok(group)
    }

    pub async fn delete_group(&self, id: &str) -> Result<(), ScimError> {
        let mut groups = self.groups.write().await;
        if groups.remove(id).is_none() {
            return Err(ScimError::NotFound(format!("Group {id}")));
        }
        Ok(())
    }

    pub async fn add_member_to_group(
        &self,
        group_id: &str,
        member: ScimMember,
    ) -> Result<(), ScimError> {
        let mut groups = self.groups.write().await;
        let group = groups
            .get_mut(group_id)
            .ok_or_else(|| ScimError::NotFound(format!("Group {group_id}")))?;
        if !group.members.iter().any(|m| m.value == member.value) {
            group.members.push(member);
        }
        Ok(())
    }

    pub async fn list_groups(&self, start: usize, count: usize) -> ScimListResponse<ScimGroup> {
        let groups: Vec<ScimGroup> = self
            .groups
            .read()
            .await
            .values()
            .skip(start.saturating_sub(1))
            .take(count)
            .cloned()
            .collect();
        let total = self.groups.read().await.len();
        ScimListResponse {
            schemas: vec![SCIM_SCHEMA_LIST.to_string()],
            total_results: total,
            start_index: start,
            items_per_page: groups.len(),
            resources: groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scim_create_and_get_user() {
        let svc = ScimService::new("https://cave.example.com");
        let user = ScimUser::new("alice@example.com", Some("alice@example.com"));
        let created = svc.create_user(user).await.unwrap();
        let fetched = svc.get_user(&created.id).await.unwrap();
        assert_eq!(fetched.user_name, "alice@example.com");
        assert!(fetched.meta.location.is_some());
    }

    #[tokio::test]
    async fn scim_duplicate_username_conflicts() {
        let svc = ScimService::new("https://cave.example.com");
        let u1 = ScimUser::new("bob@example.com", None);
        let u2 = ScimUser::new("bob@example.com", None);
        svc.create_user(u1).await.unwrap();
        let err = svc.create_user(u2).await.unwrap_err();
        assert!(matches!(err, ScimError::Conflict(_)));
    }

    #[tokio::test]
    async fn scim_update_user() {
        let svc = ScimService::new("https://cave.example.com");
        let user = ScimUser::new("carol@example.com", None);
        let created = svc.create_user(user.clone()).await.unwrap();

        let mut updated = created.clone();
        updated.active = false;
        let result = svc.update_user(&created.id, updated).await.unwrap();
        assert!(!result.active);
    }

    #[tokio::test]
    async fn scim_delete_user() {
        let svc = ScimService::new("https://cave.example.com");
        let user = ScimUser::new("dave@example.com", None);
        let created = svc.create_user(user).await.unwrap();

        svc.delete_user(&created.id).await.unwrap();
        let err = svc.get_user(&created.id).await.unwrap_err();
        assert!(matches!(err, ScimError::NotFound(_)));
    }

    #[tokio::test]
    async fn scim_create_and_get_group() {
        let svc = ScimService::new("https://cave.example.com");
        let group = ScimGroup::new("Platform Admins");
        let created = svc.create_group(group).await.unwrap();
        let fetched = svc.get_group(&created.id).await.unwrap();
        assert_eq!(fetched.display_name, "Platform Admins");
    }

    #[tokio::test]
    async fn scim_add_member_to_group() {
        let svc = ScimService::new("https://cave.example.com");
        let group = ScimGroup::new("Developers");
        let created_group = svc.create_group(group).await.unwrap();
        let user = ScimUser::new("eve@example.com", None);
        let created_user = svc.create_user(user).await.unwrap();

        svc.add_member_to_group(
            &created_group.id,
            ScimMember {
                value: created_user.id.clone(),
                ref_: None,
                display: Some("eve@example.com".to_string()),
                member_type: Some("User".to_string()),
            },
        )
        .await
        .unwrap();

        let group = svc.get_group(&created_group.id).await.unwrap();
        assert_eq!(group.members.len(), 1);
        assert_eq!(group.members[0].value, created_user.id);
    }

    #[tokio::test]
    async fn scim_list_users_pagination() {
        let svc = ScimService::new("https://cave.example.com");
        for i in 0..5 {
            svc.create_user(ScimUser::new(&format!("user{i}@example.com"), None))
                .await
                .unwrap();
        }
        let page = svc.list_users(1, 3).await;
        assert_eq!(page.total_results, 5);
        assert_eq!(page.resources.len(), 3);
    }
}
