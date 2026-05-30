// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Resource leases.
//!
//! Ports `core/leases/` from upstream containerd. A lease is a
//! mutable bag of "I'm using this" references. While a lease holds a
//! [`Resource`] reference, the resource is exempt from garbage
//! collection. cave-cri couples this directly to the content store:
//! when a content blob is added to a lease, [`crate::content::store::LocalStore::mark_in_use`]
//! gets called; when the lease is deleted, [`crate::content::store::LocalStore::release_lease`]
//! is called to undo the in-use ticking.
//!
//! Shape:
//! * [`manager::LeaseManager`] — owns the lease table.
//! * [`resource::Resource`] — typed reference to a content digest or
//!   a snapshot id.
//!
//! Persistence: [`manager::LeaseManager::open`] /
//! [`manager::LeaseManager::open_with_store`] back the table with a
//! JSON file under `<root>/leases.json` — the single-process analog of
//! containerd's bolt `leases` bucket (`core/metadata/leases.go`). Every
//! create / add / remove / delete / reap is flushed atomically, and
//! `open_with_store` rehydrates the content GC interlock on restart so
//! a blob held before a restart still can't be reaped after it. The
//! in-memory [`manager::LeaseManager::new`] path is kept for callers
//! that don't need durability (tests, ephemeral pulls).

pub mod manager;
pub mod resource;

pub use manager::{Lease, LeaseError, LeaseManager};
pub use resource::{Resource, ResourceKind};
