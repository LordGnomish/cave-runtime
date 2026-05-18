// SPDX-License-Identifier: AGPL-3.0-or-later
//! NodePort allocator — bitmap-style allocation over the Kubernetes
//! default NodePort range.
//!
//! Cite: `pkg/registry/core/service/portallocator/allocator.go:31`
//! (Interface), `:55` (PortAllocator), `:123` (Allocate), `:156`
//! (AllocateNext), `:185` (Release), `:203` (Has).
//!
//! Default range: 30000-32767 (size 2768) — see Kubernetes
//! `cmd/kube-apiserver/app/options/options.go` `DefaultServiceNodePortRange`.
//!
//! Tenant-scoped: each tenant gets a private allocator so two tenants can
//! both hold port 31000 without colliding at the cave layer (collision
//! resolution at the host iptables/nftables layer is the proxier's job).

use crate::error::{KubeProxyError, KubeProxyResult};
use std::collections::BTreeSet;

pub const DEFAULT_MIN_NODE_PORT: u16 = 30_000;
pub const DEFAULT_MAX_NODE_PORT: u16 = 32_767;

#[derive(Debug, Clone)]
pub struct NodePortAllocator {
    pub tenant_id: String,
    pub min: u16,
    pub max: u16,
    allocated: BTreeSet<u16>,
}

impl NodePortAllocator {
    /// Default 30000-32767 range — see Kubernetes
    /// `cmd/kube-apiserver/app/options/options.go` (DefaultServiceNodePortRange).
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self::with_range(tenant_id, DEFAULT_MIN_NODE_PORT, DEFAULT_MAX_NODE_PORT)
            .expect("default range is well-formed")
    }

    pub fn with_range(tenant_id: impl Into<String>, min: u16, max: u16) -> KubeProxyResult<Self> {
        if min == 0 || max < min {
            return Err(KubeProxyError::PortNotInRange { port: min, min, max });
        }
        Ok(Self { tenant_id: tenant_id.into(), min, max, allocated: BTreeSet::new() })
    }

    /// Cite: `pkg/registry/core/service/portallocator/allocator.go:123`
    /// (Allocate) — explicit port allocation: out-of-range returns
    /// ErrNotInRange, double-allocate returns ErrAllocated.
    pub fn allocate(&mut self, port: u16) -> KubeProxyResult<()> {
        if port < self.min || port > self.max {
            return Err(KubeProxyError::PortNotInRange { port, min: self.min, max: self.max });
        }
        if !self.allocated.insert(port) {
            return Err(KubeProxyError::PortAlreadyAllocated(port));
        }
        Ok(())
    }

    /// Cite: `pkg/registry/core/service/portallocator/allocator.go:156`
    /// (AllocateNext) — auto-allocate the lowest free port; ErrFull when
    /// the range is exhausted.
    pub fn allocate_next(&mut self) -> KubeProxyResult<u16> {
        for port in self.min..=self.max {
            if !self.allocated.contains(&port) {
                self.allocated.insert(port);
                return Ok(port);
            }
        }
        Err(KubeProxyError::PortRangeExhausted)
    }

    /// Cite: `pkg/registry/core/service/portallocator/allocator.go:185`
    /// (Release) — releases a previously-allocated port. Releasing an
    /// unallocated port is a no-op.
    pub fn release(&mut self, port: u16) -> bool {
        self.allocated.remove(&port)
    }

    /// Cite: `pkg/registry/core/service/portallocator/allocator.go:203` (Has).
    pub fn has(&self, port: u16) -> bool {
        self.allocated.contains(&port)
    }

    /// Cite: `pkg/registry/core/service/portallocator/allocator.go:115` (Used).
    pub fn used(&self) -> usize {
        self.allocated.len()
    }

    /// Cite: `pkg/registry/core/service/portallocator/allocator.go:110` (Free).
    pub fn free(&self) -> usize {
        (self.max as usize - self.min as usize + 1) - self.allocated.len()
    }

    pub fn capacity(&self) -> usize {
        self.max as usize - self.min as usize + 1
    }
}
