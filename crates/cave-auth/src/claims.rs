//! JWT claims extraction — maps Okta and Keycloak tokens to CaveIdentity.

use cave_core::types::{CaveIdentity, CaveRole};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

/// Raw JWT claims from the OIDC provider.
#[derive(Debug, Deserialize)]
pub struct RawClaims {
    /// IdP-specific subject (varies per provider)
    pub sub: String,
    /// Platform-generated stable UUID
    pub cave_uid: Option<String>,
    /// Tenant ID
    pub tenant_id: Option<String>,
    /// Environment
    pub env: Option<String>,
    /// Okta groups or Keycloak realm roles
    pub groups: Option<Vec<String>>,
    /// Keycloak realm_access.roles
    pub realm_access: Option<RealmAccess>,
    /// Token expiry (Unix timestamp)
    pub exp: i64,
    /// Audience
    pub aud: serde_json::Value,
    /// Issuer
    pub iss: String,
}

#[derive(Debug, Deserialize)]
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

        // Extract roles from either Okta groups or Keycloak realm roles
        let role_strings: Vec<&str> = if let Some(groups) = &self.groups {
            groups.iter().map(|s| s.as_str()).collect()
        } else if let Some(realm) = &self.realm_access {
            realm.roles.iter().map(|s| s.as_str()).collect()
        } else {
            vec![]
        };

        let roles = role_strings
            .iter()
            .filter_map(|r| match *r {
                "platform-admin" | "cave-platform-admin" => Some(CaveRole::PlatformAdmin),
                "platform-viewer" | "cave-platform-viewer" => Some(CaveRole::PlatformViewer),
                "tenant-admin" | "cave-tenant-admin" => Some(CaveRole::TenantAdmin),
                "tenant-developer" | "cave-tenant-developer" => Some(CaveRole::TenantDeveloper),
                "tenant-viewer" | "cave-tenant-viewer" => Some(CaveRole::TenantViewer),
                _ => None,
            })
            .collect();

        let exp = DateTime::<Utc>::from_timestamp(self.exp, 0)
            .ok_or("Invalid exp timestamp")?;

        Ok(CaveIdentity {
            cave_uid,
            tenant_id,
            env,
            roles,
            exp,
        })
    }
}
