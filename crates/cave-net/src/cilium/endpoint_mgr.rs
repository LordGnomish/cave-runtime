// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! EndpointManager — in-memory Endpoint registry.
//!
//! Mirrors `pkg/endpointmanager/endpointsynchronizer.go` (the
//! synchroniser that materialises endpoint identity onto the BPF maps),
//! `pkg/endpointmanager/gc.go` (the periodic sweep), plus the registry
//! itself.
//!
//! `endpoint.rs` already ports the `Endpoint` shape; this module owns
//! the *manager* surface — lookup, lifecycle events, GC pass.

use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;
use std::time::Duration;

/// GC interval — how often dead endpoints are reaped. Mirrors the
/// upstream `gcInterval` constant.
pub const GC_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointState {
    Restoring,
    Ready,
    WaitForIdentity,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointHandle {
    pub id: u64,
    pub identity: u32,
    pub state: EndpointState,
    pub container_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EndpointMgrError {
    #[error("endpoint id {0} not found")]
    NotFound(u64),
    #[error("tenant {tenant} cannot mutate endpoint manager owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct EndpointManager {
    pub tenant: TenantId,
    by_id: BTreeMap<u64, EndpointHandle>,
    by_container: BTreeMap<String, u64>,
}

impl EndpointManager {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, by_id: BTreeMap::new(), by_container: BTreeMap::new() }
    }

    pub fn add(&mut self, ep: EndpointHandle) {
        self.by_container.insert(ep.container_id.clone(), ep.id);
        self.by_id.insert(ep.id, ep);
    }

    pub fn lookup(&self, id: u64) -> Option<&EndpointHandle> { self.by_id.get(&id) }
    pub fn lookup_by_container(&self, c: &str) -> Option<&EndpointHandle> {
        self.by_container.get(c).and_then(|id| self.by_id.get(id))
    }

    pub fn set_state(&mut self, id: u64, st: EndpointState) -> Result<(), EndpointMgrError> {
        let ep = self.by_id.get_mut(&id).ok_or(EndpointMgrError::NotFound(id))?;
        ep.state = st;
        Ok(())
    }

    /// Sweep one GC pass — drop endpoints in `Disconnected`. Mirrors the
    /// behaviour of `pkg/endpointmanager/gc.go` "delete after disconnect".
    pub fn gc_once(&mut self) -> usize {
        let to_drop: Vec<u64> = self.by_id.values()
            .filter(|e| e.state == EndpointState::Disconnected)
            .map(|e| e.id)
            .collect();
        for id in &to_drop {
            if let Some(ep) = self.by_id.remove(id) {
                self.by_container.remove(&ep.container_id);
            }
        }
        to_drop.len()
    }

    pub fn len(&self) -> usize { self.by_id.len() }
    pub fn is_empty(&self) -> bool { self.by_id.is_empty() }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/endpointmanager/endpointsynchronizer.go", "EndpointSynchronizer");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ep(id: u64, c: &str, state: EndpointState) -> EndpointHandle {
        EndpointHandle { id, identity: 1024 + id as u32, state, container_id: c.into() }
    }

    #[test]
    fn gc_interval_is_five_minutes() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpointmanager/gc.go", "GC.Interval", "tenant-em-gi");
        assert_eq!(GC_INTERVAL, Duration::from_secs(300));
    }

    #[test]
    fn add_then_lookup_returns_endpoint() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/endpointsynchronizer.go", "Lookup", "tenant-em-l");
        let mut m = EndpointManager::new(t);
        m.add(ep(1, "c1", EndpointState::Ready));
        assert_eq!(m.lookup(1).unwrap().identity, 1025);
    }

    #[test]
    fn lookup_by_container_id() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/endpointsynchronizer.go", "LookupByContainer", "tenant-em-lc");
        let mut m = EndpointManager::new(t);
        m.add(ep(2, "ctr-abc", EndpointState::Ready));
        let found = m.lookup_by_container("ctr-abc").unwrap();
        assert_eq!(found.id, 2);
    }

    #[test]
    fn set_state_unknown_endpoint_errors() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/endpointsynchronizer.go", "SetState.Miss", "tenant-em-ssm");
        let mut m = EndpointManager::new(t);
        let e = m.set_state(99, EndpointState::Ready).unwrap_err();
        assert_eq!(e, EndpointMgrError::NotFound(99));
    }

    #[test]
    fn gc_drops_disconnected_endpoints() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/gc.go", "GC.Drop", "tenant-em-gd");
        let mut m = EndpointManager::new(t);
        m.add(ep(1, "c1", EndpointState::Ready));
        m.add(ep(2, "c2", EndpointState::Disconnected));
        m.add(ep(3, "c3", EndpointState::Disconnected));
        assert_eq!(m.gc_once(), 2);
        assert_eq!(m.len(), 1);
        assert!(m.lookup(1).is_some());
    }

    #[test]
    fn gc_clears_container_index_on_drop() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/gc.go", "GC.Index", "tenant-em-gi2");
        let mut m = EndpointManager::new(t);
        m.add(ep(1, "c1", EndpointState::Disconnected));
        assert_eq!(m.gc_once(), 1);
        assert!(m.lookup_by_container("c1").is_none());
    }

    #[test]
    fn set_state_transitions_through_lifecycle() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/endpointsynchronizer.go", "Lifecycle", "tenant-em-lf");
        let mut m = EndpointManager::new(t);
        m.add(ep(1, "c1", EndpointState::Restoring));
        m.set_state(1, EndpointState::WaitForIdentity).unwrap();
        m.set_state(1, EndpointState::Ready).unwrap();
        m.set_state(1, EndpointState::Disconnecting).unwrap();
        m.set_state(1, EndpointState::Disconnected).unwrap();
        assert_eq!(m.lookup(1).unwrap().state, EndpointState::Disconnected);
    }

    #[test]
    fn empty_manager_reports_zero() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/endpointsynchronizer.go", "Empty", "tenant-em-e");
        let m = EndpointManager::new(t);
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn gc_with_no_disconnected_endpoints_returns_zero() {
        let (_c, t) = cilium_test_ctx!("pkg/endpointmanager/gc.go", "GC.NoOp", "tenant-em-gn");
        let mut m = EndpointManager::new(t);
        m.add(ep(1, "c1", EndpointState::Ready));
        assert_eq!(m.gc_once(), 0);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn endpoint_mgr_error_renders() {
        let (_c, _t) = cilium_test_ctx!("pkg/endpointmanager/endpointsynchronizer.go", "Errors", "tenant-em-er");
        let e = EndpointMgrError::NotFound(7);
        assert!(format!("{}", e).contains("7"));
        let e = EndpointMgrError::TenantDenied { tenant: TenantId::new("t").expect("test fixture") };
        assert!(format!("{}", e).contains("t"));
    }
}
