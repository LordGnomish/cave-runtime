// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-sandbox — Sandbox runtime triumvirate.
//!
//! - google/gvisor release-20260520.0 — Sentry/Gofer user-space syscall isolation.
//! - kata-containers/kata-containers 3.31.0 — VM-based OCI runtime (QEMU/CH/Firecracker).
//! - firecracker-microvm/firecracker v1.15.1 — minimal KVM VMM with REST API.
//!
//! All upstreams Apache-2.0.

pub mod api;
pub mod error;
pub mod firecracker_api;
pub mod firecracker_jailer;
pub mod firecracker_vmm;
pub mod gvisor_gofer;
pub mod gvisor_runsc;
pub mod gvisor_sentry;
pub mod kata_agent;
pub mod kata_hypervisor;
pub mod kata_runtime;
pub mod kata_shim;
pub mod lifecycle;
pub mod models;
pub mod oci_runtime_spec;
pub mod observability;
pub mod store;

pub use error::SandboxError;
pub use models::*;

/// Crate name as referenced by orchestrator parity tooling.
pub const MODULE_NAME: &str = "cave-sandbox";
