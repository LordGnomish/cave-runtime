// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Multi-tenancy — tenant isolation, tenant-scoped roles and configuration.
//!
//! Each tenant in CAVE is completely isolated: separate namespaces, resource quotas,
//! role assignments, and audit logs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Tenant status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    Suspended,
    Deprovisioning,
    Deleted,
}

/// A CAVE tenant (organization/team using the platform).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub display_name: String,
    pub slug: String,
    /// Okta/Keycloak group or realm this tenant maps to.
    pub idp_group: Option<String>,
    pub status: TenantStatus,
    /// Allowed environments for this tenant.
    pub environments: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub settings: TenantSettings,
}

/// Per-tenant configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantSettings {
    /// Maximum members allowed.
    pub max_members: Option<usize>,
    /// Whether this tenant can create sub-tenants.
    pub allow_sub_tenants: bool,
    /// Custom OIDC group prefix for this tenant.
    pub oidc_group_prefix: Option<String>,
    /// SSO enforced (no password login).
    pub sso_enforced: bool,
    /// MFA required for all members.
    pub mfa_required: bool,
    /// Session TTL override in minutes.
    pub session_ttl_minutes: Option<i64>,
    /// Custom metadata.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Default for TenantSettings {
    fn default() -> Self {
        Self {
            max_members: None,
            allow_sub_tenants: false,
            oidc_group_prefix: None,
            sso_enforced: false,
            mfa_required: false,
            session_ttl_minutes: None,
            metadata: HashMap::new(),
        }
    }
}

/// A member of a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantMember {
    pub user_id: Uuid,
    pub tenant_id: String,
    pub display_name: String,
    pub email: String,
    pub joined_at: DateTime<Utc>,
    pub invited_by: Option<Uuid>,
    pub active: bool,
}

/// Multi-tenant registry.
#[derive(Clone)]
pub struct TenantRegistry {
    tenants: Arc<RwLock<HashMap<String, Tenant>>>,
    members: Arc<RwLock<Vec<TenantMember>>>,
}

