//! JWT claims extraction — maps Okta and Keycloak tokens to CaveIdentity.

use cave_core::types::{CaveIdentity, CaveRole, TokenType};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Raw JWT claims from the OIDC provider.
/// Supports both Okta (groups, custom claims) and Keycloak (realm_access.roles) formats.
#[derive(Debug, Serialize, Deserialize)]
pub struct RawClaims {
    /// IdP-specific subject (varies per provider)
    pub sub: String,
    /// Platform-generated stable UUID
    pub cave_uid: Option<String>,
    /// Tenant ID
    pub tenant_id: Option<String>,
    /// Environment
    pub env: Option<String>,
    /// User email
    pub email: Option<String>,
    /// Okta groups or Keycloak realm roles
    pub groups: Option<Vec<String>>,
    /// Keycloak realm_access.roles
    pub realm_access: Option<RealmAccess>,
    /// Okta custom authorization server scopes (space-separated)
    pub scp: Option<String>,
    /// Token expiry (Unix timestamp)
    pub exp: i64,
    /// Audience
    pub aud: serde_json::Value,
    /// Issuer
    pub iss: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RealmAccess {
    pub roles: Vec<String>,
}

impl RawClaims {
    /// Convert raw claims to canonical CaveIdentity.
    /// Handles both Okta (groups) and Keycloak (realm_access.roles) formats.
    pub fn to_identity(&self) -> Result<CaveIdentity, String> {
        let cave_uid = self
            .cave_uid
            .as_ref()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or("Missing or invalid cave_uid claim")?;

        let tenant_id = self
            .tenant_id
            .clone()
            .unwrap_or_else(|| "platform".to_string());

        let env = self.env.clone().unwrap_or_else(|| "prod".to_string());

        // Extract role strings from either Okta groups or Keycloak realm roles
        let group_strings: Vec<String> = if let Some(groups) = &self.groups {
            groups.clone()
        } else if let Some(realm) = &self.realm_access {
            realm.roles.clone()
        } else {
            vec![]
        };

        let roles: Vec<CaveRole> = group_strings
            .iter()
            .filter_map(|r| map_group_to_role(r.as_str()))
            .collect();

        let permissions = resolve_permissions(&roles, self.scp.as_deref());

        let exp = DateTime::<Utc>::from_timestamp(self.exp, 0)
            .ok_or("Invalid exp timestamp")?;

        Ok(CaveIdentity {
            cave_uid,
            tenant_id,
            env,
            roles,
            exp,
            email: self.email.clone(),
            groups: group_strings,
            permissions,
            token_type: TokenType::Jwt,
        })
    }
}

/// Map an Okta group / Keycloak role name to a CaveRole.
fn map_group_to_role(name: &str) -> Option<CaveRole> {
    match name {
        "platform-admin" | "cave-platform-admin" | "CAVE_PLATFORM_ADMIN" => {
            Some(CaveRole::PlatformAdmin)
        }
        "platform-viewer" | "cave-platform-viewer" => Some(CaveRole::PlatformViewer),
        "tenant-admin" | "cave-tenant-admin" => Some(CaveRole::TenantAdmin),
        "module-admin" | "cave-module-admin" => Some(CaveRole::ModuleAdmin),
        "tenant-developer" | "cave-tenant-developer" | "developer" | "cave-developer" => {
            Some(CaveRole::TenantDeveloper)
        }
        "tenant-viewer" | "cave-tenant-viewer" => Some(CaveRole::TenantViewer),
        "auditor" | "cave-auditor" => Some(CaveRole::Auditor),
        _ => None,
    }
}

/// Resolve fine-grained permissions from roles + optional OAuth scopes.
/// This is the default resolver; the RBAC engine can add per-resource bindings on top.
fn resolve_permissions(roles: &[CaveRole], scp: Option<&str>) -> Vec<String> {
    let mut perms: Vec<String> = Vec::new();

    // Coarse permissions derived from role hierarchy
    for role in roles {
        match role {
            CaveRole::PlatformAdmin => {
                perms.push("*".to_string());
                return perms; // wildcard covers everything
            }
            CaveRole::TenantAdmin | CaveRole::ModuleAdmin => {
                for module in ALL_MODULES {
                    perms.push(format!("{module}:read"));
                    perms.push(format!("{module}:write"));
                    perms.push(format!("{module}:manage"));
                }
            }
            CaveRole::TenantDeveloper | CaveRole::Developer => {
                for module in ALL_MODULES {
                    perms.push(format!("{module}:read"));
                    perms.push(format!("{module}:write"));
                }
            }
            CaveRole::TenantViewer | CaveRole::PlatformViewer => {
                for module in ALL_MODULES {
                    perms.push(format!("{module}:read"));
                }
            }
            CaveRole::Auditor => {
                perms.push("cave-logs:read".to_string());
                perms.push("cave-auth:audit".to_string());
                perms.push("cave-vulns:read".to_string());
                perms.push("cave-incidents:read".to_string());
            }
        }
    }

    // Merge explicit OAuth scopes (e.g., "cave-flags:write cave-vulns:read")
    if let Some(scopes) = scp {
        for scope in scopes.split_whitespace() {
            let s = scope.to_string();
            if !perms.contains(&s) {
                perms.push(s);
            }
        }
    }

    perms.sort();
    perms.dedup();
    perms
}

/// All CAVE module slugs — used for bulk permission generation.
const ALL_MODULES: &[&str] = &[
    "cave-flags",
    "cave-secrets",
    "cave-lint",
    "cave-docs",
    "cave-status",
    "cave-changelog",
    "cave-certs",
    "cave-vulns",
    "cave-sbom",
    "cave-uptime",
    "cave-cost",
    "cave-sign",
    "cave-forensics",
    "cave-devlake",
    "cave-ai-obs",
    "cave-pii",
    "cave-incidents",
    "cave-chat",
    "cave-slo",
    "cave-alerts",
    "cave-profiler",
    "cave-registry",
    "cave-workflows",
    "cave-scan",
    "cave-portal",
    "cave-scaffold",
    "cave-chaos",
    "cave-policy",
    "cave-dast",
    "cave-backup",
    "cave-pam",
    "cave-logs",
    "cave-auth",
];
