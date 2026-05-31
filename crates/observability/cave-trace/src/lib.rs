// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-trace — Distributed tracing engine with full Jaeger/Tempo parity.
//!
//! Ingestion protocols
//! ───────────────────
//! • OTLP HTTP/JSON   (POST /v1/traces, Content-Type: application/json)
//! • OTLP HTTP/proto  (501 — requires prost build step)
//! • Jaeger HTTP JSON (POST /api/traces)
//! • Jaeger Thrift binary (POST /api/traces, Content-Type: application/x-thrift)
//! • Jaeger UDP agent  (UDP 6831 compact / 6832 binary — start via start_background_services)
//! • Zipkin v2 JSON   (POST /api/v2/spans)
//! • OpenCensus JSON  (POST /oc/v1/traces)
//!
//! Query APIs
//! ──────────
//! • Jaeger /api/traces, /api/services, /api/dependencies, /api/metrics (SPM)
//! • Tempo  /api/traces, /api/search, /api/search/tags, TraceQL
//!
//! Features
//! ────────
//! • Columnar in-memory storage with Bloom filter + tag index
//! • Head-based, tail-based, and adaptive sampling
//! • TraceQL query language
//! • Service Performance Monitoring (RED metrics)
//! • Trace-to-logs and trace-to-metrics correlation
//! • Multi-tenant (X-Scope-OrgID header)
//! • Background: Jaeger UDP agent, retention GC

pub mod analyzer;
pub mod correlation;
pub mod dependency;
pub mod error;
pub mod ingestion;
pub mod multi_tenant;
pub mod propagation;
pub mod query;
pub mod routes;
pub mod adaptive_sampling;
pub mod sampling;
pub mod servicegraph;
pub mod spm;
pub mod storage;
pub mod storage_badger;
pub mod storage_cassandra;
pub mod storage_es;
pub mod storage_kafka;
pub mod traceql;
pub mod tracegen;
pub mod types;

pub use error::{Result, TraceError};
pub use types::{Span, SpanId, SpanKind, SpanStatus, TagValue, Trace, TraceId};

use axum::Router;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use multi_tenant::TenantRegistry;
use sampling::{SamplingConfig, build_sampler};
use spm::SpmRegistry;
use storage::{RetentionPolicy, TraceStore};

// ─── TraceConfig ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TraceConfig {
    pub max_traces: usize,
    pub retention_hours: u64,
    pub sampling: SamplingConfig,
    /// Jaeger UDP compact protocol agent port.
    pub jaeger_udp_compact_port: u16,
    /// Jaeger UDP binary protocol agent port.
    pub jaeger_udp_binary_port: u16,
    /// SPM window in seconds.
    pub spm_window_secs: u64,
    /// Retention GC interval.
    pub gc_interval: Duration,
    /// Whether to auto-register unknown tenants.
    pub auto_register_tenants: bool,
}

impl Default for TraceConfig {
    fn default() -> Self {
        TraceConfig {
            max_traces: 100_000,
            retention_hours: 72,
            sampling: SamplingConfig::Constant { sample: true },
            jaeger_udp_compact_port: 6831,
            jaeger_udp_binary_port: 6832,
            spm_window_secs: 60,
            gc_interval: Duration::from_secs(300),
            auto_register_tenants: true,
        }
    }
}

// ─── TraceState ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TraceState {
    pub store: Arc<RwLock<TraceStore>>,
    pub sampler: Arc<dyn sampling::Sampler + Send + Sync>,
    pub spm_registry: Arc<SpmRegistry>,
    pub tenant_registry: Arc<TenantRegistry>,
    pub query: Arc<query::QueryEngine>,
}

impl TraceState {
    pub fn new(config: &TraceConfig) -> Self {
        let retention = RetentionPolicy::from_hours(config.retention_hours, config.max_traces);
        let store = Arc::new(RwLock::new(TraceStore::new(retention)));
        let sampler = build_sampler(&config.sampling);
        let spm_registry = Arc::new(SpmRegistry::new(config.spm_window_secs));
        let tenant_registry = Arc::new(TenantRegistry::new(config.auto_register_tenants));
        let query = Arc::new(query::QueryEngine::new(store.clone()));

        TraceState {
            store,
            sampler,
            spm_registry,
            tenant_registry,
            query,
        }
    }
}

