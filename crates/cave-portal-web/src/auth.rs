//! Auth context — resolves the principal for the current request.
//!
//! Plugins inspect the [`AuthContext`] to decide whether the caller may render
//! a page or panel. The actual identity provider is a `cave-auth` concern; this
//! module only models *the result* (an [`Identity`] plus a set of granted
//! roles) and offers ergonomic checks (`has_role`, `has_any_role`,
//! `require_role`).

use std::collections::HashSet;

/// A resolved identity for the in-flight request.
///
/// Identities are issued by the auth layer (JWT, session cookie, mTLS); this
/// crate is intentionally agnostic about *how* the identity was obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub subject: String,
    pub display_name: String,
    pub email: Option<String>,
    pub roles: HashSet<String>,
}

impl Identity {
    pub fn new(subject: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            display_name: display_name.into(),
            email: None,
            roles: HashSet::new(),
        }
    }

    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
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

    pub fn has_any_role<'a, I: IntoIterator<Item = &'a str>>(&self, roles: I) -> bool {
        roles.into_iter().any(|r| self.has_role(r))
    }

    pub fn has_all_roles<'a, I: IntoIterator<Item = &'a str>>(&self, roles: I) -> bool {
        roles.into_iter().all(|r| self.has_role(r))
    }
}

/// Per-request auth state. Either a resolved [`Identity`] is present or the
/// caller is anonymous; default-deny pages must reject the latter.
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    identity: Option<Identity>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("no identity in context")]
    Anonymous,
    #[error("missing role: {0}")]
    MissingRole(String),
    #[error("missing any of: {0:?}")]
    MissingAnyRole(Vec<String>),
}

