// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alert routing & management — Alertmanager parity.
//!
//! Compatible with: Alertmanager v0.26+ (route tree, group_by, inhibits,
//! silences, receivers, multi-tenant via `X-Scope-OrgID`).
//!
//! Module layout:
//! - `models`     — Alert, Matcher, Silence, InhibitRule, Route (hierarchical), Receiver
//! - `matcher`    — anchored-regex matcher evaluation, fingerprinting
//! - `routing`    — hierarchical route tree walker
//! - `silence`    — silence application + tenant scoping
//! - `inhibit`    — inhibit rule application
//! - `grouping`   — alert grouping, dedup, throttle (group_wait/_interval/repeat_interval)
//! - `receivers`  — payload renderers (Slack, webhook, email, PagerDuty, OpsGenie, GrafanaOnCall)
//! - `engine`     — top-level pipeline + legacy compat
//! - `tenant`     — `X-Scope-OrgID` header parsing + label injection
//! - `store`      — in-memory store
//! - `routes`     — Alertmanager v2 HTTP API

pub mod engine;
pub mod grouping;
pub mod inhibit;
pub mod matcher;
pub mod models;
pub mod receivers;
pub mod routes;
pub mod routing;
pub mod rules;
pub mod silence;
pub mod store;
pub mod tenant;

pub use models::{
    Alert, AlertSeverity, AlertState, DEFAULT_TENANT, EmailConfig, GrafanaOnCallConfig,
    InhibitRule, MatchType, Matcher, OpsGenieConfig, PagerDutyConfig, Receiver, ReceiverConfig,
    Route, Silence, SlackConfig, TENANT_LABEL, WebhookConfig,
};
pub use routes::{AppState, create_router as router};
pub use store::AlertStore;

/// Backwards-compatible legacy alias.
pub type State = AppState;

pub const MODULE_NAME: &str = "alerts";
