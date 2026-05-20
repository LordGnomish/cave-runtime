// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Index metadata for the CRM store.
//!
//! Twenty's TypeORM migrations declare a fixed set of secondary indexes
//! on the standard objects (e.g. `person.email`, `company.domain_name`,
//! `opportunity.pipeline_step_id`). The in-memory store doesn't *need*
//! those indexes for correctness, but tracking them in metadata lets the
//! v0.2 Postgres-backed store rebuild them on schema sync. The metadata
//! itself is the same enumeration upstream uses — see
//! `packages/twenty-server/src/engine/workspace-manager/workspace-migration-runner/`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IndexKind {
    BTree,
    Hash,
    /// Composite (multi-column) BTree.
    Composite,
    /// Unique constraint.
    Unique,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexSpec {
    pub object_singular: String,
    pub columns: Vec<String>,
    pub kind: IndexKind,
}

impl IndexSpec {
    pub fn name(&self) -> String {
        format!("idx_{}_{}", self.object_singular, self.columns.join("_"))
    }
}

#[derive(Debug, Default)]
pub struct IndexSet {
    /// per-workspace index list (workspace_id → list of index specs).
    pub by_workspace: BTreeMap<Uuid, Vec<IndexSpec>>,
}

impl IndexSet {
    /// The default index list Twenty migrations emit on workspace init.
    /// Captured here as the canonical list to seed for new workspaces.
    pub fn default_specs() -> Vec<IndexSpec> {
        vec![
            IndexSpec { object_singular: "person".into(), columns: vec!["workspace_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "person".into(), columns: vec!["email".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "person".into(), columns: vec!["company_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "company".into(), columns: vec!["workspace_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "company".into(), columns: vec!["domain_name".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "opportunity".into(), columns: vec!["workspace_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "opportunity".into(), columns: vec!["pipeline_step_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "opportunity".into(), columns: vec!["company_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "opportunity".into(), columns: vec!["status".into(), "close_date".into()], kind: IndexKind::Composite },
            IndexSpec { object_singular: "task".into(), columns: vec!["workspace_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "task".into(), columns: vec!["assignee_user_id".into(), "status".into()], kind: IndexKind::Composite },
            IndexSpec { object_singular: "note".into(), columns: vec!["workspace_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "activity_target".into(), columns: vec!["activity_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "activity_target".into(), columns: vec!["target_id".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "calendar_event".into(), columns: vec!["starts_at".into()], kind: IndexKind::BTree },
            IndexSpec { object_singular: "api_key".into(), columns: vec!["secret_hash".into()], kind: IndexKind::Unique },
            IndexSpec { object_singular: "workspace_member".into(), columns: vec!["user_id".into(), "workspace_id".into()], kind: IndexKind::Unique },
            IndexSpec { object_singular: "user".into(), columns: vec!["email".into()], kind: IndexKind::Unique },
        ]
    }

    pub fn seed_default_for_workspace(&mut self, workspace_id: Uuid) {
        self.by_workspace.insert(workspace_id, Self::default_specs());
    }

    /// Lookup the index list for a workspace; empty if unseeded.
    pub fn list(&self, workspace_id: Uuid) -> &[IndexSpec] {
        self.by_workspace
            .get(&workspace_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_specs_contains_critical_indexes() {
        let s = IndexSet::default_specs();
        assert!(s.iter().any(|i| i.object_singular == "person" && i.columns == ["email"]));
        assert!(s.iter().any(|i| i.object_singular == "user" && i.kind == IndexKind::Unique));
        assert!(s.iter().any(|i| i.object_singular == "opportunity" && i.columns == ["status", "close_date"]));
    }

    #[test]
    fn index_name_is_predictable() {
        let i = IndexSpec {
            object_singular: "opportunity".into(),
            columns: vec!["status".into(), "close_date".into()],
            kind: IndexKind::Composite,
        };
        assert_eq!(i.name(), "idx_opportunity_status_close_date");
    }

    #[test]
    fn seed_default_for_workspace_populates() {
        let mut s = IndexSet::default();
        let ws = Uuid::new_v4();
        s.seed_default_for_workspace(ws);
        assert!(!s.list(ws).is_empty());
        assert!(s.list(Uuid::new_v4()).is_empty());
    }
}
