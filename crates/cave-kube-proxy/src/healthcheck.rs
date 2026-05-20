// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-Service health check NodePorts.
//!
//! Cite: `pkg/proxy/healthcheck/service_health.go:43` (ServiceHealthServer),
//! `:122` (server.SyncServices), `:241` (hcHandler.ServeHTTP),
//! `:274` (server.SyncEndpoints).
//!
//! Each Service with externalTrafficPolicy=Local has a dedicated
//! HealthCheckNodePort. The server returns 200 OK iff there is at least
//! one ready endpoint local to the node, otherwise 503 Service
//! Unavailable. cave keeps the same semantics in-process (no HTTP server
//! at this layer — datapath emission is the proxier's job).

use crate::error::{KubeProxyError, KubeProxyResult};
use crate::service::ServicePortName;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct HealthCheckServer {
    pub tenant_id: String,
    /// Service → assigned health-check NodePort.
    services: HashMap<ServicePortName, u16>,
    /// Service → number of ready local endpoints.
    endpoint_counts: HashMap<ServicePortName, u32>,
}

impl HealthCheckServer {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            services: HashMap::new(),
            endpoint_counts: HashMap::new(),
        }
    }

    /// Cite: `pkg/proxy/healthcheck/service_health.go:122`
    /// (server.SyncServices) — diff-based update; previously-tracked
    /// services no longer present are dropped from the table.
    pub fn sync_services(&mut self, new_services: HashMap<ServicePortName, u16>) {
        self.services = new_services;
        self.endpoint_counts
            .retain(|svc, _| self.services.contains_key(svc));
    }

    /// Cite: `pkg/proxy/healthcheck/service_health.go:274`
    /// (server.SyncEndpoints).
    pub fn sync_endpoints(&mut self, new_counts: HashMap<ServicePortName, u32>) {
        self.endpoint_counts = new_counts;
    }

    /// Cite: `pkg/proxy/healthcheck/service_health.go:241`
    /// (hcHandler.ServeHTTP) — 200 if local ready endpoints > 0,
    /// otherwise 503. Unknown service ⇒ 404.
    pub fn http_status(&self, svc: &ServicePortName) -> u16 {
        if !self.services.contains_key(svc) {
            return 404;
        }
        match self.endpoint_counts.get(svc).copied().unwrap_or(0) {
            0 => 503,
            _ => 200,
        }
    }

    /// Returns the JSON body served alongside the status code.
    /// Cite: `pkg/proxy/healthcheck/service_health.go:241`
    /// (hcHandler.ServeHTTP) — body shape is
    /// `{"service":{"namespace":..., "name":...}, "localEndpoints":N}`.
    pub fn http_body(&self, svc: &ServicePortName) -> serde_json::Value {
        let n = self.endpoint_counts.get(svc).copied().unwrap_or(0);
        serde_json::json!({
            "service": { "namespace": svc.namespace, "name": svc.name },
            "localEndpoints": n,
        })
    }

    pub fn assigned_port(&self, svc: &ServicePortName) -> Option<u16> {
        self.services.get(svc).copied()
    }

    pub fn tracked_count(&self) -> usize {
        self.services.len()
    }

    pub fn check_tenant(&self, requesting_tenant: &str) -> KubeProxyResult<()> {
        if self.tenant_id != requesting_tenant {
            return Err(KubeProxyError::CrossTenantDenied {
                store: self.tenant_id.clone(),
                req: requesting_tenant.to_string(),
            });
        }
        Ok(())
    }
}
