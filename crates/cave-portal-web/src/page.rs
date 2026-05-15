// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pages — the unit a plugin contributes to the web shell.
//!
//! A [`Page`] is a metadata + render-fn pair. The metadata declares the
//! page's [`Scope`] (Public, Tenant-scoped, or Admin), the route path,
//! a display title, and an icon glyph. The render fn turns a
//! [`PageRequest`] into a [`PageResponse`] (the inner HTML body that the
//! [`crate::AppShell`] wraps).

use crate::auth::{AuthContext, AuthError};
use crate::tenant::{TenantContext, TenantError};
use std::sync::Arc;

/// Visibility scope of a page.
///
/// - `Public`: no auth, no tenant required.
/// - `Tenant`: auth + tenant required (default-deny).
/// - `Admin`: requires the listed role(s) regardless of tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Public,
    Tenant,
    Admin(Vec<String>),
}

impl Scope {
    pub fn is_public(&self) -> bool {
        matches!(self, Scope::Public)
    }
    pub fn is_tenant(&self) -> bool {
        matches!(self, Scope::Tenant)
    }
    pub fn is_admin(&self) -> bool {
        matches!(self, Scope::Admin(_))
    }
    pub fn required_roles(&self) -> &[String] {
        match self {
            Scope::Admin(roles) => roles.as_slice(),
            _ => &[],
        }
    }
}

/// Inputs to a page render. Holds the resolved auth and tenant context, plus
/// any path parameters captured by the router.
#[derive(Debug, Clone)]
pub struct PageRequest {
    pub path: String,
    pub auth: AuthContext,
    pub tenant: TenantContext,
    pub params: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
}

