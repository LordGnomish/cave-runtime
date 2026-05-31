// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! cave-falco — Falco runtime-security userspace surface.
//!
//! Upstream: <https://github.com/falcosecurity/falco> v0.43.1 (Apache-2.0).
//!
//! Per `ADR-RUNTIME-SANDBOX-NO-FFI-001` (no-unsafe / no-FFI / kernel
//! out-of-process), this crate models Falco's userspace surface only:
//!
//! - Rule DSL (YAML rules + macros + lists + outputs + priorities)
//! - Engine (rule compilation, condition evaluation against event JSON)
//! - Event types (syscall events from libsinsp wire format; k8s_audit
//!   audit events)
//! - Output formatters (text, JSON, sidekick HTTP payload)
//! - Plugin SDK (Rust trait + plugin capabilities)
//! - Observability dashboards + alerts
//!
//! Kernel-side capture (kmod / modern_bpf / libbpf / pdig / ptrace) is
//! delegated to external Falco binaries — formal scope_cuts per the ADR.

#![forbid(unsafe_code)]

pub mod error;
pub mod event;
pub mod k8s_audit;
pub mod observability;
pub mod output;
pub mod parity_self_audit;
pub mod plugin_sdk;
pub mod token_bucket;
pub mod rule;
pub mod rule_loader;
pub mod engine;
pub mod cli;
pub mod routes;

pub use error::{FalcoError, Result};
pub use event::{FalcoEvent, EventClass, Priority};
pub use rule::{Rule, RuleAction, MacroDef, ListDef, OutputField};
pub use engine::{Engine, EngineMatch};
