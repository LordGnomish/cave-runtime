// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts integrations module root (cross-crate adapters)
//! Cross-crate adapters. Each sub-module wires a cave-artifacts surface to
//! another cave-* crate via a real trait-shaped bridge.
//!
//! - [`trivy`] — Harbor scan + Pulp container plugin → `cave-scan`
//! - [`auth`]  — Harbor RBAC → `cave-auth` OIDC

pub mod auth;
pub mod trivy;
