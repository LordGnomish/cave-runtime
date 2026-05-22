// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cave Portal web shell — Backstage-style plugin host.
//!
//! The web shell provides:
//! - **App shell**: top nav + sidebar layout shared across plugins.
//! - **Plugin host**: registry where modules contribute pages, panels, and nav entries.
//! - **Page router**: maps incoming paths → registered pages.
//! - **Auth context**: resolves the current user from a request and exposes it
//!   to plugins.
//! - **Tenant context**: scopes every page render to a tenant; default-deny when
//!   no tenant is set.
//!
//! Tenant scoping is an *invariant*: pages whose route declares
//! [`Scope::Tenant`] cannot render without a resolved tenant id. This mirrors
//! the RBAC story enforced by `cave-portal-api`.

pub mod app_shell;
pub mod auth;
pub mod page;
pub mod plugin;
pub mod render;
pub mod router;
pub mod tenant;

pub use app_shell::{AppShell, ShellConfig};
pub use auth::{AuthContext, AuthError, Identity};
pub use page::{Page, PageError, PageRequest, PageResponse, Scope};
pub use plugin::{Plugin, PluginRegistry};
pub use render::{escape_html, render_page};
pub use router::{Route, RouteMatch, Router};
pub use tenant::{TenantContext, TenantError, TenantId};

pub const MODULE_NAME: &str = "portal-web";
