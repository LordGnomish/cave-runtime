// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

use crate::models::{ChaosExperiment, ExperimentSchedule, ExperimentStatus};

pub struct ChaosStore {
    experiments: RwLock<HashMap<Uuid, ChaosExperiment>>,
    schedules: RwLock<HashMap<Uuid, ExperimentSchedule>>,
}

impl ChaosStore {
    pub fn new() -> Self {
        ChaosStore {
            experiments: RwLock::new(HashMap::new()),
            schedules: RwLock::new(HashMap::new()),
        }
    }

    pub fn insert(&self, exp: ChaosExperiment) {
        self.experiments.write().unwrap().insert(exp.id, exp);
    }

    pub fn get(&self, id: Uuid) -> Option<ChaosExperiment> {
        self.experiments.read().unwrap().get(&id).cloned()
    }

    /// Update an existing experiment. Returns `true` if the experiment existed.
    pub fn update(&self, exp: ChaosExperiment) -> bool {
        let mut map = self.experiments.write().unwrap();
        if map.contains_key(&exp.id) {
            map.insert(exp.id, exp);
            true
        } else {
            false
        }
    }

    pub fn remove(&self, id: Uuid) -> Option<ChaosExperiment> {
        self.experiments.write().unwrap().remove(&id)
    }

    pub fn list(&self) -> Vec<ChaosExperiment> {
        self.experiments.read().unwrap().values().cloned().collect()
    }

    pub fn list_by_status(&self, status: &ExperimentStatus) -> Vec<ChaosExperiment> {
        self.experiments
            .read()
            .unwrap()
            .values()
            .filter(|e| &e.status == status)
            .cloned()
            .collect()
    }

    pub fn add_schedule(&self, schedule: ExperimentSchedule) {
        self.schedules.write().unwrap().insert(schedule.id, schedule);
    }

    pub fn get_schedule(&self, id: Uuid) -> Option<ExperimentSchedule> {
        self.schedules.read().unwrap().get(&id).cloned()
    }

    pub fn remove_schedule(&self, id: Uuid) -> Option<ExperimentSchedule> {
        self.schedules.write().unwrap().remove(&id)
    }

    pub fn list_schedules(&self) -> Vec<ExperimentSchedule> {
        self.schedules.read().unwrap().values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.experiments.read().unwrap().len()
    }
}

impl Default for ChaosStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BlastRadius, ChaosTarget, ExperimentParams, ExperimentType, SafetyGuard};
    use std::collections::HashMap;
    use uuid::Uuid;
    use chrono::Utc;

    fn make_experiment(status: ExperimentStatus) -> ChaosExperiment {
        ChaosExperiment {
            id: Uuid::new_v4(),
            name: "store-test".to_string(),
            experiment_type: ExperimentType::PodKill,
            target: ChaosTarget {
                namespace: "staging".to_string(),
                selector: HashMap::new(),
                pod_count: None,
            },
            parameters: ExperimentParams {
                latency_ms: None,
                packet_loss_percent: None,
                cpu_load_percent: None,
                memory_mb: None,
            },
            status,
            created_at: Utc::now(),
            started_at: None,
            ended_at: None,
            duration_secs: 30,
            blast_radius: BlastRadius::default(),
            safety_guard: SafetyGuard::default(),
            result: None,
            annotations: HashMap::new(),
        }
    }

    #[test]
    fn test_insert_and_get() {
        let store = ChaosStore::new();
        let exp = make_experiment(ExperimentStatus::Draft);
        let id = exp.id;
        store.insert(exp.clone());
        let retrieved = store.get(id).unwrap();
        assert_eq!(retrieved.id, id);
    }

    #[test]
    fn test_get_nonexistent() {
        let store = ChaosStore::new();
        assert!(store.get(Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_update_existing() {
        let store = ChaosStore::new();
        let mut exp = make_experiment(ExperimentStatus::Draft);
        store.insert(exp.clone());
        exp.status = ExperimentStatus::Running;
        assert!(store.update(exp.clone()));
        assert_eq!(store.get(exp.id).unwrap().status, ExperimentStatus::Running);
    }

    #[test]
    fn test_update_nonexistent_returns_false() {
        let store = ChaosStore::new();
        let exp = make_experiment(ExperimentStatus::Draft);
        assert!(!store.update(exp));
    }

    #[test]
    fn test_remove() {
        let store = ChaosStore::new();
        let exp = make_experiment(ExperimentStatus::Draft);
        let id = exp.id;
        store.insert(exp);
        assert!(store.remove(id).is_some());
        assert!(store.get(id).is_none());
    }

    #[test]
    fn test_remove_nonexistent() {
        let store = ChaosStore::new();
        assert!(store.remove(Uuid::new_v4()).is_none());
    }

    #[test]
    fn test_list_all() {
        let store = ChaosStore::new();
        store.insert(make_experiment(ExperimentStatus::Draft));
        store.insert(make_experiment(ExperimentStatus::Running));
        store.insert(make_experiment(ExperimentStatus::Completed));
        assert_eq!(store.list().len(), 3);
    }

    #[test]
    fn test_list_by_status() {
        let store = ChaosStore::new();
        store.insert(make_experiment(ExperimentStatus::Draft));
        store.insert(make_experiment(ExperimentStatus::Draft));
        store.insert(make_experiment(ExperimentStatus::Running));
        let drafts = store.list_by_status(&ExperimentStatus::Draft);
        assert_eq!(drafts.len(), 2);
        let running = store.list_by_status(&ExperimentStatus::Running);
        assert_eq!(running.len(), 1);
    }

    #[test]
    fn test_count() {
        let store = ChaosStore::new();
        assert_eq!(store.count(), 0);
        store.insert(make_experiment(ExperimentStatus::Draft));
        assert_eq!(store.count(), 1);
        store.insert(make_experiment(ExperimentStatus::Running));
        assert_eq!(store.count(), 2);
    }

    #[test]
    fn test_add_and_list_schedules() {
        let store = ChaosStore::new();
        let schedule = ExperimentSchedule {
            id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            cron_expression: "0 2 * * 1".to_string(),
            enabled: true,
            last_run: None,
            next_run: None,
            max_runs: Some(10),
            run_count: 0,
        };
        store.add_schedule(schedule.clone());
        let schedules = store.list_schedules();
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].cron_expression, "0 2 * * 1");
    }

    #[test]
    fn test_remove_schedule() {
        let store = ChaosStore::new();
        let schedule = ExperimentSchedule {
            id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            cron_expression: "0 * * * *".to_string(),
            enabled: true,
            last_run: None,
            next_run: None,
            max_runs: None,
            run_count: 0,
        };
        let sid = schedule.id;
        store.add_schedule(schedule);
        assert!(store.remove_schedule(sid).is_some());
        assert!(store.list_schedules().is_empty());
    }

    #[test]
    fn test_get_schedule() {
        let store = ChaosStore::new();
        let schedule = ExperimentSchedule {
            id: Uuid::new_v4(),
            experiment_id: Uuid::new_v4(),
            cron_expression: "0 0 * * *".to_string(),
            enabled: false,
            last_run: None,
            next_run: None,
            max_runs: None,
            run_count: 5,
        };
        let sid = schedule.id;
        store.add_schedule(schedule);
        let s = store.get_schedule(sid).unwrap();
        assert_eq!(s.run_count, 5);
        assert!(!s.enabled);
    }
}
