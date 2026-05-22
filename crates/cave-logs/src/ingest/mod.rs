// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Log ingestion subsystem — multiple protocol receivers.

pub mod fluentd;
pub mod loki_push;
pub mod otlp;
pub mod syslog;
