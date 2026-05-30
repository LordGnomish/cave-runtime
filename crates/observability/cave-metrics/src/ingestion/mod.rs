// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Ingestion protocol implementations.
//! Supports: remote_write, remote_read, Prometheus exposition, OpenMetrics,
//!           OTLP (gRPC+HTTP), StatsD, Graphite, InfluxDB line protocol.

pub mod exposition;
pub mod graphite;
pub mod influx;
pub mod openmetrics;
pub mod otlp;
pub mod remote_read;
pub mod remote_write;
pub mod statsd;

use crate::model::{Labels, Sample, TimeSeries};

/// Parsed ingestion result: a batch of time series.
pub type IngestedBatch = Vec<TimeSeries>;

pub mod chunked;
