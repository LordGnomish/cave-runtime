// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared RBAC + tenant scoping guard for cave-portal-api routes.
//!
//! Every route handler that touches tenant-owned data must call
//! [`Guard::authorize`] before reading state. The guard implements *default
//! deny*: a request with no resolved persona, no tenant id, or insufficient
//! permissions is rejected with [`GuardError`] before any data is fetched.
//!
//! There are three personas — Tenant (member of a tenant), Operator (cluster
//! operator who manages many tenants but is not a customer), and Admin
//! (platform staff). Each route declares the personas it allows, plus an
//! optional set of fine-grained roles (`secrets:write`, `deployments:rollout`).

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Persona {
    /// Member of a tenant (developer, app owner). Always tenant-scoped.
    Tenant,
    /// Cluster operator — sees many tenants, never owns data.
    Operator,
    /// Platform staff — system-wide admin.
    Admin,
}

impl Persona {
    pub fn as_str(&self) -> &'static str {
        match self {
            Persona::Tenant => "tenant",
            Persona::Operator => "operator",
            Persona::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tenant" => Some(Persona::Tenant),
            "operator" => Some(Persona::Operator),
            "admin" => Some(Persona::Admin),
            _ => None,
        }
    }

    /// Operators and admins can act across tenants; tenant-persona is
    /// restricted to its own tenant id.
    pub fn is_cross_tenant(&self) -> bool {
        matches!(self, Persona::Operator | Persona::Admin)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub subject: String,
    pub persona: Persona,
    pub tenant: Option<String>,
    pub roles: HashSet<String>,
}

impl Principal {
    pub fn new(subject: impl Into<String>, persona: Persona) -> Self {
        Self {
            subject: subject.into(),
            persona,
            tenant: None,
            roles: HashSet::new(),
        }
    }

    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.roles.insert(role.into());
        self
    }

    pub fn with_roles<I, S>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for r in roles {
            self.roles.insert(r.into());
        }
        self
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.roles.contains(role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    #[error("authentication required")]
    Anonymous,
    #[error("persona {got:?} not allowed; expected one of {allowed:?}")]
    PersonaForbidden { got: Persona, allowed: Vec<Persona> },
    #[error("missing role: {0}")]
    MissingRole(String),
    #[error("tenant id required")]
    TenantRequired,
    #[error(
        "cross-tenant access denied: principal tenant {principal:?} does not match {requested:?}"
    )]
    TenantMismatch {
        principal: String,
        requested: String,
    },
}

/// Per-handler guard configuration.
#[derive(Debug, Clone)]
pub struct Guard {
    pub allowed_personas: Vec<Persona>,
    pub required_role: Option<String>,
    /// If `true`, the request must carry a tenant id, and (when the principal
    /// is a `Persona::Tenant`) the principal's tenant must match.
    pub requires_tenant: bool,
}

impl Guard {
    pub fn tenant_only(role: Option<&str>) -> Self {
        Self {
            allowed_personas: vec![Persona::Tenant, Persona::Admin],
            required_role: role.map(String::from),
            requires_tenant: true,
        }
    }

    pub fn operator_only() -> Self {
        Self {
            allowed_personas: vec![Persona::Operator, Persona::Admin],
            required_role: None,
            requires_tenant: false,
        }
    }

    pub fn admin_only() -> Self {
        Self {
            allowed_personas: vec![Persona::Admin],
            required_role: None,
            requires_tenant: false,
        }
    }

    pub fn cross_persona(role: Option<&str>) -> Self {
        Self {
            allowed_personas: vec![Persona::Tenant, Persona::Operator, Persona::Admin],
            required_role: role.map(String::from),
            requires_tenant: true,
        }
    }

