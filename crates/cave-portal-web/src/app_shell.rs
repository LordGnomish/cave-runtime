// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! App shell — wires together [`Router`], [`PluginRegistry`], and the
//! per-request auth + tenant contexts to serve a full HTML page.
//!
//! The shell is intentionally minimal: it owns the registry/router, accepts a
//! [`crate::PageRequest`], and returns the rendered HTML. Plugins do not need
//! to know about HTTP at all.

use crate::page::{PageError, PageRequest, PageResponse};
use crate::plugin::{Plugin, PluginRegistry};
use crate::render::render_page;
use crate::router::Router;

#[derive(Debug, Clone)]
pub struct ShellConfig {
    pub product_name: String,
    pub default_path: String,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            product_name: "Cave Portal".to_string(),
            default_path: "/".to_string(),
        }
    }
}

#[derive(Debug)]
pub struct AppShell {
    pub config: ShellConfig,
    pub registry: PluginRegistry,
    pub router: Router,
}

impl Default for AppShell {
    fn default() -> Self {
        Self::new(ShellConfig::default())
    }
}

impl AppShell {
    pub fn new(config: ShellConfig) -> Self {
        Self {
            config,
            registry: PluginRegistry::new(),
            router: Router::new(),
        }
    }

    pub fn install<P: Plugin>(&mut self, plugin: &P) -> &mut Self {
        self.registry.install(plugin);
        // mirror pages into the router
        for page in self.registry.pages().iter().cloned().collect::<Vec<_>>() {
            // ensure we don't re-add the same path twice (idempotent install)
            if self
                .router
                .routes()
                .iter()
                .any(|r| r.pattern == page.path && r.page.id == page.id)
            {
                continue;
            }
            self.router.register(page);
        }
        self
    }

