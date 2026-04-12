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

#[cfg(test)]
mod tests {
    use super::*;
    use cave_core::types::CaveRole;

    fn base_claims(cave_uid: &str) -> RawClaims {
        RawClaims {
            sub: "user-sub-123".to_string(),
            cave_uid: Some(cave_uid.to_string()),
            tenant_id: None,
            env: None,
            groups: None,
            realm_access: None,
            exp: 9999999999,
            aud: serde_json::Value::String("cave-runtime".to_string()),
            iss: "https://auth.example.com".to_string(),
        }
    }

    const VALID_UUID: &str = "550e8400-e29b-41d4-a716-446655440000";

    #[test]
    fn test_okta_platform_admin_role() {
        let mut claims = base_claims(VALID_UUID);
        claims.groups = Some(vec!["platform-admin".to_string()]);
        let identity = claims.to_identity().expect("should parse");
        assert!(identity.roles.contains(&CaveRole::PlatformAdmin));
    }

    #[test]
    fn test_okta_tenant_admin_role() {
        let mut claims = base_claims(VALID_UUID);
        claims.groups = Some(vec!["tenant-admin".to_string()]);
        let identity = claims.to_identity().expect("should parse");
        assert!(identity.roles.contains(&CaveRole::TenantAdmin));
    }

    #[test]
    fn test_okta_tenant_developer_role() {
        let mut claims = base_claims(VALID_UUID);
        claims.groups = Some(vec!["tenant-developer".to_string()]);
        let identity = claims.to_identity().expect("should parse");
        assert!(identity.roles.contains(&CaveRole::TenantDeveloper));
    }

    #[test]
    fn test_keycloak_role_mapping() {
        let mut claims = base_claims(VALID_UUID);
        claims.realm_access = Some(RealmAccess {
            roles: vec!["cave-platform-admin".to_string()],
        });
        let identity = claims.to_identity().expect("should parse");
        assert!(identity.roles.contains(&CaveRole::PlatformAdmin));
    }

    #[test]
    fn test_valid_uuid_cave_uid() {
        let claims = base_claims(VALID_UUID);
        let result = claims.to_identity();
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_uuid_cave_uid() {
        let claims = base_claims("not-a-uuid");
        let result = claims.to_identity();
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_groups_no_roles() {
        let mut claims = base_claims(VALID_UUID);
        claims.groups = Some(vec![]);
        let identity = claims.to_identity().expect("should parse");
        assert!(identity.roles.is_empty());
    }
}
