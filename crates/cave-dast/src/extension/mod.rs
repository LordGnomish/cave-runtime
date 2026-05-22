// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: zaproxy/zaproxy@v2.14.0
//   zap/src/main/java/org/zaproxy/zap/extension/
//
//! ZAP extension subsystems — parity port for the four 2026-05-19
//! parity-uplift modules:
//!
//! * [`forced_user`]  — multi-user context impersonation
//! * [`fuzz`]         — fuzzer payload generators / processors
//! * [`websocket`]    — WebSocket proxy + scan
//! * [`anticsrf`]     — anti-CSRF token replay engine

pub mod anticsrf;
pub mod forced_user;
pub mod fuzz;
pub mod websocket;
