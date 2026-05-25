// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Table identifier + Table struct.
//!
//! Upstream:
//! * `crates/iceberg/src/table.rs`
//! * `crates/iceberg/src/spec/table_metadata.rs::TableMetadata`

use crate::namespace::NamespaceIdent;
use crate::scan::ScanBuilder;
use crate::table_metadata::TableMetadata;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TableIdent {
    pub namespace: NamespaceIdent,
    pub name: String,
}

impl TableIdent {
    pub fn new(namespace: NamespaceIdent, name: impl Into<String>) -> Self {
        Self {
            namespace,
            name: name.into(),
        }
    }

    pub fn from_dot(dotted: &str) -> Self {
        let mut parts: Vec<String> = dotted.split('.').map(str::to_string).collect();
        let name = parts.pop().unwrap_or_default();
        Self {
            namespace: NamespaceIdent(parts),
            name,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Table {
    pub ident: TableIdent,
    pub metadata: Arc<TableMetadata>,
    /// Pointer to the JSON metadata file the snapshot was loaded from.
    pub metadata_location: Option<String>,
}

impl Table {
    pub fn new(ident: TableIdent, metadata: TableMetadata) -> Self {
        Self {
            ident,
            metadata: Arc::new(metadata),
            metadata_location: None,
        }
    }

    pub fn with_metadata_location(mut self, loc: impl Into<String>) -> Self {
        self.metadata_location = Some(loc.into());
        self
    }

    /// Begin a scan for the current snapshot. The returned builder is
    /// the entrypoint for predicate filtering, snapshot pinning, and
    /// reading file scan tasks.
    pub fn scan(&self) -> ScanBuilder {
        ScanBuilder::new(self.metadata.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;
    use crate::table_metadata::TableMetadataBuilder;

    #[test]
    fn table_ident_from_dot() {
        let t = TableIdent::from_dot("analytics.raw.events");
        assert_eq!(t.namespace.as_dot(), "analytics.raw");
        assert_eq!(t.name, "events");
    }

    #[test]
    fn table_new_carries_metadata() {
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(Schema::default())
            .build()
            .unwrap();
        let t = Table::new(TableIdent::from_dot("ns.t"), m);
        assert_eq!(t.ident.name, "t");
    }

    #[test]
    fn scan_returns_builder() {
        let m = TableMetadataBuilder::new()
            .location("s3://x/t")
            .schema(Schema::default())
            .build()
            .unwrap();
        let t = Table::new(TableIdent::from_dot("t"), m);
        let _builder = t.scan();
        // Compile-time: scan returns a builder
    }
}
