// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Sales-pipeline stage definition.
//!
//! Twenty upstream:
//! `packages/twenty-server/src/modules/opportunity/standard-objects/pipeline-step.workspace-entity.ts`
//!
//! In Twenty `PipelineStep` is the per-workspace stage list backing
//! the opportunity kanban (`New` / `Screening` / `Meeting` / `Proposal` /
//! `Customer`). Each Opportunity carries `pipelineStepId` and gets
//! grouped into kanban lanes by that FK.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineStep {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    /// Visual lane color. Twenty exposes `pink`/`purple`/`sky`/`turquoise`/
    /// `yellow`/`orange`/`red`/`green` as tag values. We store the raw
    /// upstream slug so the wire is round-trippable.
    pub color: String,
    /// Order in the kanban (left → right).
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PipelineStep {
    pub fn new(workspace_id: Uuid, name: impl Into<String>, position: i64) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            color: "sky".to_string(),
            position,
            created_at: now,
            updated_at: now,
        }
    }

    /// The five standard stages Twenty seeds on new-workspace setup
    /// (`packages/twenty-server/src/modules/.../workspace-init.service.ts`).
    pub fn defaults(workspace_id: Uuid) -> Vec<Self> {
        ["New", "Screening", "Meeting", "Proposal", "Customer"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| Self::new(workspace_id, name, i as i64))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_seeds_five_stages_in_order() {
        let steps = PipelineStep::defaults(Uuid::nil());
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0].name, "New");
        assert_eq!(steps[4].name, "Customer");
        for (i, s) in steps.iter().enumerate() {
            assert_eq!(s.position, i as i64);
        }
    }
}
