// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-sandbox — Sandbox runtime triumvirate.
//!
//! - google/gvisor release-20260520.0 — Sentry/Gofer user-space syscall isolation.
//! - kata-containers/kata-containers 3.31.0 — VM-based OCI runtime (QEMU/CH/Firecracker).
//! - firecracker-microvm/firecracker v1.15.1 — minimal KVM VMM with REST API.
//!
//! All upstreams Apache-2.0.

pub mod error;
pub mod models;

pub use error::SandboxError;
pub use models::*;