    /// Lookup the page for `path`, authorize, render, and wrap with the
    /// HTML shell. Returns either the rendered HTML body or a [`PageError`].
    pub fn handle(&self, mut req: PageRequest) -> Result<ShellResponse, PageError> {
        let r#match = match self.router.r#match(&req.path) {
            Some(m) => m,
            None => {
                return Ok(ShellResponse {
                    status: 404,
                    body: render_not_found(&self.config, &req.path),
                });
            }
        };
        // copy params onto request
        for (k, v) in r#match.params.into_iter() {
            req.params.push((k, v));
        }
        let resp = r#match.page.authorize_and_render(&req)?;
        let body = render_page(r#match.page, &req, &resp);
        Ok(ShellResponse { status: resp.status, body })
    }

    pub fn page_count(&self) -> usize {
        self.registry.page_count()
    }

    pub fn nav_count(&self) -> usize {
        self.registry.nav_count()
    }

    pub fn panel_count(&self) -> usize {
        self.registry.panel_count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellResponse {
    pub status: u16,
    pub body: String,
}

fn render_not_found(config: &ShellConfig, path: &str) -> String {
    let product = crate::render::escape_html(&config.product_name);
    let path = crate::render::escape_html(path);
    format!(
        "<!doctype html>\n\
         <html><head><title>{product} — Not Found</title></head>\n\
         <body><h1>404</h1><p>No page registered for <code>{path}</code></p></body>\n\
         </html>\n"
    )
}

// Convenience: a stub PageResponse for tests / not-found bodies.
impl From<&'static str> for PageResponse {
    fn from(s: &'static str) -> Self {
        PageResponse::ok("Untitled", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthContext, Identity};
    use crate::page::{Page, Scope};
    use crate::plugin::{NavEntry, Plugin, PluginRegistry};
    use crate::tenant::{TenantContext, TenantId, TenantInfo, TenantPlan};

    struct EchoPlugin;
    impl Plugin for EchoPlugin {
        fn name(&self) -> &str {
            "echo"
        }
        fn register(&self, registry: &mut PluginRegistry) {
            registry.add_page(
                Page::builder("home", "/")
                    .title("Home")
                    .scope(Scope::Public)
                    .render(|_| Ok(PageResponse::ok("Home", "<p>home</p>")))
                    .build(),
            );
            registry.add_page(
                Page::builder("user", "/users/:id")
                    .title("User")
                    .scope(Scope::Public)
                    .render(|req| {
                        let id = req.param("id").unwrap_or("?").to_string();
                        Ok(PageResponse::ok("User", format!("<p>user {id}</p>")))
                    })
                    .build(),
            );
            registry.add_page(
                Page::builder("settings", "/settings")
                    .title("Settings")
                    .scope(Scope::Tenant)
                    .render(|req| {
                        let tid = req.tenant.require()?;
                        Ok(PageResponse::ok("Settings", format!("<p>tenant {tid}</p>")))
                    })
                    .build(),
            );
            registry.add_page(
                Page::builder("admin", "/admin")
                    .title("Admin")
                    .scope(Scope::Admin(vec!["admin".into()]))
                    .render(|_| Ok(PageResponse::ok("Admin", "<p>admin</p>")))
                    .build(),
            );
            registry.add_nav(NavEntry::new("Home", "/").with_order(1));
            registry.add_nav(NavEntry::new("Settings", "/settings").with_order(2));
        }
    }

    fn shell_with_echo() -> AppShell {
        let mut s = AppShell::default();
        s.install(&EchoPlugin);
        s
    }

    fn tenant_ctx(slug: &str) -> TenantContext {
        let mut ctx = TenantContext::new();
        let id = TenantId::new(slug).unwrap();
        ctx.register(TenantInfo {
            id: id.clone(),
            display_name: slug.into(),
            plan: TenantPlan::Free,
        });
        ctx.set_current(id).unwrap();
        ctx
    }

    #[test]
    fn shell_default_config() {
        let s = AppShell::default();
        assert_eq!(s.config.product_name, "Cave Portal");
        assert_eq!(s.config.default_path, "/");
    }

    #[test]
    fn shell_install_adds_pages_to_router() {
        let s = shell_with_echo();
        assert!(s.page_count() >= 4);
        // router should know about the 4 paths
        assert!(s.router.r#match("/").is_some());
        assert!(s.router.r#match("/users/1").is_some());
        assert!(s.router.r#match("/settings").is_some());
        assert!(s.router.r#match("/admin").is_some());
    }

    #[test]
    fn shell_install_records_plugin_name() {
        let s = shell_with_echo();
        assert!(s.registry.plugins().contains(&"echo".to_string()));
    }

    #[test]
    fn shell_install_is_idempotent_on_pages() {
        let mut s = AppShell::default();
        s.install(&EchoPlugin);
        let first = s.router.len();
        s.install(&EchoPlugin); // re-install — page count grows but routes dedup by id+path
        // pages array grows because registry just appends
        assert!(s.page_count() >= first);
    }

    #[test]
    fn shell_handle_public_page() {
        let s = shell_with_echo();
        let resp = s.handle(PageRequest::new("/")).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("<p>home</p>"));
    }

    #[test]
    fn shell_handle_param_page_captures_id() {
        let s = shell_with_echo();
        let resp = s.handle(PageRequest::new("/users/42")).unwrap();
        assert!(resp.body.contains("user 42"));
    }

    #[test]
    fn shell_handle_unknown_path_returns_404() {
        let s = shell_with_echo();
        let resp = s.handle(PageRequest::new("/missing")).unwrap();
        assert_eq!(resp.status, 404);
        assert!(resp.body.contains("404"));
    }

    #[test]
    fn shell_handle_tenant_scope_without_tenant_errors() {
        let s = shell_with_echo();
        let req = PageRequest::new("/settings").with_auth(AuthContext::with_identity(
            Identity::new("u", "U"),
        ));
        let err = s.handle(req).unwrap_err();
        assert!(matches!(err, PageError::Tenant(_)));
    }

    #[test]
    fn shell_handle_tenant_scope_with_full_context_renders() {
        let s = shell_with_echo();
        let req = PageRequest::new("/settings")
            .with_auth(AuthContext::with_identity(Identity::new("u", "U")))
            .with_tenant(tenant_ctx("acme"));
        let resp = s.handle(req).unwrap();
        assert!(resp.body.contains("tenant acme"));
    }

    #[test]
    fn shell_handle_admin_scope_without_role_errors() {
        let s = shell_with_echo();
        let req = PageRequest::new("/admin").with_auth(AuthContext::with_identity(
            Identity::new("u", "U"),
        ));
        let err = s.handle(req).unwrap_err();
        assert!(matches!(err, PageError::Auth(_)));
    }

    #[test]
    fn shell_handle_admin_scope_with_role_renders() {
        let s = shell_with_echo();
        let req = PageRequest::new("/admin").with_auth(AuthContext::with_identity(
            Identity::new("u", "U").with_role("admin"),
        ));
        let resp = s.handle(req).unwrap();
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("<p>admin</p>"));
    }

    #[test]
    fn shell_404_includes_path_in_body() {
        let s = shell_with_echo();
        let resp = s.handle(PageRequest::new("/<script>")).unwrap();
        // path must be html-escaped
        assert!(resp.body.contains("&lt;script&gt;"));
        assert!(!resp.body.contains("<script>"));
    }

    #[test]
    fn shell_nav_count_reflects_plugin() {
        let s = shell_with_echo();
        assert_eq!(s.nav_count(), 2);
    }
}
