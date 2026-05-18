// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl kubectl …` — drop-in shim for kubectl users.
//!
//! Each subcommand here mirrors `kubectl` flags and output, then maps
//! to the equivalent native verb. The shim's job is vocabulary
//! translation: kubectl says `-n <ns>` and `--all-namespaces`; Cave
//! says `-t <tenant>` and `--all-tenants`. The shim accepts the
//! kubectl shape and stores the raw values; the route stays under
//! `/api/compat/kubectl/...` so server-side telemetry can distinguish
//! native vs compat traffic.

pub mod apply;
pub mod create;
pub mod delete;
pub mod describe;
pub mod exec;
pub mod get;
pub mod logs;
pub mod output;
pub mod resource;
