//! Out-of-tree provider scaffolds.
//!
//! Each submodule wraps a thin in-memory model of the upstream cloud SDK so
//! the controllers can be unit-tested without network. Real provider clients
//! (calling `hcloud-go` / `azure-sdk-for-go`) will replace these structs as
//! the parity work progresses.

pub mod azure;
pub mod hetzner;
