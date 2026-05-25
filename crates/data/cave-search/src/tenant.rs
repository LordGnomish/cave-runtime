// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tenant identity primitives.
//!
//! `TenantId` is re-exported from `cave_kernel::ns` (sweep-002 F2-G adoption,
//! 2026-05-01) so cave-search shares the canonical DNS-1123-validated newtype
//! used by the rest of the platform. The local `Tenant` aggregate stays here.
//!
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/cluster/metadata/

pub use cave_kernel::ns::TenantId;

#[derive(Debug, Clone)]
pub struct Tenant {
    pub id: TenantId,
}