impl AuthContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn anonymous() -> Self {
        Self::default()
    }

    pub fn with_identity(identity: Identity) -> Self {
        Self { identity: Some(identity) }
    }

    pub fn set_identity(&mut self, identity: Identity) {
        self.identity = Some(identity);
    }

    pub fn clear(&mut self) {
        self.identity = None;
    }

    pub fn identity(&self) -> Option<&Identity> {
        self.identity.as_ref()
    }

    pub fn is_anonymous(&self) -> bool {
        self.identity.is_none()
    }

    pub fn require(&self) -> Result<&Identity, AuthError> {
        self.identity.as_ref().ok_or(AuthError::Anonymous)
    }

    pub fn require_role(&self, role: &str) -> Result<&Identity, AuthError> {
        let id = self.require()?;
        if !id.has_role(role) {
            return Err(AuthError::MissingRole(role.to_string()));
        }
        Ok(id)
    }

    pub fn require_any_role<'a, I>(&self, roles: I) -> Result<&Identity, AuthError>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let collected: Vec<&str> = roles.into_iter().collect();
        let id = self.require()?;
        if !id.has_any_role(collected.iter().copied()) {
            return Err(AuthError::MissingAnyRole(
                collected.into_iter().map(String::from).collect(),
            ));
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_new_has_no_email_no_roles() {
        let id = Identity::new("sub", "Alice");
        assert_eq!(id.subject, "sub");
        assert_eq!(id.display_name, "Alice");
        assert!(id.email.is_none());
        assert!(id.roles.is_empty());
    }

    #[test]
    fn identity_with_email_sets_email() {
        let id = Identity::new("sub", "Alice").with_email("a@x.com");
        assert_eq!(id.email.as_deref(), Some("a@x.com"));
    }

    #[test]
    fn identity_with_role_adds_role() {
        let id = Identity::new("sub", "Alice").with_role("admin");
        assert!(id.has_role("admin"));
        assert!(!id.has_role("viewer"));
    }

    #[test]
    fn identity_with_roles_adds_many() {
        let id = Identity::new("sub", "Alice").with_roles(["admin", "viewer"]);
        assert!(id.has_role("admin"));
        assert!(id.has_role("viewer"));
        assert_eq!(id.roles.len(), 2);
    }

    #[test]
    fn identity_with_role_dedupes() {
        let id = Identity::new("sub", "Alice")
            .with_role("admin")
            .with_role("admin");
        assert_eq!(id.roles.len(), 1);
    }

    #[test]
    fn identity_has_any_role_returns_true_on_match() {
        let id = Identity::new("s", "A").with_role("admin");
        assert!(id.has_any_role(["viewer", "admin"]));
    }

    #[test]
    fn identity_has_any_role_returns_false_when_none_match() {
        let id = Identity::new("s", "A").with_role("admin");
        assert!(!id.has_any_role(["viewer", "editor"]));
    }

    #[test]
    fn identity_has_all_roles_requires_every_role() {
        let id = Identity::new("s", "A").with_roles(["a", "b"]);
        assert!(id.has_all_roles(["a", "b"]));
        assert!(!id.has_all_roles(["a", "b", "c"]));
    }

    #[test]
    fn auth_context_default_is_anonymous() {
        let ctx = AuthContext::new();
        assert!(ctx.is_anonymous());
        assert!(ctx.identity().is_none());
    }

    #[test]
    fn auth_context_anonymous_helper_matches_default() {
        let a = AuthContext::anonymous();
        let b = AuthContext::default();
        assert_eq!(a.is_anonymous(), b.is_anonymous());
    }

    #[test]
    fn auth_context_require_errors_when_anonymous() {
        let ctx = AuthContext::new();
        assert_eq!(ctx.require().err(), Some(AuthError::Anonymous));
    }

    #[test]
    fn auth_context_with_identity_holds_principal() {
        let id = Identity::new("u-1", "User One");
        let ctx = AuthContext::with_identity(id.clone());
        assert!(!ctx.is_anonymous());
        assert_eq!(ctx.identity(), Some(&id));
    }

    #[test]
    fn auth_context_require_returns_identity_when_set() {
        let id = Identity::new("u", "U");
        let ctx = AuthContext::with_identity(id.clone());
        assert_eq!(ctx.require().unwrap(), &id);
    }

    #[test]
    fn auth_context_require_role_succeeds_with_role() {
        let id = Identity::new("u", "U").with_role("admin");
        let ctx = AuthContext::with_identity(id);
        assert!(ctx.require_role("admin").is_ok());
    }

    #[test]
    fn auth_context_require_role_fails_without_role() {
        let id = Identity::new("u", "U").with_role("viewer");
        let ctx = AuthContext::with_identity(id);
        let err = ctx.require_role("admin").unwrap_err();
        assert!(matches!(err, AuthError::MissingRole(r) if r == "admin"));
    }

    #[test]
    fn auth_context_require_role_fails_when_anonymous() {
        let ctx = AuthContext::new();
        assert_eq!(ctx.require_role("admin").err(), Some(AuthError::Anonymous));
    }

    #[test]
    fn auth_context_require_any_role_succeeds_with_any_match() {
        let id = Identity::new("u", "U").with_role("editor");
        let ctx = AuthContext::with_identity(id);
        assert!(ctx.require_any_role(["admin", "editor"]).is_ok());
    }

    #[test]
    fn auth_context_require_any_role_fails_when_none_match() {
        let id = Identity::new("u", "U").with_role("viewer");
        let ctx = AuthContext::with_identity(id);
        let err = ctx.require_any_role(["admin", "editor"]).unwrap_err();
        match err {
            AuthError::MissingAnyRole(rs) => {
                assert!(rs.contains(&"admin".to_string()));
                assert!(rs.contains(&"editor".to_string()));
            }
            other => panic!("expected MissingAnyRole, got {:?}", other),
        }
    }

    #[test]
    fn auth_context_set_identity_replaces() {
        let mut ctx = AuthContext::new();
        ctx.set_identity(Identity::new("a", "A"));
        ctx.set_identity(Identity::new("b", "B"));
        assert_eq!(ctx.identity().unwrap().subject, "b");
    }

    #[test]
    fn auth_context_clear_resets() {
        let mut ctx = AuthContext::with_identity(Identity::new("a", "A"));
        ctx.clear();
        assert!(ctx.is_anonymous());
    }

    #[test]
    fn auth_error_displays_role() {
        let e = AuthError::MissingRole("admin".into());
        assert!(e.to_string().contains("admin"));
    }

    #[test]
    fn auth_error_displays_any_role_list() {
        let e = AuthError::MissingAnyRole(vec!["a".into(), "b".into()]);
        let s = e.to_string();
        assert!(s.contains("a"));
        assert!(s.contains("b"));
    }
}
