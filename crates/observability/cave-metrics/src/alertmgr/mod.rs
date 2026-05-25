// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! AlertManager-compatible API implementation.
//! Handles: silences, inhibitions, routing, grouping, notification channels.

pub mod client;
pub mod model;
pub mod routes;
pub mod silence;

pub use client::AlertmanagerClient;
pub use model::{Alert, AlertGroup, InhibitRule, Receiver, Route, Silence};
