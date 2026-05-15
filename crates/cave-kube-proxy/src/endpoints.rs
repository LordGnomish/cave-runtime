// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EndpointSlice consumer.
//!
//! Cite: `pkg/proxy/endpointslicecache.go:34` (EndpointSliceCache),
//! `:69` (NewEndpointSliceCache), `:95` (updatePending),
//! `:122` (checkoutChanges), `:162` (getEndpointsMap).
//!
//! cave's [`EndpointSliceMap`] is a tenant-scoped cache that mirrors the
//! upstream `EndpointSliceCache` minus the apiserver informer plumbing
//! (we receive slice events from cave-apiserver instead).

use crate::error::{KubeProxyError, KubeProxyResult};
use crate::service::ServicePortName;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

/// Cite: `pkg/proxy/endpointslicecache.go:64` (endpointSliceData) +
/// the upstream BaseEndpointInfo (ready/serving/terminating + zone +
/// nodeName), narrowed to what cave needs at the proxier surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub ready: bool,
    pub serving: bool,
    pub terminating: bool,
    pub node_name: Option<String>,
    pub zone: Option<String>,
}

impl EndpointInfo {
    pub fn ready(addr: IpAddr, port: u16) -> Self {
        Self {
            addresses: vec![addr],
            port,
            ready: true,
            serving: true,
            terminating: false,
            node_name: None,
            zone: None,
        }
    }

    /// Cite: `pkg/proxy/endpointslicecache.go:90` (standardEndpointInfo)
    /// — endpoints local to the current node have `node_name == nodeName`.
    pub fn is_local(&self, current_node: &str) -> bool {
        self.node_name.as_deref() == Some(current_node)
    }
}

/// Tenant-scoped cache of EndpointSlices keyed by (Service, slice name).
#[derive(Debug, Clone)]
pub struct EndpointSliceMap {
    pub tenant_id: String,
    /// Outer key = Service; inner key = slice name (for upsert/delete granularity).
    slices: HashMap<ServicePortName, HashMap<String, Vec<EndpointInfo>>>,
}

impl EndpointSliceMap {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self { tenant_id: tenant_id.into(), slices: HashMap::new() }
    }

    /// Cite: `pkg/proxy/endpointslicecache.go:95` (updatePending) — an
    /// upsert overwrites the slice's endpoints atomically; the old
    /// content is discarded.
    pub fn upsert_slice(
        &mut self,
        svc: ServicePortName,
        slice_name: impl Into<String>,
        endpoints: Vec<EndpointInfo>,
    ) {
        self.slices.entry(svc).or_default().insert(slice_name.into(), endpoints);
    }

    /// Cite: `pkg/proxy/endpointslicecache.go:95` (updatePending,
    /// `remove == true` branch) — slice deletion drops just that slice;
    /// other slices on the same Service remain.
    pub fn delete_slice(&mut self, svc: &ServicePortName, slice_name: &str) -> bool {
        if let Some(per_service) = self.slices.get_mut(svc) {
            let removed = per_service.remove(slice_name).is_some();
            if per_service.is_empty() {
                self.slices.remove(svc);
            }
            return removed;
        }
        false
    }

    /// Cite: `pkg/proxy/endpointslicecache.go:162` (getEndpointsMap) —
    /// flatten all slices for a Service into a single endpoint list.
    pub fn endpoints_for(&self, svc: &ServicePortName) -> Vec<&EndpointInfo> {
        self.slices.get(svc).into_iter()
            .flat_map(|per_slice| per_slice.values().flatten())
            .collect()
    }

    /// Cite: `pkg/proxy/endpointslicecache.go:90` (standardEndpointInfo)
    /// + the proxier consumer — only `ready` endpoints are eligible to
    /// receive new connections.
    pub fn ready_endpoints_for(&self, svc: &ServicePortName) -> Vec<&EndpointInfo> {
        self.endpoints_for(svc).into_iter().filter(|e| e.ready).collect()
    }

    pub fn ready_endpoint_count(&self, svc: &ServicePortName) -> usize {
        self.ready_endpoints_for(svc).len()
    }

    /// Cite: `pkg/proxy/topology.go:48` (CategorizeEndpoints) — local
    /// endpoints support externalTrafficPolicy=Local + DSR routing.
    pub fn local_ready_endpoints(&self, svc: &ServicePortName, node: &str) -> Vec<&EndpointInfo> {
        self.ready_endpoints_for(svc).into_iter().filter(|e| e.is_local(node)).collect()
    }

    pub fn services(&self) -> Vec<&ServicePortName> {
        self.slices.keys().collect()
    }

    /// Tenancy guard — callers from a different tenant get a typed error
    /// instead of silent data leakage.
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
