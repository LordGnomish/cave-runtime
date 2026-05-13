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
//! Out of scope (documented in the manifest):
//! * Boltdb persistence — leases live in memory; on restart the
//!   cluster controller is expected to re-register active leases.

pub mod manager;
pub mod resource;

pub use manager::{Lease, LeaseError, LeaseManager};
pub use resource::{Resource, ResourceKind};