    /// Default-deny check.
    pub fn authorize(
        &self,
        principal: Option<&Principal>,
        requested_tenant: Option<&str>,
    ) -> Result<(), GuardError> {
        let p = principal.ok_or(GuardError::Anonymous)?;
        if !self.allowed_personas.contains(&p.persona) {
            return Err(GuardError::PersonaForbidden {
                got: p.persona,
                allowed: self.allowed_personas.clone(),
            });
        }
        if let Some(role) = &self.required_role {
            if !p.has_role(role) {
                return Err(GuardError::MissingRole(role.clone()));
            }
        }
        if self.requires_tenant {
            let req = requested_tenant.ok_or(GuardError::TenantRequired)?;
            if p.persona == Persona::Tenant {
                let pt = p.tenant.as_deref().ok_or(GuardError::TenantRequired)?;
                if pt != req {
                    return Err(GuardError::TenantMismatch {
                        principal: pt.to_string(),
                        requested: req.to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant_principal(tenant: &str) -> Principal {
        Principal::new("u-1", Persona::Tenant).with_tenant(tenant)
    }

    fn operator_principal() -> Principal {
        Principal::new("ops-1", Persona::Operator)
    }

    fn admin_principal() -> Principal {
        Principal::new("admin-1", Persona::Admin)
    }

    #[test]
    fn persona_as_str() {
        assert_eq!(Persona::Tenant.as_str(), "tenant");
        assert_eq!(Persona::Operator.as_str(), "operator");
        assert_eq!(Persona::Admin.as_str(), "admin");
    }

    #[test]
    fn persona_parse_known() {
        assert_eq!(Persona::parse("tenant"), Some(Persona::Tenant));
        assert_eq!(Persona::parse("operator"), Some(Persona::Operator));
        assert_eq!(Persona::parse("admin"), Some(Persona::Admin));
    }

    #[test]
    fn persona_parse_unknown() {
        assert_eq!(Persona::parse("god"), None);
        assert_eq!(Persona::parse(""), None);
    }

    #[test]
    fn persona_is_cross_tenant() {
        assert!(!Persona::Tenant.is_cross_tenant());
        assert!(Persona::Operator.is_cross_tenant());
        assert!(Persona::Admin.is_cross_tenant());
    }

    #[test]
    fn principal_default_no_tenant_no_roles() {
        let p = Principal::new("u", Persona::Tenant);
        assert!(p.tenant.is_none());
        assert!(p.roles.is_empty());
    }

    #[test]
    fn principal_with_tenant() {
        let p = Principal::new("u", Persona::Tenant).with_tenant("acme");
        assert_eq!(p.tenant.as_deref(), Some("acme"));
    }

    #[test]
    fn principal_with_roles_dedupes() {
        let p = Principal::new("u", Persona::Tenant)
            .with_role("a")
            .with_role("a");
        assert_eq!(p.roles.len(), 1);
    }

    #[test]
    fn principal_has_role() {
        let p = Principal::new("u", Persona::Tenant).with_role("admin");
        assert!(p.has_role("admin"));
        assert!(!p.has_role("viewer"));
    }

    #[test]
    fn principal_with_roles_iter() {
        let p = Principal::new("u", Persona::Tenant).with_roles(["a", "b", "c"]);
        assert_eq!(p.roles.len(), 3);
    }

    #[test]
    fn guard_anonymous_request_denied() {
        let g = Guard::tenant_only(None);
        assert_eq!(g.authorize(None, Some("acme")), Err(GuardError::Anonymous));
    }

    #[test]
    fn guard_tenant_only_allows_tenant_persona() {
        let g = Guard::tenant_only(None);
        assert!(
            g.authorize(Some(&tenant_principal("acme")), Some("acme"))
                .is_ok()
        );
    }

    #[test]
    fn guard_tenant_only_allows_admin_too() {
        let g = Guard::tenant_only(None);
        assert!(g.authorize(Some(&admin_principal()), Some("acme")).is_ok());
    }

    #[test]
    fn guard_tenant_only_rejects_operator() {
        let g = Guard::tenant_only(None);
        let err = g
            .authorize(Some(&operator_principal()), Some("acme"))
            .unwrap_err();
        assert!(matches!(err, GuardError::PersonaForbidden { .. }));
    }

    #[test]
    fn guard_operator_only_rejects_tenant() {
        let g = Guard::operator_only();
        let err = g
            .authorize(Some(&tenant_principal("acme")), None)
            .unwrap_err();
        assert!(matches!(err, GuardError::PersonaForbidden { .. }));
    }

    #[test]
    fn guard_operator_only_allows_operator() {
        let g = Guard::operator_only();
        assert!(g.authorize(Some(&operator_principal()), None).is_ok());
    }

    #[test]
    fn guard_admin_only_rejects_others() {
        let g = Guard::admin_only();
        assert!(g.authorize(Some(&tenant_principal("acme")), None).is_err());
        assert!(g.authorize(Some(&operator_principal()), None).is_err());
    }

    #[test]
    fn guard_admin_only_allows_admin() {
        let g = Guard::admin_only();
        assert!(g.authorize(Some(&admin_principal()), None).is_ok());
    }

    #[test]
    fn guard_required_role_enforced() {
        let g = Guard::tenant_only(Some("secrets:write"));
        let p = tenant_principal("acme");
        let err = g.authorize(Some(&p), Some("acme")).unwrap_err();
        assert!(matches!(err, GuardError::MissingRole(r) if r == "secrets:write"));
    }

    #[test]
    fn guard_required_role_satisfied() {
        let g = Guard::tenant_only(Some("secrets:write"));
        let p = tenant_principal("acme").with_role("secrets:write");
        assert!(g.authorize(Some(&p), Some("acme")).is_ok());
    }

    #[test]
    fn guard_requires_tenant_errors_when_missing() {
        let g = Guard::tenant_only(None);
        let p = tenant_principal("acme");
        let err = g.authorize(Some(&p), None).unwrap_err();
        assert_eq!(err, GuardError::TenantRequired);
    }

    #[test]
    fn guard_tenant_persona_must_match_requested_tenant() {
        let g = Guard::tenant_only(None);
        let p = tenant_principal("acme");
        let err = g.authorize(Some(&p), Some("globex")).unwrap_err();
        assert!(matches!(err, GuardError::TenantMismatch { .. }));
    }

    #[test]
    fn guard_admin_can_access_any_tenant() {
        let g = Guard::tenant_only(None);
        // Admin persona is cross-tenant — not constrained by p.tenant
        assert!(
            g.authorize(Some(&admin_principal()), Some("globex"))
                .is_ok()
        );
    }

    #[test]
    fn guard_cross_persona_allows_all_three() {
        let g = Guard::cross_persona(None);
        assert!(
            g.authorize(Some(&tenant_principal("acme")), Some("acme"))
                .is_ok()
        );
        assert!(
            g.authorize(Some(&operator_principal()), Some("acme"))
                .is_ok()
        );
        assert!(g.authorize(Some(&admin_principal()), Some("acme")).is_ok());
    }

    #[test]
    fn guard_cross_persona_still_requires_tenant() {
        let g = Guard::cross_persona(None);
        assert_eq!(
            g.authorize(Some(&operator_principal()), None).unwrap_err(),
            GuardError::TenantRequired
        );
    }

    #[test]
    fn guard_role_required_short_circuits_after_persona_check() {
        let g = Guard::admin_only();
        let g_with_role = Guard {
            required_role: Some("nuke".into()),
            ..g
        };
        let err = g_with_role
            .authorize(Some(&tenant_principal("acme")), None)
            .unwrap_err();
        assert!(matches!(err, GuardError::PersonaForbidden { .. }));
    }
}