impl TenantRegistry {
    pub fn new() -> Self {
        Self {
            tenants: Arc::new(RwLock::new(HashMap::new())),
            members: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn create_tenant(&self, tenant: Tenant) -> Result<Tenant, String> {
        let mut tenants = self.tenants.write().await;
        if tenants.contains_key(&tenant.id) {
            return Err(format!("Tenant '{}' already exists", tenant.id));
        }
        if tenants.values().any(|t| t.slug == tenant.slug) {
            return Err(format!("Slug '{}' already in use", tenant.slug));
        }
        tenants.insert(tenant.id.clone(), tenant.clone());
        Ok(tenant)
    }

    pub async fn get_tenant(&self, id: &str) -> Option<Tenant> {
        self.tenants.read().await.get(id).cloned()
    }

    pub async fn get_by_slug(&self, slug: &str) -> Option<Tenant> {
        self.tenants
            .read()
            .await
            .values()
            .find(|t| t.slug == slug)
            .cloned()
    }

    pub async fn suspend_tenant(&self, id: &str) -> Result<(), String> {
        let mut tenants = self.tenants.write().await;
        let tenant = tenants
            .get_mut(id)
            .ok_or_else(|| format!("Tenant {id} not found"))?;
        tenant.status = TenantStatus::Suspended;
        Ok(())
    }

    pub async fn activate_tenant(&self, id: &str) -> Result<(), String> {
        let mut tenants = self.tenants.write().await;
        let tenant = tenants
            .get_mut(id)
            .ok_or_else(|| format!("Tenant {id} not found"))?;
        tenant.status = TenantStatus::Active;
        Ok(())
    }

    pub async fn add_member(&self, member: TenantMember) -> Result<(), String> {
        let tenants = self.tenants.read().await;
        let tenant = tenants
            .get(&member.tenant_id)
            .ok_or_else(|| format!("Tenant {} not found", member.tenant_id))?;

        if tenant.status != TenantStatus::Active {
            return Err(format!("Tenant {} is not active", member.tenant_id));
        }

        // Check member limit
        if let Some(max) = tenant.settings.max_members {
            let count = self
                .members
                .read()
                .await
                .iter()
                .filter(|m| m.tenant_id == member.tenant_id && m.active)
                .count();
            if count >= max {
                return Err(format!("Tenant {} has reached member limit ({max})", member.tenant_id));
            }
        }
        drop(tenants);

        self.members.write().await.push(member);
        Ok(())
    }

    pub async fn list_members(&self, tenant_id: &str) -> Vec<TenantMember> {
        self.members
            .read()
            .await
            .iter()
            .filter(|m| m.tenant_id == tenant_id && m.active)
            .cloned()
            .collect()
    }

    pub async fn remove_member(&self, user_id: Uuid, tenant_id: &str) {
        let mut members = self.members.write().await;
        if let Some(m) = members
            .iter_mut()
            .find(|m| m.user_id == user_id && m.tenant_id == tenant_id)
        {
            m.active = false;
        }
    }

    /// Verify a user belongs to a tenant and it's active.
    pub async fn assert_member(&self, user_id: Uuid, tenant_id: &str) -> Result<(), String> {
        let tenants = self.tenants.read().await;
        let tenant = tenants
            .get(tenant_id)
            .ok_or_else(|| format!("Tenant {tenant_id} not found"))?;
        if tenant.status != TenantStatus::Active {
            return Err(format!("Tenant {tenant_id} is not active"));
        }
        drop(tenants);

        let members = self.members.read().await;
        let is_member = members
            .iter()
            .any(|m| m.user_id == user_id && m.tenant_id == tenant_id && m.active);

        if is_member {
            Ok(())
        } else {
            Err(format!("User {user_id} is not a member of tenant {tenant_id}"))
        }
    }

    pub async fn list_tenants(&self) -> Vec<Tenant> {
        self.tenants.read().await.values().cloned().collect()
    }
}

impl Default for TenantRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to build a tenant.
pub fn new_tenant(id: &str, display_name: &str, created_by: Uuid) -> Tenant {
    Tenant {
        id: id.to_string(),
        display_name: display_name.to_string(),
        slug: id.to_lowercase().replace(' ', "-"),
        idp_group: None,
        status: TenantStatus::Active,
        environments: vec!["prod".to_string(), "staging".to_string(), "dev".to_string()],
        created_at: Utc::now(),
        created_by,
        settings: TenantSettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tenant_create_and_get() {
        let reg = TenantRegistry::new();
        let admin = Uuid::new_v4();
        let tenant = new_tenant("acme", "ACME Corp", admin);
        reg.create_tenant(tenant).await.unwrap();

        let fetched = reg.get_tenant("acme").await.unwrap();
        assert_eq!(fetched.display_name, "ACME Corp");
        assert_eq!(fetched.status, TenantStatus::Active);
    }

    #[tokio::test]
    async fn tenant_duplicate_id_fails() {
        let reg = TenantRegistry::new();
        let admin = Uuid::new_v4();
        reg.create_tenant(new_tenant("acme", "ACME", admin))
            .await
            .unwrap();
        let err = reg
            .create_tenant(new_tenant("acme", "ACME Dup", admin))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn tenant_suspend_and_activate() {
        let reg = TenantRegistry::new();
        let admin = Uuid::new_v4();
        reg.create_tenant(new_tenant("corp", "Corp", admin))
            .await
            .unwrap();

        reg.suspend_tenant("corp").await.unwrap();
        assert_eq!(
            reg.get_tenant("corp").await.unwrap().status,
            TenantStatus::Suspended
        );

        reg.activate_tenant("corp").await.unwrap();
        assert_eq!(
            reg.get_tenant("corp").await.unwrap().status,
            TenantStatus::Active
        );
    }

    #[tokio::test]
    async fn tenant_member_isolation() {
        let reg = TenantRegistry::new();
        let admin = Uuid::new_v4();
        reg.create_tenant(new_tenant("tenant-a", "A", admin))
            .await
            .unwrap();
        reg.create_tenant(new_tenant("tenant-b", "B", admin))
            .await
            .unwrap();

        let user_a = Uuid::new_v4();
        reg.add_member(TenantMember {
            user_id: user_a,
            tenant_id: "tenant-a".to_string(),
            display_name: "User A".to_string(),
            email: "a@a.com".to_string(),
            joined_at: Utc::now(),
            invited_by: None,
            active: true,
        })
        .await
        .unwrap();

        // user_a is member of tenant-a
        assert!(reg.assert_member(user_a, "tenant-a").await.is_ok());
        // user_a is NOT member of tenant-b
        assert!(reg.assert_member(user_a, "tenant-b").await.is_err());
    }
}
