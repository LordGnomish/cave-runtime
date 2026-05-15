// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant context — every portal request is scoped to a tenant id.
//!
//! Default-deny: a plugin that declares [`super::Scope::Tenant`] cannot be
//! rendered when no tenant is resolved. Public pages explicitly opt out via
//! [`super::Scope::Public`].

use std::collections::HashMap;
use std::fmt;

/// A tenant identifier.
///
/// Tenants are identified by a stable string slug (e.g. `acme`, `globex`).
/// Constructing a [`TenantId`] validates the slug — empty strings, uppercase
/// letters and forbidden characters are rejected so the id can be used in URLs
/// and storage keys without re-escaping.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(String);

impl TenantId {
    pub fn new(slug: impl Into<String>) -> Result<Self, TenantError> {
        let slug: String = slug.into();
        if slug.is_empty() {
            return Err(TenantError::Empty);
        }
        if slug.len() > 64 {
            return Err(TenantError::TooLong);
        }
        for ch in slug.chars() {
            let ok = ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || ch == '-'
                || ch == '_';
            if !ok {
                return Err(TenantError::InvalidChar(ch));
            }
        }
        if slug.starts_with('-') || slug.ends_with('-') {
            return Err(TenantError::HyphenBoundary);
        }
        Ok(Self(slug))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TenantError {
    #[error("tenant slug must not be empty")]
    Empty,
    #[error("tenant slug too long (max 64 chars)")]
    TooLong,
    #[error("tenant slug contains invalid character: {0:?}")]
    InvalidChar(char),
    #[error("tenant slug must not start or end with a hyphen")]
    HyphenBoundary,
    #[error("tenant {0:?} is not registered")]
    Unknown(String),
    #[error("no tenant in context (default-deny)")]
    Missing,
}

/// Per-request tenant resolution.
///
/// A [`TenantContext`] tracks the *current* tenant for the in-flight request
/// and the *roster* of tenants known to the portal. The roster is used to
/// validate slugs at the edge — an unknown tenant id is rejected before any
/// plugin code runs.
#[derive(Debug, Clone, Default)]
pub struct TenantContext {
    current: Option<TenantId>,
    roster: HashMap<TenantId, TenantInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantInfo {
    pub id: TenantId,
    pub display_name: String,
    pub plan: TenantPlan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantPlan {
    Free,
    Pro,
    Enterprise,
}

impl TenantContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, info: TenantInfo) {
        self.roster.insert(info.id.clone(), info);
    }

    pub fn lookup(&self, id: &TenantId) -> Option<&TenantInfo> {
        self.roster.get(id)
    }

    pub fn roster(&self) -> impl Iterator<Item = &TenantInfo> {
        self.roster.values()
    }

    pub fn count(&self) -> usize {
        self.roster.len()
    }

    /// Set the current tenant by id; rejects unknown tenants.
    pub fn set_current(&mut self, id: TenantId) -> Result<(), TenantError> {
        if !self.roster.contains_key(&id) {
            return Err(TenantError::Unknown(id.into_inner()));
        }
        self.current = Some(id);
        Ok(())
    }

    pub fn clear_current(&mut self) {
        self.current = None;
    }

    pub fn current(&self) -> Option<&TenantId> {
        self.current.as_ref()
    }

    /// Default-deny accessor: returns [`TenantError::Missing`] when no tenant
    /// is set. Plugins should call this rather than [`Self::current`] when the
    /// request must be tenant-scoped.
    pub fn require(&self) -> Result<&TenantId, TenantError> {
        self.current.as_ref().ok_or(TenantError::Missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(slug: &str) -> TenantId {
        TenantId::new(slug).unwrap()
    }

    #[test]
    fn tenant_id_accepts_lowercase_slug() {
        assert!(TenantId::new("acme").is_ok());
        assert!(TenantId::new("globex").is_ok());
        assert!(TenantId::new("abc-123").is_ok());
        assert!(TenantId::new("a_b").is_ok());
    }

    #[test]
    fn tenant_id_accepts_digits() {
        assert!(TenantId::new("tenant-001").is_ok());
        assert!(TenantId::new("0123").is_ok());
    }

    #[test]
    fn tenant_id_rejects_empty() {
        assert_eq!(TenantId::new(""), Err(TenantError::Empty));
    }

    #[test]
    fn tenant_id_rejects_uppercase() {
        match TenantId::new("Acme") {
            Err(TenantError::InvalidChar('A')) => {}
            other => panic!("expected uppercase rejection, got {:?}", other),
        }
    }

    #[test]
    fn tenant_id_rejects_spaces() {
        match TenantId::new("hello world") {
            Err(TenantError::InvalidChar(' ')) => {}
            other => panic!("expected space rejection, got {:?}", other),
        }
    }

    #[test]
    fn tenant_id_rejects_path_traversal() {
        assert!(matches!(TenantId::new("../etc"), Err(TenantError::InvalidChar('.'))));
        assert!(matches!(TenantId::new("a/b"), Err(TenantError::InvalidChar('/'))));
    }

    #[test]
    fn tenant_id_rejects_leading_hyphen() {
        assert_eq!(TenantId::new("-acme"), Err(TenantError::HyphenBoundary));
    }

    #[test]
    fn tenant_id_rejects_trailing_hyphen() {
        assert_eq!(TenantId::new("acme-"), Err(TenantError::HyphenBoundary));
    }

    #[test]
    fn tenant_id_rejects_too_long() {
        let too_long = "a".repeat(65);
        assert_eq!(TenantId::new(too_long), Err(TenantError::TooLong));
    }

    #[test]
    fn tenant_id_accepts_64_char_boundary() {
        let exact = "a".repeat(64);
        assert!(TenantId::new(exact).is_ok());
    }

    #[test]
    fn tenant_id_round_trips_as_str() {
        let id = t("acme");
        assert_eq!(id.as_str(), "acme");
        assert_eq!(id.to_string(), "acme");
    }

    #[test]
    fn tenant_id_into_inner_returns_owned_string() {
        let id = t("globex");
        let s = id.into_inner();
        assert_eq!(s, "globex");
    }

    #[test]
    fn tenant_context_default_has_no_current() {
        let ctx = TenantContext::new();
        assert!(ctx.current().is_none());
    }

    #[test]
    fn tenant_context_default_require_errors() {
        let ctx = TenantContext::new();
        assert_eq!(ctx.require().err(), Some(TenantError::Missing));
    }

    #[test]
    fn tenant_context_register_makes_lookup_succeed() {
        let mut ctx = TenantContext::new();
        ctx.register(TenantInfo {
            id: t("acme"),
            display_name: "Acme Corp".into(),
            plan: TenantPlan::Pro,
        });
        assert!(ctx.lookup(&t("acme")).is_some());
        assert_eq!(ctx.lookup(&t("acme")).unwrap().display_name, "Acme Corp");
    }

    #[test]
    fn tenant_context_count_tracks_roster() {
        let mut ctx = TenantContext::new();
        assert_eq!(ctx.count(), 0);
        ctx.register(TenantInfo {
            id: t("a"),
            display_name: "A".into(),
            plan: TenantPlan::Free,
        });
        ctx.register(TenantInfo {
            id: t("b"),
            display_name: "B".into(),
            plan: TenantPlan::Free,
        });
        assert_eq!(ctx.count(), 2);
    }

    #[test]
    fn tenant_context_register_overwrites_existing() {
        let mut ctx = TenantContext::new();
        ctx.register(TenantInfo {
            id: t("acme"),
            display_name: "old".into(),
            plan: TenantPlan::Free,
        });
        ctx.register(TenantInfo {
            id: t("acme"),
            display_name: "new".into(),
            plan: TenantPlan::Enterprise,
        });
        assert_eq!(ctx.count(), 1);
        assert_eq!(ctx.lookup(&t("acme")).unwrap().display_name, "new");
        assert_eq!(ctx.lookup(&t("acme")).unwrap().plan, TenantPlan::Enterprise);
    }

    #[test]
    fn tenant_context_set_current_rejects_unknown() {
        let mut ctx = TenantContext::new();
        let err = ctx.set_current(t("ghost")).unwrap_err();
        assert!(matches!(err, TenantError::Unknown(s) if s == "ghost"));
    }

    #[test]
    fn tenant_context_set_current_accepts_known() {
        let mut ctx = TenantContext::new();
        ctx.register(TenantInfo {
            id: t("acme"),
            display_name: "Acme".into(),
            plan: TenantPlan::Pro,
        });
        ctx.set_current(t("acme")).unwrap();
        assert_eq!(ctx.current(), Some(&t("acme")));
    }

    #[test]
    fn tenant_context_require_succeeds_when_set() {
        let mut ctx = TenantContext::new();
        ctx.register(TenantInfo {
            id: t("acme"),
            display_name: "Acme".into(),
            plan: TenantPlan::Free,
        });
        ctx.set_current(t("acme")).unwrap();
        assert_eq!(ctx.require().unwrap(), &t("acme"));
    }

    #[test]
    fn tenant_context_clear_current_resets_to_default_deny() {
        let mut ctx = TenantContext::new();
        ctx.register(TenantInfo {
            id: t("acme"),
            display_name: "Acme".into(),
            plan: TenantPlan::Free,
        });
        ctx.set_current(t("acme")).unwrap();
        ctx.clear_current();
        assert!(ctx.require().is_err());
    }

    #[test]
    fn tenant_context_roster_returns_all_infos() {
        let mut ctx = TenantContext::new();
        for slug in ["a", "b", "c"] {
            ctx.register(TenantInfo {
                id: t(slug),
                display_name: slug.to_uppercase(),
                plan: TenantPlan::Free,
            });
        }
        let names: std::collections::HashSet<String> =
            ctx.roster().map(|i| i.display_name.clone()).collect();
        assert!(names.contains("A"));
        assert!(names.contains("B"));
        assert!(names.contains("C"));
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn tenant_plan_equality() {
        assert_eq!(TenantPlan::Pro, TenantPlan::Pro);
        assert_ne!(TenantPlan::Pro, TenantPlan::Free);
    }

    #[test]
    fn tenant_error_messages_are_descriptive() {
        let e = TenantError::InvalidChar('!');
        assert!(e.to_string().contains("invalid"));
        let e = TenantError::Missing;
        assert!(e.to_string().contains("default-deny"));
        let e = TenantError::Unknown("foo".into());
        assert!(e.to_string().contains("foo"));
    }
}
