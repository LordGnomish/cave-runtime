// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Out-of-tree provider scaffolds.
//!
//! Each submodule wraps a thin in-memory model of the upstream cloud SDK so
//! the controllers can be unit-tested without network. Real provider clients
//! (calling `hcloud-go` / `azure-sdk-for-go`) will replace these structs as
//! the parity work progresses.

pub mod azure;
pub mod azure_advanced;
pub mod azure_extras;
pub mod azure_networking;
pub mod azure_resources;
pub mod hetzner;
pub mod hetzner_failover;
pub mod hetzner_lb;
pub mod hetzner_lb_advanced;
pub mod hetzner_resources;
pub mod hetzner_targets;
