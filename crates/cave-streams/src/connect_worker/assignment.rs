//! Distributed task assignment. Mirrors
//! `org.apache.kafka.connect.runtime.distributed.DistributedHerder`
//! from upstream — the herder decides which Worker process
//! owns which task. cave-streams ships an in-memory rendezvous-
//! hash assignment behind the same logical API; the
//! cooperative-rebalance protocol (the herder's actual
//! algorithm) is tracked, not in this batch.
//!
//! The rendezvous hash is deterministic — given the same
//! `(workers, tasks)` set, every Worker computes the same
//! mapping. That's enough for cave-streams' single-broker
//! deployment plus tests; production multi-broker installs need
//! the full rebalance protocol.

use std::collections::{BTreeMap, BTreeSet};

/// Worker process identity.
pub type WorkerId = i32;

/// Outcome of an assignment compute — which tasks moved
/// where, plus which were dropped because their owner left.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rebalance {
    /// Final assignment after the rebalance: task_id → owner.
    pub assignment: BTreeMap<String, WorkerId>,
    /// Tasks that changed owner since the previous assignment.
    pub moved: BTreeSet<String>,
    /// Tasks that have no owner (no live workers).
    pub orphaned: BTreeSet<String>,
}

/// `&self` snapshot of the current assignment. Used by Workers
/// to decide which tasks they own.
#[derive(Debug, Clone, Default)]
pub struct AssignmentTable {
    workers: BTreeSet<WorkerId>,
    /// task_id → current owner.
    assignment: BTreeMap<String, WorkerId>,
}

impl AssignmentTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn workers(&self) -> &BTreeSet<WorkerId> {
        &self.workers
    }

    pub fn add_worker(&mut self, w: WorkerId) {
        self.workers.insert(w);
    }

    pub fn remove_worker(&mut self, w: WorkerId) {
        self.workers.remove(&w);
    }

    /// Tasks currently owned by `worker` — what its runtime
    /// loop should be running.
    pub fn tasks_for(&self, worker: WorkerId) -> Vec<&str> {
        self.assignment
            .iter()
            .filter(|(_, w)| **w == worker)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Owner of `task`, if any.
    pub fn owner(&self, task: &str) -> Option<WorkerId> {
        self.assignment.get(task).copied()
    }

    pub fn total_tasks(&self) -> usize {
        self.assignment.len()
    }

    /// Recompute assignment from the given task set, using a
    /// stable rendezvous-hash decision per task. Returns the
    /// rebalance diff against the previous assignment.
    pub fn rebalance(&mut self, tasks: impl IntoIterator<Item = String>) -> Rebalance {
        let prev = std::mem::take(&mut self.assignment);
        let task_list: BTreeSet<String> = tasks.into_iter().collect();
        let mut new_assignment: BTreeMap<String, WorkerId> = BTreeMap::new();
        let mut orphaned: BTreeSet<String> = BTreeSet::new();
        let mut moved: BTreeSet<String> = BTreeSet::new();

        for task in &task_list {
            match pick_worker(&self.workers, task) {
                Some(w) => {
                    new_assignment.insert(task.clone(), w);
                    if prev.get(task).copied() != Some(w) {
                        moved.insert(task.clone());
                    }
                }
                None => {
                    orphaned.insert(task.clone());
                }
            }
        }

        self.assignment = new_assignment.clone();

        Rebalance {
            assignment: new_assignment,
            moved,
            orphaned,
        }
    }
}

/// Rendezvous-hash worker pick. Each (worker, task) pair maps
/// to a deterministic score; the worker with the highest score
/// wins. Adding or removing a worker shifts O(tasks/n) of the
/// keys — the property that makes rendezvous superior to naive
/// modulo for rebalance churn.
fn pick_worker(workers: &BTreeSet<WorkerId>, task: &str) -> Option<WorkerId> {
    workers
        .iter()
        .map(|w| (rendezvous_score(*w, task), *w))
        .max_by_key(|(s, _)| *s)
        .map(|(_, w)| w)
}

