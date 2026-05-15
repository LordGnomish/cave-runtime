// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE Tracker — Issue & project tracking engine.
//! Compatible with: Jira
//! Features: Projects, boards, sprints, issues, workflows, custom fields, JQL-like queries.

pub mod models;
pub mod workflow;
pub mod query;
pub mod sprint;
pub mod board;
pub mod fields;
pub mod routes;

use axum::Router;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use models::*;

pub struct TrackerStore {
    pub projects: HashMap<uuid::Uuid, Project>,
    pub issues: HashMap<uuid::Uuid, Issue>,
    pub sprints: HashMap<uuid::Uuid, Sprint>,
    pub boards: HashMap<uuid::Uuid, Board>,
    pub workflows: HashMap<uuid::Uuid, Workflow>,
    pub custom_field_defs: HashMap<uuid::Uuid, CustomFieldDef>,
    pub comments: HashMap<uuid::Uuid, Comment>,
    pub attachments: HashMap<uuid::Uuid, Attachment>,
    pub issue_links: HashMap<uuid::Uuid, IssueLink>,
    pub time_logs: HashMap<uuid::Uuid, TimeLog>,
    pub activity_events: Vec<ActivityEvent>,
    pub notifications: Vec<Notification>,
}

impl Default for TrackerStore {
    fn default() -> Self {
        let mut store = TrackerStore {
            projects: HashMap::new(),
            issues: HashMap::new(),
            sprints: HashMap::new(),
            boards: HashMap::new(),
            workflows: HashMap::new(),
            custom_field_defs: HashMap::new(),
            comments: HashMap::new(),
            attachments: HashMap::new(),
            issue_links: HashMap::new(),
            time_logs: HashMap::new(),
            activity_events: Vec::new(),
            notifications: Vec::new(),
        };
        // Seed default workflow
        let wf = workflow::default_scrum_workflow();
        store.workflows.insert(wf.id, wf);
        let kanban_wf = workflow::default_kanban_workflow();
        store.workflows.insert(kanban_wf.id, kanban_wf);
        store
    }
}

pub struct TrackerState {
    pub store: Arc<RwLock<TrackerStore>>,
}

impl Default for TrackerState {
    fn default() -> Self {
        Self { store: Arc::new(RwLock::new(TrackerStore::default())) }
    }
}

pub fn router(state: Arc<TrackerState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "tracker";
