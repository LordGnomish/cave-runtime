//! HTTP route modules.
//!
//! Routes are split by API surface:
//!   jaeger   — Jaeger UI / query API  (/api/traces, /api/services, /api/dependencies, /api/metrics)
//!   tempo    — Grafana Tempo API      (/api/traces, /api/search, /api/search/tags, TraceQL)
//!   ingest   — All ingestion endpoints (OTLP, Jaeger collector, Zipkin, OpenCensus)

pub mod ingest;
pub mod jaeger;
pub mod tempo;