// ─── Router ────────────────────────────────────────────────────────────────

/// Build the combined axum Router for all trace endpoints.
pub fn router(state: Arc<TraceState>) -> Router {
    let ingest_router = routes::ingest::create_router(state.clone());
    let jaeger_router = routes::jaeger::create_router(state.clone());
    let tempo_router = routes::tempo::create_router(state);

    Router::new()
        .merge(ingest_router)
        .merge(jaeger_router)
        .merge(tempo_router)
}

// ─── Background services ───────────────────────────────────────────────────

/// Start background tasks:
/// - Jaeger UDP agent listeners
/// - Retention GC
/// - SPM window rotation
pub async fn start_background_services(state: Arc<TraceState>, config: TraceConfig) {
    // Retention GC
    {
        let store = state.store.clone();
        let interval = config.gc_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let removed = store.write().await.apply_retention();
                if removed > 0 {
                    tracing::info!(removed, "cave-trace: retention GC removed traces");
                }
            }
        });
    }

    // SPM window rotation
    {
        let spm = state.spm_registry.clone();
        let window = config.spm_window_secs;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(window));
            loop {
                ticker.tick().await;
                spm.rotate();
                tracing::debug!("cave-trace: SPM window rotated");
            }
        });
    }

    // Jaeger UDP compact agent (port 6831)
    {
        let store = state.store.clone();
        let sampler = state.sampler.clone();
        let spm = state.spm_registry.clone();
        let port = config.jaeger_udp_compact_port;
        tokio::spawn(async move {
            if let Err(e) = run_jaeger_udp_agent(store, sampler, spm, port, false).await {
                tracing::warn!("Jaeger UDP compact agent error: {}", e);
            }
        });
    }

    // Jaeger UDP binary agent (port 6832)
    {
        let store = state.store.clone();
        let sampler = state.sampler.clone();
        let spm = state.spm_registry.clone();
        let port = config.jaeger_udp_binary_port;
        tokio::spawn(async move {
            if let Err(e) = run_jaeger_udp_agent(store, sampler, spm, port, true).await {
                tracing::warn!("Jaeger UDP binary agent error: {}", e);
            }
        });
    }
}

/// UDP receive loop for the Jaeger agent protocol.
async fn run_jaeger_udp_agent(
    store: Arc<RwLock<TraceStore>>,
    sampler: Arc<dyn sampling::Sampler + Send + Sync>,
    spm: Arc<SpmRegistry>,
    port: u16,
    binary: bool,
) -> std::io::Result<()> {
    use tokio::net::UdpSocket;

    let socket = UdpSocket::bind(format!("0.0.0.0:{}", port)).await?;
    tracing::info!(port, binary, "cave-trace: Jaeger UDP agent listening");

    // Jaeger UDP packets are max 64 KB
    let mut buf = vec![0u8; 65_536];

    loop {
        let (len, _addr) = match socket.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("UDP recv error: {}", e);
                continue;
            }
        };

        let data = &buf[..len];
        let result = if binary {
            ingestion::jaeger::parse_jaeger_thrift_binary(data, multi_tenant::DEFAULT_TENANT)
        } else {
            ingestion::jaeger::parse_jaeger_thrift_compact(data, multi_tenant::DEFAULT_TENANT)
        };

        match result {
            Ok(spans) if !spans.is_empty() => {
                // Apply sampling
                let keep = spans
                    .iter()
                    .any(|s| sampler.should_sample(s.trace_id, s).is_sample());
                if keep {
                    spm.record_spans(&spans);
                    store.write().await.ingest_spans(spans);
                }
            }
            Ok(_) => {}
            Err(e) => tracing::debug!("Jaeger UDP parse error (port {}): {}", port, e),
        }
    }
}

pub const MODULE_NAME: &str = "trace";