impl PageRequest {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            auth: AuthContext::new(),
            tenant: TenantContext::new(),
            params: Vec::new(),
            query: Vec::new(),
        }
    }

    pub fn with_auth(mut self, auth: AuthContext) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_tenant(mut self, tenant: TenantContext) -> Self {
        self.tenant = tenant;
        self
    }

    pub fn with_param(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.params.push((k.into(), v.into()));
        self
    }

    pub fn with_query(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.query.push((k.into(), v.into()));
        self
    }

    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn query_value(&self, key: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Output of a page render — inner HTML body + status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageResponse {
    pub status: u16,
    pub body: String,
    pub title: String,
}

impl PageResponse {
    pub fn ok(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self { status: 200, title: title.into(), body: body.into() }
    }

    pub fn not_found(title: impl Into<String>) -> Self {
        Self { status: 404, title: title.into(), body: String::new() }
    }

    pub fn forbidden(title: impl Into<String>) -> Self {
        Self { status: 403, title: title.into(), body: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    #[error("auth: {0}")]
    Auth(#[from] AuthError),
    #[error("tenant: {0}")]
    Tenant(#[from] TenantError),
    #[error("page render failed: {0}")]
    Render(String),
}

pub type RenderFn = Arc<dyn Fn(&PageRequest) -> Result<PageResponse, PageError> + Send + Sync>;

/// A registered page. Cloning a [`Page`] is cheap — the render fn is shared.
#[derive(Clone)]
pub struct Page {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub path: String,
    pub scope: Scope,
    pub category: String,
    pub render: RenderFn,
}

impl Page {
    pub fn builder(id: impl Into<String>, path: impl Into<String>) -> PageBuilder {
        PageBuilder::new(id, path)
    }

    /// Authorize and render the page in one call. Enforces the scope's
    /// invariants before invoking the user-supplied render fn.
    pub fn authorize_and_render(&self, req: &PageRequest) -> Result<PageResponse, PageError> {
        match &self.scope {
            Scope::Public => {}
            Scope::Tenant => {
                req.auth.require()?;
                req.tenant.require()?;
            }
            Scope::Admin(roles) => {
                let role_refs: Vec<&str> = roles.iter().map(String::as_str).collect();
                req.auth.require_any_role(role_refs)?;
            }
        }
        (self.render)(req)
    }
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("icon", &self.icon)
            .field("path", &self.path)
            .field("scope", &self.scope)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

pub struct PageBuilder {
    id: String,
    title: String,
    icon: String,
    path: String,
    scope: Scope,
    category: String,
    render: Option<RenderFn>,
}

impl PageBuilder {
    pub fn new(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: String::new(),
            icon: "default".into(),
            path: path.into(),
            scope: Scope::Tenant,
            category: "general".into(),
            render: None,
        }
    }

    pub fn title(mut self, s: impl Into<String>) -> Self {
        self.title = s.into();
        self
    }

    pub fn icon(mut self, s: impl Into<String>) -> Self {
        self.icon = s.into();
        self
    }

    pub fn scope(mut self, s: Scope) -> Self {
        self.scope = s;
        self
    }

    pub fn category(mut self, s: impl Into<String>) -> Self {
        self.category = s.into();
        self
    }

    pub fn render<F>(mut self, f: F) -> Self
    where
        F: Fn(&PageRequest) -> Result<PageResponse, PageError> + Send + Sync + 'static,
    {
        self.render = Some(Arc::new(f));
        self
    }

    pub fn build(self) -> Page {
        let render = self.render.unwrap_or_else(|| {
            Arc::new(|_req| Ok(PageResponse::ok("Untitled", "<p>placeholder</p>")))
        });
        Page {
            id: self.id,
            title: if self.title.is_empty() { "Untitled".into() } else { self.title },
            icon: self.icon,
            path: self.path,
            scope: self.scope,
            category: self.category,
            render,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Identity;
    use crate::tenant::{TenantId, TenantInfo, TenantPlan};

    fn known_tenant_ctx(slug: &str) -> TenantContext {
        let mut ctx = TenantContext::new();
        let id = TenantId::new(slug).unwrap();
        ctx.register(TenantInfo {
            id: id.clone(),
            display_name: slug.to_string(),
            plan: TenantPlan::Free,
        });
        ctx.set_current(id).unwrap();
        ctx
    }

    fn auth_with(roles: &[&str]) -> AuthContext {
        let mut id = Identity::new("u", "User");
        for r in roles {
            id = id.with_role(*r);
        }
        AuthContext::with_identity(id)
    }

    #[test]
    fn scope_public_classifiers() {
        let s = Scope::Public;
        assert!(s.is_public());
        assert!(!s.is_tenant());
        assert!(!s.is_admin());
        assert!(s.required_roles().is_empty());
    }

    #[test]
    fn scope_tenant_classifiers() {
        let s = Scope::Tenant;
        assert!(s.is_tenant());
        assert!(!s.is_public());
        assert!(!s.is_admin());
    }

    #[test]
    fn scope_admin_lists_required_roles() {
        let s = Scope::Admin(vec!["admin".into(), "owner".into()]);
        assert!(s.is_admin());
        assert_eq!(s.required_roles().len(), 2);
    }

    #[test]
    fn page_request_param_lookup() {
        let r = PageRequest::new("/x").with_param("id", "42");
        assert_eq!(r.param("id"), Some("42"));
        assert!(r.param("missing").is_none());
    }

    #[test]
    fn page_request_query_lookup() {
        let r = PageRequest::new("/x").with_query("page", "3");
        assert_eq!(r.query_value("page"), Some("3"));
    }

    #[test]
    fn page_request_with_auth_and_tenant() {
        let r = PageRequest::new("/x")
            .with_auth(auth_with(&["admin"]))
            .with_tenant(known_tenant_ctx("acme"));
        assert!(!r.auth.is_anonymous());
        assert!(r.tenant.current().is_some());
    }

    #[test]
    fn page_response_ok_has_status_200() {
        let r = PageResponse::ok("Title", "body");
        assert_eq!(r.status, 200);
        assert_eq!(r.title, "Title");
        assert_eq!(r.body, "body");
    }

    #[test]
    fn page_response_not_found_has_status_404() {
        let r = PageResponse::not_found("not found");
        assert_eq!(r.status, 404);
    }

    #[test]
    fn page_response_forbidden_has_status_403() {
        let r = PageResponse::forbidden("denied");
        assert_eq!(r.status, 403);
    }

    #[test]
    fn page_builder_sets_title_and_icon() {
        let p = Page::builder("about", "/about")
            .title("About")
            .icon("info")
            .build();
        assert_eq!(p.title, "About");
        assert_eq!(p.icon, "info");
        assert_eq!(p.path, "/about");
    }

    #[test]
    fn page_builder_default_title_is_untitled() {
        let p = Page::builder("x", "/x").build();
        assert_eq!(p.title, "Untitled");
    }

    #[test]
    fn page_builder_default_scope_is_tenant() {
        let p = Page::builder("x", "/x").build();
        assert!(p.scope.is_tenant());
    }

    #[test]
    fn page_builder_default_category_is_general() {
        let p = Page::builder("x", "/x").build();
        assert_eq!(p.category, "general");
    }

    #[test]
    fn page_render_runs_user_fn() {
        let p = Page::builder("x", "/x")
            .scope(Scope::Public)
            .render(|_| Ok(PageResponse::ok("T", "hello")))
            .build();
        let req = PageRequest::new("/x");
        let r = p.authorize_and_render(&req).unwrap();
        assert_eq!(r.body, "hello");
    }

    #[test]
    fn page_public_skips_auth_and_tenant_checks() {
        let p = Page::builder("x", "/x").scope(Scope::Public).build();
        let req = PageRequest::new("/x");
        assert!(p.authorize_and_render(&req).is_ok());
    }

    #[test]
    fn page_tenant_scope_requires_auth() {
        let p = Page::builder("x", "/x").scope(Scope::Tenant).build();
        let req = PageRequest::new("/x").with_tenant(known_tenant_ctx("acme"));
        let err = p.authorize_and_render(&req).unwrap_err();
        assert!(matches!(err, PageError::Auth(_)));
    }

    #[test]
    fn page_tenant_scope_requires_tenant() {
        let p = Page::builder("x", "/x").scope(Scope::Tenant).build();
        let req = PageRequest::new("/x").with_auth(auth_with(&[]));
        let err = p.authorize_and_render(&req).unwrap_err();
        assert!(matches!(err, PageError::Tenant(_)));
    }

    #[test]
    fn page_tenant_scope_succeeds_with_both() {
        let p = Page::builder("x", "/x")
            .scope(Scope::Tenant)
            .render(|_| Ok(PageResponse::ok("T", "ok")))
            .build();
        let req = PageRequest::new("/x")
            .with_auth(auth_with(&[]))
            .with_tenant(known_tenant_ctx("acme"));
        assert_eq!(p.authorize_and_render(&req).unwrap().body, "ok");
    }

    #[test]
    fn page_admin_scope_requires_role() {
        let p = Page::builder("x", "/x")
            .scope(Scope::Admin(vec!["admin".into()]))
            .build();
        let req = PageRequest::new("/x").with_auth(auth_with(&[]));
        let err = p.authorize_and_render(&req).unwrap_err();
        assert!(matches!(err, PageError::Auth(AuthError::MissingAnyRole(_))));
    }

    #[test]
    fn page_admin_scope_accepts_any_listed_role() {
        let p = Page::builder("x", "/x")
            .scope(Scope::Admin(vec!["admin".into(), "owner".into()]))
            .render(|_| Ok(PageResponse::ok("T", "ok")))
            .build();
        let req = PageRequest::new("/x").with_auth(auth_with(&["owner"]));
        assert!(p.authorize_and_render(&req).is_ok());
    }

    #[test]
    fn page_admin_scope_fails_when_anonymous() {
        let p = Page::builder("x", "/x")
            .scope(Scope::Admin(vec!["admin".into()]))
            .build();
        let req = PageRequest::new("/x");
        let err = p.authorize_and_render(&req).unwrap_err();
        assert!(matches!(err, PageError::Auth(AuthError::Anonymous)));
    }

    #[test]
    fn page_render_error_propagates() {
        let p = Page::builder("x", "/x")
            .scope(Scope::Public)
            .render(|_| Err(PageError::Render("boom".into())))
            .build();
        let req = PageRequest::new("/x");
        let err = p.authorize_and_render(&req).unwrap_err();
        assert!(matches!(err, PageError::Render(s) if s == "boom"));
    }
}
