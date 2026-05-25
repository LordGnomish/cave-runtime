// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use thiserror::Error;

pub type KubeProxyResult<T> = Result<T, KubeProxyError>;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum KubeProxyError {
    /// Mirrors `pkg/registry/core/service/portallocator/allocator.go:43`
    /// (ErrAllocated): the requested port is already taken.
    #[error("port {0} already allocated")]
    PortAlreadyAllocated(u16),

    /// Mirrors `pkg/registry/core/service/portallocator/allocator.go:47`
    /// (ErrNotInRange): the requested port is outside the configured range.
    #[error("port {port} not in range [{min}-{max}]")]
    PortNotInRange { port: u16, min: u16, max: u16 },

    /// Mirrors `pkg/registry/core/service/portallocator/allocator.go:152`
    /// (ErrFull): the entire NodePort range is exhausted.
    #[error("port range exhausted")]
    PortRangeExhausted,

    #[error("invalid CIDR '{0}': {1}")]
    InvalidCidr(String, String),

    #[error(
        "cross-tenant access denied: store tenant '{store}' does not match request tenant '{req}'"
    )]
    CrossTenantDenied { store: String, req: String },

    /// Mirrors `cmd/kube-proxy/app/server.go:412` (validateProxyMode) —
    /// the runtime rejects malformed `KubeProxyConfiguration` at startup.
    #[error("invalid kube-proxy config: {0}")]
    InvalidConfig(String),

    /// Mirrors `pkg/proxy/conntrack/conntrack.go:48` (Exec.Run failure) —
    /// the conntrack helper could not reach `/proc/sys/net/netfilter/`
    /// to apply the requested setting.
    #[error("conntrack setting '{key}' could not be applied: {reason}")]
    ConntrackApply { key: String, reason: String },
}
