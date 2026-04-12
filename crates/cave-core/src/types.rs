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
/// Hierarchy: PlatformAdmin > ModuleAdmin ≥ TenantAdmin > Developer/TenantDeveloper > Auditor > Viewer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaveRole {
    // ── Existing (kept for backward compat) ──
    PlatformAdmin,
    PlatformViewer,
    TenantAdmin,
    TenantDeveloper,
    TenantViewer,
    // ── New fine-grained roles ──
    /// Admin scoped to a single module (e.g., cave-flags admin)
    ModuleAdmin,
    /// Developer — read + write, no destructive/admin actions
    Developer,
    /// Read-only access to audit logs and security events
    Auditor,
}

/// Module permission check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub module: String,
    pub action: String, // e.g., "flags:write", "vulns:triage", "scan:admin"
}

impl CaveIdentity {
    /// Check if this identity has the required permission.
    /// Falls back to role-based evaluation when no explicit permissions are set.
    pub fn has_permission(&self, module: &str, action: &str) -> bool {
        // Explicit fine-grained permissions take priority
        let perm = format!("{module}:{action}");
        let wildcard = format!("{module}:*");
        if self.permissions.contains(&perm)
            || self.permissions.contains(&wildcard)
            || self.permissions.contains(&"*".to_string())
        {
            return true;
        }

        // Fall back to coarse role evaluation
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
    /// Source project (e.g., "unleash", "defectdojo")
    pub project: String,
    /// Upstream version where feature appeared
    pub upstream_version: String,
    /// GitHub issue/PR URL in upstream
    pub upstream_url: String,
    /// Our triage decision
    pub triage: UpstreamTriage,
    /// cave-runtime version where implemented (if adopted)
    pub implemented_in: Option<String>,
    /// Evaluation notes
    pub notes: String,
    /// When we detected this
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum UpstreamTriage {
    /// Implement in cave-runtime
    Adopt,
    /// Track but don't implement yet
    Watch,
    /// Not relevant to our use case
    Skip,
}
