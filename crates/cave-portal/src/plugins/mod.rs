//! Portal plugins — per-domain views rendered natively.
//!
//! Each plugin owns a set of panels, list pages, and detail views for one
//! capability area. The portal *never* embeds an upstream UI; if the
//! capability is provided by an external tool (Argo CD, Grafana, Vault, etc.)
//! the plugin re-implements the relevant view shape and uses cave-portal-api
//! as the single data plane.

pub mod argocd;
pub mod badges;
pub mod cost_insight;
pub mod grafana;
pub mod kubernetes;
pub mod reflex;
pub mod scaffolder;
pub mod search;
pub mod techdocs;
pub mod vault;

/// Persona for whom a plugin view is intended.
///
/// Mirrors the persona model in `cave-portal-api`: tenants are app owners,
/// operators are cluster admins, admins are platform staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPersona {
    Tenant,
    Operator,
    Admin,
}

impl ViewPersona {
    pub fn label(&self) -> &'static str {
        match self {
            ViewPersona::Tenant => "tenant",
            ViewPersona::Operator => "operator",
            ViewPersona::Admin => "admin",
        }
    }
}
