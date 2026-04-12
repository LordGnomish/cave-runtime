//! In-memory store for rollouts, experiments, analysis templates and runs.

use crate::types::{AnalysisRun, AnalysisTemplate, Experiment, Rollout};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Default)]
pub struct RolloutsStore {
    pub rollouts: RwLock<Vec<Rollout>>,
    pub experiments: RwLock<Vec<Experiment>>,
    pub templates: RwLock<Vec<AnalysisTemplate>>,
    pub runs: RwLock<Vec<AnalysisRun>>,
}

impl RolloutsStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Rollouts ──────────────────────────────────────────────────────────────

    pub async fn insert_rollout(&self, r: Rollout) {
        self.rollouts.write().await.push(r);
    }

    pub async fn list_rollouts(&self) -> Vec<Rollout> {
        self.rollouts.read().await.clone()
    }

    pub async fn get_rollout(&self, id: Uuid) -> Option<Rollout> {
        self.rollouts.read().await.iter().find(|r| r.id == id).cloned()
    }

    pub async fn update_rollout(&self, updated: Rollout) -> bool {
        let mut rollouts = self.rollouts.write().await;
        if let Some(r) = rollouts.iter_mut().find(|r| r.id == updated.id) {
            *r = updated;
            true
        } else {
            false
        }
    }

    pub async fn delete_rollout(&self, id: Uuid) -> bool {
        let mut rollouts = self.rollouts.write().await;
        let before = rollouts.len();
        rollouts.retain(|r| r.id != id);
        rollouts.len() < before
    }

    // ── Experiments ───────────────────────────────────────────────────────────

    pub async fn insert_experiment(&self, e: Experiment) {
        self.experiments.write().await.push(e);
    }

    pub async fn list_experiments(&self) -> Vec<Experiment> {
        self.experiments.read().await.clone()
    }

    pub async fn get_experiment(&self, id: Uuid) -> Option<Experiment> {
        self.experiments.read().await.iter().find(|e| e.id == id).cloned()
    }

    // ── Analysis Templates ────────────────────────────────────────────────────

    pub async fn insert_template(&self, t: AnalysisTemplate) {
        self.templates.write().await.push(t);
    }

    pub async fn list_templates(&self) -> Vec<AnalysisTemplate> {
        self.templates.read().await.clone()
    }

    pub async fn get_template(&self, id: Uuid) -> Option<AnalysisTemplate> {
        self.templates.read().await.iter().find(|t| t.id == id).cloned()
    }

    pub async fn get_template_by_name(&self, name: &str) -> Option<AnalysisTemplate> {
        self.templates.read().await.iter().find(|t| t.name == name).cloned()
    }

    // ── Analysis Runs ─────────────────────────────────────────────────────────

    pub async fn insert_run(&self, r: AnalysisRun) {
        self.runs.write().await.push(r);
    }

    pub async fn list_runs(&self) -> Vec<AnalysisRun> {
        self.runs.read().await.clone()
    }

    pub async fn get_run(&self, id: Uuid) -> Option<AnalysisRun> {
        self.runs.read().await.iter().find(|r| r.id == id).cloned()
    }
}
