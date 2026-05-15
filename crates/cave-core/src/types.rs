// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared types used across all modules.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical user identity — extracted from JWT, used across all modules.
/// Always use `cave_uid`, never the IdP `sub` claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaveIdentity {
    /// Platform-generated stable UUID (survives IdP migration)
    pub cave_uid: Uuid,
    /// Tenant scope
    pub tenant_id: String,
    /// Environment scope
    pub env: String,
    /// Platform roles
    pub roles: Vec<CaveRole>,
    /// Token expiry
    pub exp: DateTime<Utc>,
    /// User email from IdP
    #[serde(default)]
    pub email: Option<String>,
    /// Okta group memberships (raw group names)
    #[serde(default)]
    pub groups: Vec<String>,
    /// Resolved fine-grained permissions, e.g. "cave-flags:write"
    #[serde(default)]
    pub permissions: Vec<String>,
    /// How this identity was authenticated
    #[serde(default)]
    pub token_type: TokenType,
}

/// How a request was authenticated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    #[default]
    Jwt,
    PersonalAccessToken,
    ServiceToken,
}

/// Hierarchical platform roles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaveRole {
    PlatformAdmin,
    PlatformViewer,
    TenantAdmin,
    TenantDeveloper,
    TenantViewer,
    ModuleAdmin,
    Developer,
    Auditor,
}

/// Module permission check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub module: String,
    pub action: String,
}

impl CaveIdentity {
    /// Check if this identity has the required permission.
    pub fn has_permission(&self, module: &str, action: &str) -> bool {
        let perm = format!("{module}:{action}");
        let wildcard = format!("{module}:*");
        if self.permissions.contains(&perm)
            || self.permissions.contains(&wildcard)
            || self.permissions.contains(&"*".to_string())
        {
            return true;
        }

        for role in &self.roles {
            let allowed = match role {
                CaveRole::PlatformAdmin => true,
                CaveRole::TenantAdmin | CaveRole::ModuleAdmin => {
                    !action.contains("platform:")
                }
                CaveRole::TenantDeveloper | CaveRole::Developer => {
                    !action.contains("admin") && !action.contains("platform:")
                }
                CaveRole::TenantViewer | CaveRole::PlatformViewer => {
                    action.contains("read") || action.contains("list")
                }
                CaveRole::Auditor => {
                    action.contains("read")
                        || action.contains("list")
                        || action.contains("audit")
                }
            };
            if allowed {
                return true;
            }
        }
        false
    }
}

/// Upstream tracking status for a feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamFeature {
    pub project: String,
    pub upstream_version: String,
    pub upstream_url: String,
    pub triage: UpstreamTriage,
    pub implemented_in: Option<String>,
    pub notes: String,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum UpstreamTriage {
    Adopt,
    Watch,
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_identity(roles: Vec<CaveRole>) -> CaveIdentity {
        CaveIdentity {
            cave_uid: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            env: "prod".to_string(),
            roles,
            exp: Utc::now() + Duration::hours(1),
            email: None,
            groups: vec![],
            permissions: vec![],
            token_type: TokenType::Jwt,
        }
    }

    #[test]
    fn test_platform_admin_can_read() {
        let id = make_identity(vec![CaveRole::PlatformAdmin]);
        assert!(id.has_permission("flags", "flags:read"));
    }

    #[test]
    fn test_platform_admin_can_write() {
        let id = make_identity(vec![CaveRole::PlatformAdmin]);
        assert!(id.has_permission("flags", "flags:write"));
    }

    #[test]
    fn test_tenant_admin_cannot_platform() {
        let id = make_identity(vec![CaveRole::TenantAdmin]);
        assert!(!id.has_permission("admin", "platform:manage"));
    }

    #[test]
    fn test_tenant_developer_cannot_admin() {
        let id = make_identity(vec![CaveRole::TenantDeveloper]);
        assert!(!id.has_permission("flags", "flags:admin"));
    }

    #[test]
    fn test_tenant_viewer_can_read() {
        let id = make_identity(vec![CaveRole::TenantViewer]);
        assert!(id.has_permission("flags", "flags:read"));
    }

    #[test]
    fn test_tenant_viewer_cannot_write() {
        let id = make_identity(vec![CaveRole::TenantViewer]);
        assert!(!id.has_permission("flags", "flags:write"));
    }
}
