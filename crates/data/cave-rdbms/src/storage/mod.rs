// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory storage backend.

pub mod catalog;
pub mod index;
pub mod schema;
pub mod transaction;

pub use catalog::SystemCatalog;
pub use index::BTreeIndex;
pub use schema::{Database, Schema, Table};
pub use transaction::Transaction;
