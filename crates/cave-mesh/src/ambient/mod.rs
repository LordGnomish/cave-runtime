//! Istio Ambient-mode parity batch.
//!
//! This module is a *deeper* (no-stub) parity port of the Ambient-mode
//! pieces in upstream `istio/istio` v1.29.2:
//!
//! * [`hbone`] — HBONE (HTTP/2 CONNECT-tunnelling) request/response framing.
//! * [`ztunnel`] — node-local L4 mTLS proxy state machine.
//! * [`waypoint`] — per-namespace L7 router (HTTP/1.1, HTTP/2, gRPC).
//! * [`authz`] — AuthorizationPolicy evaluator
//!   (source identity + HTTP method + path + JWT claim).
//! * [`virtualservice`] — VirtualService → ordered route table compiler.
//! * [`destinationrule`] — Cluster + load-balancing policy.
//! * [`svid`] — SPIFFE SVID enrolment (cave-auth issuer integration).
//! * [`telemetry`] — access log + Prometheus metric + OpenTelemetry span.
//!
//! Every public type is annotated with the upstream [`Cite`] it ports, and
//! every test in this subtree carries a `Cite` and a `TenantId` via the
//! module-local [`crate::ambient_test_ctx`] macro.

pub mod types;

pub mod authz;
pub mod destinationrule;
pub mod hbone;
pub mod svid;
pub mod telemetry;
pub mod virtualservice;
pub mod waypoint;
pub mod ztunnel;

pub use types::{Cite, TenantId, UPSTREAM_VERSION};
