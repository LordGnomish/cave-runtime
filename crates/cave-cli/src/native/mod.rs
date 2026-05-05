//! Native verbs for `cavectl` — Cave-domain, upstream-agnostic.
//!
//! Per ADR-RUNTIME-CLI-CONSOLIDATION-001 these are the canonical
//! commands. Each verb composes across modules (federated watch,
//! tenant scope, cross-module snapshot) and has no direct upstream-CLI
//! equivalent — that is precisely why it ships as a first-class verb
//! rather than a `kubectl` plugin.
//!
//! Compat shims under `crate::compat` delegate into this surface.

pub mod auth;
pub mod chaos;
pub mod deploy;
pub mod describe;
pub mod events;
pub mod flag;
pub mod get;
pub mod logs;
pub mod secrets;
pub mod topology;

pub mod request;

pub use request::{HttpVerb, PreparedRequest};
