// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Iceberg SortOrder.
//!
//! Upstream: `crates/iceberg/src/spec/sort_order.rs`
//! Spec: <https://iceberg.apache.org/spec/#sorting>
//!
//! A SortOrder declares how data files within a partition should be
//! sorted. Each sort field references a source-column field-id, a
//! transform (identity / `bucket[N]` / `truncate[W]` / year / month / day /
//! hour / void), a direction, and a null-ordering. SortOrder #0 is
//! the unsorted-default.

use crate::transform::Transform;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NullOrder {
    NullsFirst,
    NullsLast,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SortField {
    #[serde(rename = "source-id")]
    pub source_id: i32,
    pub transform: Transform,
    pub direction: SortDirection,
    #[serde(rename = "null-order")]
    pub null_order: NullOrder,
}

impl SortField {
    pub fn new(source_id: i32, transform: Transform, direction: SortDirection) -> Self {
        let null_order = match direction {
            SortDirection::Asc => NullOrder::NullsFirst,
            SortDirection::Desc => NullOrder::NullsLast,
        };
        Self {
            source_id,
            transform,
            direction,
            null_order,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SortOrder {
    pub order_id: i32,
    pub fields: Vec<SortField>,
}

impl SortOrder {
    /// The Iceberg default — order-id 0, empty fields (unsorted).
    pub fn unsorted() -> Self {
        Self {
            order_id: 0,
            fields: vec![],
        }
    }

    pub fn is_unsorted(&self) -> bool {
        self.fields.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsorted_is_empty() {
        let o = SortOrder::unsorted();
        assert!(o.is_unsorted());
        assert_eq!(o.order_id, 0);
    }

    #[test]
    fn asc_defaults_nulls_first() {
        let f = SortField::new(1, Transform::Identity, SortDirection::Asc);
        assert_eq!(f.null_order, NullOrder::NullsFirst);
    }

    #[test]
    fn desc_defaults_nulls_last() {
        let f = SortField::new(1, Transform::Identity, SortDirection::Desc);
        assert_eq!(f.null_order, NullOrder::NullsLast);
    }

    #[test]
    fn sort_order_serializes_with_kebab_case() {
        let mut o = SortOrder::default();
        o.order_id = 1;
        o.fields
            .push(SortField::new(1, Transform::Identity, SortDirection::Asc));
        let j = serde_json::to_value(&o).unwrap();
        assert_eq!(j["fields"][0]["direction"], "asc");
        assert_eq!(j["fields"][0]["null-order"], "nulls-first");
    }
}
