//! Compatibility shims for `cavectl`.
//!
//! Per ADR-RUNTIME-CLI-CONSOLIDATION-001, each shim accepts the
//! upstream CLI's exact flag set and output format, then maps onto
//! the native verb that Cave actually implements. The shim layer is
//! intentionally thin — flag mapping, path routing, output shaping —
//! and delegates real work to `crate::native`.

pub mod argocd;
pub mod helm;
pub mod kubectl;
