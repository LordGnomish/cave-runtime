// SPDX-License-Identifier: AGPL-3.0-or-later
//! AlertManager-compatible API implementation.
//! Handles: silences, inhibitions, routing, grouping, notification channels.

pub mod client;
pub mod model;
pub mod routes;
pub mod silence;

pub use model::{Alert, AlertGroup, Receiver, Route, Silence, InhibitRule};
pub use client::AlertmanagerClient;
