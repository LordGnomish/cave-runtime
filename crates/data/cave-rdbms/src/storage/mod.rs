// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory storage backend.

pub mod catalog;
pub mod extension;
pub mod heap;
pub mod index;
pub mod mvcc;
pub mod replication;
pub mod schema;
pub mod transaction;
pub mod wal;

pub use catalog::SystemCatalog;
pub use extension::{ExtensionControl, ExtensionRegistry};
pub use heap::{HeapPage, ItemId, ItemPointer};
pub use index::{BTreeIndex, HashIndex};
pub use replication::{ReorderBuffer, ReplicationSlots, SlotKind, StandbyFeedback};
pub use mvcc::{satisfies_mvcc, Clog, HeapTuple, Snapshot};
pub use wal::{Wal, WalRecord};
pub use schema::{Database, Schema, Table};
pub use transaction::Transaction;
