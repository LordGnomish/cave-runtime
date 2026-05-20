// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Automatic request metrics: request count, error count, latency, and active
//! connections per service pair.  Exports Prometheus text format.
//!
//! Automatic metrics (Istio parity):
//!   cave_mesh_requests_total{source,destination,method,response_code}
//!   cave_mesh_errors_total{...}
//!   cave_mesh_request_duration_ms_total{...}
//!   cave_mesh_bytes_total{...}
//!   cave_mesh_active_connections{service}
//!   cave_mesh_circuit_trips_total{service}
//!   cave_mesh_rate_limited_total{service}
//!   cave_mesh_faults_injected_total{service}

use prometheus_client::{
    encoding::{EncodeLabelSet, text::encode},
    metrics::{counter::Counter, family::Family, gauge::Gauge},
    registry::Registry,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RequestLabels {
    pub source: String,
    pub destination: String,
    pub method: String,
    pub response_code: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ServiceLabels {
    pub service: String,
}

#[derive(Clone)]
pub struct MeshMetrics {
    pub request_total: Family<RequestLabels, Counter>,
    pub error_total: Family<RequestLabels, Counter>,
    pub request_duration_ms_total: Family<RequestLabels, Counter>,
    pub bytes_total: Family<RequestLabels, Counter>,
    pub active_connections: Family<ServiceLabels, Gauge>,
    pub circuit_trips_total: Family<ServiceLabels, Counter>,
    pub rate_limited_total: Family<ServiceLabels, Counter>,
    pub faults_injected_total: Family<ServiceLabels, Counter>,

    registry: Arc<Mutex<Registry>>,
}

impl MeshMetrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let request_total = Family::<RequestLabels, Counter>::default();
        let error_total = Family::<RequestLabels, Counter>::default();
        let request_duration_ms_total = Family::<RequestLabels, Counter>::default();
        let bytes_total = Family::<RequestLabels, Counter>::default();
        let active_connections = Family::<ServiceLabels, Gauge>::default();
        let circuit_trips_total = Family::<ServiceLabels, Counter>::default();
        let rate_limited_total = Family::<ServiceLabels, Counter>::default();
        let faults_injected_total = Family::<ServiceLabels, Counter>::default();

        registry.register(
            "cave_mesh_requests_total",
            "Total requests routed through the CAVE mesh",
            request_total.clone(),
        );
        registry.register(
            "cave_mesh_errors_total",
            "Total 5xx errors in the CAVE mesh",
            error_total.clone(),
        );
        registry.register(
            "cave_mesh_request_duration_ms_total",
            "Cumulative request duration in milliseconds",
            request_duration_ms_total.clone(),
        );
        registry.register(
            "cave_mesh_bytes_total",
            "Total bytes transferred through the CAVE mesh",
            bytes_total.clone(),
        );
        registry.register(
            "cave_mesh_active_connections",
            "Active in-flight connections per service",
            active_connections.clone(),
        );
        registry.register(
            "cave_mesh_circuit_trips_total",
            "Total circuit breaker trips",
            circuit_trips_total.clone(),
        );
        registry.register(
            "cave_mesh_rate_limited_total",
            "Total requests rejected by rate limiting",
            rate_limited_total.clone(),
        );
        registry.register(
            "cave_mesh_faults_injected_total",
            "Total fault injections applied",
            faults_injected_total.clone(),
        );

        Self {
            request_total,
            error_total,
            request_duration_ms_total,
            bytes_total,
            active_connections,
            circuit_trips_total,
            rate_limited_total,
            faults_injected_total,
            registry: Arc::new(Mutex::new(registry)),
        }
    }

    pub fn record_request(
        &self,
        source: &str,
        destination: &str,
        method: &str,
        status: u16,
        bytes: u64,
        duration_ms: u64,
    ) {
        let labels = RequestLabels {
            source: source.to_string(),
            destination: destination.to_string(),
            method: method.to_string(),
            response_code: status.to_string(),
        };
        self.request_total.get_or_create(&labels).inc();
        if status >= 500 {
            self.error_total.get_or_create(&labels).inc();
        }
        if bytes > 0 {
            self.bytes_total.get_or_create(&labels).inc_by(bytes);
        }
        if duration_ms > 0 {
            self.request_duration_ms_total
                .get_or_create(&labels)
                .inc_by(duration_ms);
        }
    }

    pub fn inc_connections(&self, service: &str) {
        let lbl = ServiceLabels {
            service: service.to_string(),
        };
        self.active_connections.get_or_create(&lbl).inc();
    }

    pub fn dec_connections(&self, service: &str) {
        let lbl = ServiceLabels {
            service: service.to_string(),
        };
        self.active_connections.get_or_create(&lbl).dec();
    }

    pub fn record_circuit_trip(&self, service: &str) {
        let lbl = ServiceLabels {
            service: service.to_string(),
        };
        self.circuit_trips_total.get_or_create(&lbl).inc();
    }

    pub fn record_rate_limited(&self, service: &str) {
        let lbl = ServiceLabels {
            service: service.to_string(),
        };
        self.rate_limited_total.get_or_create(&lbl).inc();
    }

    pub fn record_fault_injected(&self, service: &str) {
        let lbl = ServiceLabels {
            service: service.to_string(),
        };
        self.faults_injected_total.get_or_create(&lbl).inc();
    }

    pub fn export(&self) -> String {
        let registry = self.registry.lock().unwrap();
        let mut output = String::new();
        encode(&mut output, &registry).unwrap_or_default();
        output
    }
}

impl Default for MeshMetrics {
    fn default() -> Self {
        Self::new()
    }
}