fn rendezvous_score(worker: WorkerId, task: &str) -> u64 {
    // Knuth multiplicative hash composed with the task hash —
    // good-enough deterministic ordering for ~1000s of tasks.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    worker.hash(&mut h);
    task.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tasks(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_workers_orphan_every_task() {
        let mut t = AssignmentTable::new();
        let r = t.rebalance(tasks(&["a", "b"]));
        assert_eq!(r.orphaned.len(), 2);
        assert!(r.assignment.is_empty());
    }

    #[test]
    fn single_worker_gets_every_task() {
        let mut t = AssignmentTable::new();
        t.add_worker(1);
        let r = t.rebalance(tasks(&["a", "b", "c"]));
        assert_eq!(r.assignment.len(), 3);
        assert!(r.assignment.values().all(|w| *w == 1));
        assert!(r.orphaned.is_empty());
    }

    #[test]
    fn rebalance_is_deterministic_for_same_inputs() {
        let mut t1 = AssignmentTable::new();
        t1.add_worker(1);
        t1.add_worker(2);
        t1.add_worker(3);
        let r1 = t1.rebalance(tasks(&["a", "b", "c", "d", "e"]));

        let mut t2 = AssignmentTable::new();
        t2.add_worker(1);
        t2.add_worker(2);
        t2.add_worker(3);
        let r2 = t2.rebalance(tasks(&["a", "b", "c", "d", "e"]));

        assert_eq!(r1.assignment, r2.assignment);
    }

    #[test]
    fn adding_worker_moves_only_some_tasks() {
        let mut t = AssignmentTable::new();
        for w in [1, 2, 3] {
            t.add_worker(w);
        }
        let tasks_v: Vec<String> = (0..30).map(|i| format!("t-{i}")).collect();
        let r1 = t.rebalance(tasks_v.clone());
        // Add a 4th worker — re-distribute.
        t.add_worker(4);
        let r2 = t.rebalance(tasks_v);
        // Some tasks should move to the new worker. Many should not.
        let moved = r2.moved.len();
        assert!(moved > 0, "at least some tasks should move");
        // With ~30/4 ≈ 7-8 expected on worker 4, churn should be
        // a meaningful minority of the total — not the whole set.
        assert!(moved < r2.assignment.len(), "churn should be < total");
    }

    #[test]
    fn removing_worker_redistributes_orphans() {
        let mut t = AssignmentTable::new();
        for w in [1, 2, 3] {
            t.add_worker(w);
        }
        let tasks_v: Vec<String> = (0..30).map(|i| format!("t-{i}")).collect();
        t.rebalance(tasks_v.clone());
        // Drop worker 3 and re-balance.
        t.remove_worker(3);
        let r = t.rebalance(tasks_v);
        // Nothing should be orphaned (workers 1, 2 still cover).
        assert!(r.orphaned.is_empty());
        // Every assignment should now reference 1 or 2.
        for w in r.assignment.values() {
            assert!(*w == 1 || *w == 2);
        }
    }

    #[test]
    fn tasks_for_returns_per_worker_subset() {
        let mut t = AssignmentTable::new();
        for w in [1, 2] {
            t.add_worker(w);
        }
        t.rebalance(tasks(&["a", "b", "c", "d"]));
        let w1 = t.tasks_for(1);
        let w2 = t.tasks_for(2);
        assert_eq!(w1.len() + w2.len(), 4);
    }

    #[test]
    fn owner_resolves_after_rebalance() {
        let mut t = AssignmentTable::new();
        t.add_worker(1);
        t.add_worker(2);
        t.rebalance(tasks(&["a"]));
        let owner = t.owner("a");
        assert!(owner == Some(1) || owner == Some(2));
        assert_eq!(t.owner("never"), None);
    }

    #[test]
    fn rebalance_with_no_tasks_yields_empty() {
        let mut t = AssignmentTable::new();
        t.add_worker(1);
        let r = t.rebalance(std::iter::empty::<String>());
        assert!(r.assignment.is_empty());
    }
}
