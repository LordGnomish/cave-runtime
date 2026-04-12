//! Issue tracking & project management — replaces Jira/Linear/Plane.
//!
//! Replaces: Jira, Linear, Plane
//! Developer-first issue tracking: sprints, kanban, roadmaps, automation.

pub mod automation;
pub mod board;
pub mod models;
pub mod roadmap;
pub mod routes;
pub mod tracker;

use axum::Router;
use models::{Activity, Automation, BacklogItem, Board, Comment, Issue, Label, Project, Roadmap, Sprint, Workflow};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// All in-memory state for the tracker module.
/// Each collection is independently locked for fine-grained concurrency.
pub struct TrackerState {
    pub projects: Mutex<HashMap<Uuid, Project>>,
    pub issues: Mutex<HashMap<Uuid, Issue>>,
    pub sprints: Mutex<HashMap<Uuid, Sprint>>,
    pub boards: Mutex<HashMap<Uuid, Board>>,
    pub workflows: Mutex<HashMap<Uuid, Workflow>>,
    pub comments: Mutex<HashMap<Uuid, Vec<Comment>>>,
    pub activities: Mutex<HashMap<Uuid, Vec<Activity>>>,
    pub labels: Mutex<HashMap<Uuid, Label>>,
    pub automations: Mutex<Vec<Automation>>,
    /// project_id → prioritized backlog (sorted by rank ascending).
    pub backlogs: Mutex<HashMap<Uuid, Vec<BacklogItem>>>,
    pub roadmaps: Mutex<HashMap<Uuid, Roadmap>>,
    /// project_id → next issue sequence number.
    pub issue_counters: Mutex<HashMap<Uuid, u64>>,
}

impl Default for TrackerState {
    fn default() -> Self {
        Self {
            projects: Mutex::new(HashMap::new()),
            issues: Mutex::new(HashMap::new()),
            sprints: Mutex::new(HashMap::new()),
            boards: Mutex::new(HashMap::new()),
            workflows: Mutex::new(HashMap::new()),
            comments: Mutex::new(HashMap::new()),
            activities: Mutex::new(HashMap::new()),
            labels: Mutex::new(HashMap::new()),
            automations: Mutex::new(Vec::new()),
            backlogs: Mutex::new(HashMap::new()),
            roadmaps: Mutex::new(HashMap::new()),
            issue_counters: Mutex::new(HashMap::new()),
        }
    }
}

pub fn router(state: Arc<TrackerState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "tracker";
