// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Index access methods.
//!
//! Pure-Rust reimplementations of the PostgreSQL `pg_am` access methods that
//! live under `src/backend/access/` in upstream. Each module is a self
//! contained, heap-TID-addressed secondary index over [`SqlValue`] keys.
//!
//! [`SqlValue`]: crate::types::SqlValue

use crate::types::SqlValue;
use std::cmp::Ordering;

pub mod btree;

pub use btree::BTreeIndex;

/// Total order over [`SqlValue`] used by the ordered access methods.
///
/// Mirrors the btree default operator class: NULLs sort first, otherwise the
/// values are compared with [`SqlValue::compare`]. When two values are of
/// incomparable types we fall back to their type-tag rank so the ordering
/// stays total (required for a search tree to remain well-formed).
pub(crate) fn key_cmp(a: &SqlValue, b: &SqlValue) -> Ordering {
    a.compare(b).unwrap_or_else(|| type_rank(a).cmp(&type_rank(b)))
}

fn type_rank(v: &SqlValue) -> u8 {
    match v {
        SqlValue::Null => 0,
        SqlValue::Bool(_) => 1,
        SqlValue::Int4(_) => 2,
        SqlValue::Int8(_) => 3,
        SqlValue::Numeric(_) => 4,
        SqlValue::Text(_) => 5,
        SqlValue::Date(_) => 6,
        SqlValue::Timestamp(_) => 7,
    }
}
