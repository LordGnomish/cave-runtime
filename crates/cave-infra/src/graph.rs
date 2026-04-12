//! Dependency graph and topological execution ordering for infra resources.

use std::collections::{HashMap, HashSet, VecDeque};

// ── GraphError ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum GraphError {
    #[error("Cycle detected involving resource: {0}")]
    CycleDetected(String),
    #[error("Unknown dependency: {0}")]
    UnknownDependency(String),
}

// ── DependencyGraph ───────────────────────────────────────────────────────────

/// Directed acyclic graph of infrastructure resource dependencies.
pub struct DependencyGraph {
    /// resource_id → set of resource_ids it depends on (edges point *to* dependencies).
    deps: HashMap<String, HashSet<String>>,
    /// resource_id → set of resource_ids that depend on *it* (reverse edges).
    rdeps: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            deps: HashMap::new(),
            rdeps: HashMap::new(),
        }
    }

    /// Register a resource and its direct dependencies.
    ///
    /// Returns `Err(UnknownDependency)` if any dependency hasn't been added yet.
    pub fn add(&mut self, id: &str, depends_on: Vec<&str>) -> Result<(), GraphError> {
        // Validate all dependencies exist before mutating state.
        for dep in &depends_on {
            if !self.deps.contains_key(*dep) {
                return Err(GraphError::UnknownDependency(dep.to_string()));
            }
        }

        // Ensure every node has an entry in both maps.
        self.deps.entry(id.to_string()).or_insert_with(HashSet::new);
        self.rdeps.entry(id.to_string()).or_insert_with(HashSet::new);

        for dep in depends_on {
            self.deps
                .get_mut(id)
                .unwrap()
                .insert(dep.to_string());
            self.rdeps
                .entry(dep.to_string())
                .or_insert_with(HashSet::new)
                .insert(id.to_string());
        }
        Ok(())
    }

    /// Kahn's algorithm — returns resources in creation order (dependencies first).
    pub fn topo_sort(&self) -> Result<Vec<String>, GraphError> {
        // in_degree[id] = number of unresolved dependencies.
        let mut in_degree: HashMap<&str, usize> = self
            .deps
            .iter()
            .map(|(id, deps)| (id.as_str(), deps.len()))
            .collect();

        // Queue starts with all nodes that have no dependencies.
        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|&(_, d)| *d == 0)
            .map(|(&id, _)| id)
            .collect();
        // Sort for determinism in tests.
        let mut queue_vec: Vec<&str> = queue.drain(..).collect();
        queue_vec.sort_unstable();
        queue.extend(queue_vec);

        let mut order: Vec<String> = Vec::with_capacity(self.deps.len());

        while let Some(id) = queue.pop_front() {
            order.push(id.to_string());

            if let Some(dependents) = self.rdeps.get(id) {
                let mut newly_free: Vec<&str> = Vec::new();
                for dep_id in dependents {
                    let entry = in_degree.get_mut(dep_id.as_str()).unwrap();
                    *entry -= 1;
                    if *entry == 0 {
                        newly_free.push(dep_id.as_str());
                    }
                }
                // Sort newly freed nodes for determinism.
                newly_free.sort_unstable();
                queue.extend(newly_free);
            }
        }

        if order.len() != self.deps.len() {
            // Some node was never freed → cycle.
            let cycled = self
                .deps
                .keys()
                .find(|id| !order.contains(*id))
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            return Err(GraphError::CycleDetected(cycled));
        }

        Ok(order)
    }

    /// Group resources into parallel batches (levels of the DAG).
    ///
    /// All resources in a batch can be provisioned simultaneously because they
    /// share no intra-batch dependencies.
    pub fn parallel_batches(&self) -> Result<Vec<Vec<String>>, GraphError> {
        let mut in_degree: HashMap<&str, usize> = self
            .deps
            .iter()
            .map(|(id, deps)| (id.as_str(), deps.len()))
            .collect();

        let mut batches: Vec<Vec<String>> = Vec::new();

        loop {
            let mut current_batch: Vec<&str> = in_degree
                .iter()
                .filter(|&(_, d)| *d == 0)
                .map(|(&id, _)| id)
                .collect();

            if current_batch.is_empty() {
                break;
            }

            current_batch.sort_unstable();

            // Remove processed nodes from in_degree before computing the next batch.
            for id in &current_batch {
                in_degree.remove(*id);
                if let Some(dependents) = self.rdeps.get(*id) {
                    for dep_id in dependents {
                        if let Some(entry) = in_degree.get_mut(dep_id.as_str()) {
                            if *entry > 0 {
                                *entry -= 1;
                            }
                        }
                    }
                }
            }

            batches.push(current_batch.iter().map(|s| s.to_string()).collect());
        }

        if batches.iter().map(|b| b.len()).sum::<usize>() != self.deps.len() {
            let processed: HashSet<String> =
                batches.iter().flatten().cloned().collect();
            let cycled = self
                .deps
                .keys()
                .find(|id| !processed.contains(*id))
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());
            return Err(GraphError::CycleDetected(cycled));
        }

        Ok(batches)
    }

    /// Compute the set of all transitive dependencies of `id`.
    pub fn transitive_deps(&self, id: &str) -> HashSet<String> {
        let mut visited = HashSet::new();
        let mut stack = vec![id.to_string()];

        while let Some(current) = stack.pop() {
            if let Some(direct) = self.deps.get(&current) {
                for dep in direct {
                    if visited.insert(dep.clone()) {
                        stack.push(dep.clone());
                    }
                }
            }
        }

        visited
    }

    pub fn len(&self) -> usize {
        self.deps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.deps.is_empty()
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple graph:
    ///   vpc → (none)
    ///   subnet → vpc
    ///   vm → subnet
    fn simple_graph() -> DependencyGraph {
        let mut g = DependencyGraph::new();
        g.add("vpc", vec![]).unwrap();
        g.add("subnet", vec!["vpc"]).unwrap();
        g.add("vm", vec!["subnet"]).unwrap();
        g
    }

    #[test]
    fn test_topo_sort_simple() {
        let g = simple_graph();
        let order = g.topo_sort().unwrap();

        assert_eq!(order.len(), 3);
        // vpc must come before subnet, subnet before vm.
        let vpc_pos = order.iter().position(|x| x == "vpc").unwrap();
        let subnet_pos = order.iter().position(|x| x == "subnet").unwrap();
        let vm_pos = order.iter().position(|x| x == "vm").unwrap();

        assert!(vpc_pos < subnet_pos);
        assert!(subnet_pos < vm_pos);
    }

    #[test]
    fn test_parallel_batches() {
        // vpc + dns have no deps → batch 0
        // subnet depends on vpc → batch 1
        // vm depends on subnet → batch 2
        let mut g = DependencyGraph::new();
        g.add("vpc", vec![]).unwrap();
        g.add("dns", vec![]).unwrap();
        g.add("subnet", vec!["vpc"]).unwrap();
        g.add("vm", vec!["subnet"]).unwrap();

        let batches = g.parallel_batches().unwrap();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 2); // vpc and dns can run in parallel
        assert!(batches[0].contains(&"vpc".to_string()));
        assert!(batches[0].contains(&"dns".to_string()));
        assert_eq!(batches[1], vec!["subnet"]);
        assert_eq!(batches[2], vec!["vm"]);
    }

    #[test]
    fn test_cycle_detection() {
        // Build a 3-node cycle manually by bypassing add() validation.
        // We use the internal maps directly to simulate a cycle:
        //   a → b → c → a
        let mut g = DependencyGraph::new();
        // Add three nodes without deps first so rdeps entries exist.
        g.deps.insert("a".to_string(), HashSet::from(["b".to_string()]));
        g.deps.insert("b".to_string(), HashSet::from(["c".to_string()]));
        g.deps.insert("c".to_string(), HashSet::from(["a".to_string()]));
        g.rdeps.insert("a".to_string(), HashSet::from(["c".to_string()]));
        g.rdeps.insert("b".to_string(), HashSet::from(["a".to_string()]));
        g.rdeps.insert("c".to_string(), HashSet::from(["b".to_string()]));

        let result = g.topo_sort();
        assert!(matches!(result, Err(GraphError::CycleDetected(_))));
    }
}
