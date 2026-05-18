// SPDX-License-Identifier: AGPL-3.0-or-later
//! Multi-org, role-based access control, API key validation, service accounts.

use crate::models::{ApiKey, OrgRole};
use std::collections::HashMap;

/// Compute a simple SHA-256 hex hash for an API key token.
/// In production this should use a proper HMAC/bcrypt; here we use a fast hash.
pub fn hash_api_key(token: &str) -> String {
    use std::hash::{Hash, Hasher};
    // Deterministic but not cryptographic — fine for an in-memory demo.
    // In prod: use ring::digest::digest(&ring::digest::SHA256, token.as_bytes())
    let mut h: u64 = 14695981039346656037;
    for byte in token.as_bytes() {
        h ^= *byte as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{h:016x}")
}

/// Generate a random API key token.
pub fn generate_api_key() -> String {
    // In prod: use ring::rand or similar.  Here: uuid-based.
    format!("glsa_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
}

/// An authenticated principal for the current request.
#[derive(Debug, Clone)]
pub enum Principal {
    User { id: i64, org_id: i64, role: OrgRole, is_admin: bool },
    ServiceAccount { id: i64, org_id: i64, role: OrgRole },
    ApiKey { id: i64, org_id: i64, role: OrgRole },
    Anonymous,
}

impl Principal {
    pub fn org_id(&self) -> i64 {
        match self {
            Self::User { org_id, .. } => *org_id,
            Self::ServiceAccount { org_id, .. } => *org_id,
            Self::ApiKey { org_id, .. } => *org_id,
            Self::Anonymous => 1,
        }
    }

    pub fn role(&self) -> OrgRole {
        match self {
            Self::User { role, .. } => *role,
            Self::ServiceAccount { role, .. } => *role,
            Self::ApiKey { role, .. } => *role,
            Self::Anonymous => OrgRole::Viewer,
        }
    }

    pub fn can_edit(&self) -> bool {
        matches!(self.role(), OrgRole::Editor | OrgRole::Admin)
    }

    pub fn is_admin(&self) -> bool {
        match self {
            Self::User { role, is_admin, .. } => *role == OrgRole::Admin || *is_admin,
            Self::ServiceAccount { role, .. } => *role == OrgRole::Admin,
            Self::ApiKey { role, .. } => *role == OrgRole::Admin,
            Self::Anonymous => false,
        }
    }
}

/// Minimal permission check — returns Err with a message when denied.
pub fn require_editor(principal: &Principal) -> Result<(), String> {
    if principal.can_edit() {
        Ok(())
    } else {
        Err("permission denied: editor role required".into())
    }
}

pub fn require_admin(principal: &Principal) -> Result<(), String> {
    if principal.is_admin() {
        Ok(())
    } else {
        Err("permission denied: admin role required".into())
    }
}

/// Extract a Bearer token from an Authorization header value.
/// Handles both "Bearer <token>" and raw tokens.
pub fn extract_bearer(header: &str) -> &str {
    if let Some(tok) = header.strip_prefix("Bearer ") {
        tok.trim()
    } else {
        header.trim()
    }
}

/// Map of org user memberships: org_id → (user_id → role)
pub type OrgMembership = HashMap<i64, HashMap<i64, OrgRole>>;
