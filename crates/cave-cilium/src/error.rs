// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-cilium error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CiliumError {
    #[error("invalid CIDR: {0}")]
    InvalidCidr(String),

    #[error("invalid IP address: {0}")]
    InvalidIp(String),

    #[error("IPAM pool exhausted (pool={pool}, capacity={capacity})")]
    PoolExhausted { pool: String, capacity: u32 },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("already exists: {0}")]
    AlreadyExists(String),

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error("identity allocator: {0}")]
    Identity(String),

    #[error("BGP: {0}")]
    Bgp(String),

    #[error("Hubble: {0}")]
    Hubble(String),

    #[error("CNI: {0}")]
    Cni(String),

    #[error("conntrack: {0}")]
    Conntrack(String),

    #[error("WireGuard: {0}")]
    WireGuard(String),

    #[error("clustermesh: {0}")]
    ClusterMesh(String),

    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, CiliumError>;
