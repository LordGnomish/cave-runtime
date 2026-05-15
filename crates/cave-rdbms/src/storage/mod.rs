// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory storage backend.

pub mod schema;
pub mod catalog;
pub mod transaction;

pub use schema::{Database, Schema, Table};
pub use catalog::SystemCatalog;
pub use transaction::Transaction;
