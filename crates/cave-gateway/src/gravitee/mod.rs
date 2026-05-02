//! Gravitee feature pack — adds developer portal, API analytics, debug mode,
//! design-time governance (linting + quality gates), and federation gateway
//! on top of the Kong proxy core.
//!
//! After 2026-05-02 this is the Gravitee half of cave-gateway's two-upstream
//! parity (Kong + Gravitee). The previous Envoy xDS surface is gone; the
//! canonical Gravitee API/plan/application/subscription surface is added in
//! a follow-up commit.

pub mod analytics;
pub mod catalog;
pub mod debug;
pub mod devportal;
pub mod federation;
pub mod governance;

pub use analytics::AnalyticsStore;
pub use catalog::CatalogStore;
pub use debug::DebugStore;
pub use devportal::DevPortalStore;
pub use federation::FederationStore;
pub use governance::GovernanceEngine;
