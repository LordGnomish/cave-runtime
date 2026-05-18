// SPDX-License-Identifier: AGPL-3.0-or-later
//! Log ingestion subsystem — multiple protocol receivers.

pub mod fluentd;
pub mod loki_push;
pub mod otlp;
pub mod syslog;
