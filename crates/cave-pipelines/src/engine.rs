// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DAG execution engine for pipelines.
//!
//! Resolves task dependencies, runs steps in parallel where possible,
//! handles fan-in/fan-out (Matrix), finally tasks, retries, and
//! when expressions.

use crate::models::*;
use std::collections::{HashMap, HashSet, VecDeque};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Handle for an in-flight pipeline run.
pub struct RunHandle {
    pub run_id: Uuid,
    pub cancel_tx: mpsc::Sender<()>,
    pub log_tx: mpsc::Sender<LogEntry>,
}

/// Execution graph node.
#[derive(Debug, Clone)]
pub struct DagNode {
    pub task: PipelineTask,
    pub deps: Vec<String>,
}

/// Build a DAG from a pipeline spec.
pub struct Dag {
    pub nodes: HashMap<String, DagNode>,
}

impl Dag {
    /// Construct DAG from pipeline tasks, including explicit runAfter deps.
    pub fn from_spec(tasks: &[PipelineTask]) -> Self {
        let mut nodes = HashMap::new();
        for t in tasks {
            let deps = t.run_after.clone();
            nodes.insert(
                t.name.clone(),
                DagNode { task: t.clone(), deps },
            );
        }
        Self { nodes }
    }

    /// Topological sort (Kahn's algorithm). Returns ordered execution waves.
    /// Each wave is a set of tasks that can run in parallel.
    pub fn execution_waves(&self) -> Result<Vec<Vec<String>>, DagError> {
        // Validate all dependencies exist
        for node in self.nodes.values() {
            for dep in &node.deps {
                if !self.nodes.contains_key(dep.as_str()) {
                    return Err(DagError::UnknownDependency {
                        task: node.task.name.clone(),
                        dep: dep.clone(),
                    });
                }
            }
        }

        let mut waves = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut remaining: HashMap<String, usize> = self
            .nodes
            .iter()
            .map(|(k, v)| (k.clone(), v.deps.len()))
            .collect();

        loop {
            let mut wave_sorted: Vec<String> = remaining
                .iter()
                .filter(|(_, d)| **d == 0)
                .filter(|(k, _)| !visited.contains(*k))
                .map(|(k, _)| k.clone())
                .collect();

            if wave_sorted.is_empty() {
                break;
            }

            wave_sorted.sort();
            for name in &wave_sorted {
                visited.insert(name.clone());
                remaining.remove(name.as_str());
                // Decrement dependents
                for (other_name, other_node) in &self.nodes {
                    if other_node.deps.contains(name) {
                        if let Some(d) = remaining.get_mut(other_name.as_str()) {
                            if *d > 0 {
                                *d -= 1;
                            }
                        }
                    }
                }
            }
            waves.push(wave_sorted);
        }

        if visited.len() != self.nodes.len() {
            return Err(DagError::CycleDetected);
        }

        Ok(waves)
    }

    /// Get all tasks that must complete before `task_name` can start.
    pub fn ancestors(&self, task_name: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(task_name.to_string());
        while let Some(current) = queue.pop_front() {
            if let Some(node) = self.nodes.get(&current) {
                for dep in &node.deps {
                    if result.insert(dep.clone()) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
        result
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DagError {
    #[error("Task '{task}' has unknown dependency '{dep}'")]
    UnknownDependency { task: String, dep: String },
    #[error("Pipeline has a cycle in task dependencies")]
    CycleDetected,
}

// ─── Parameter resolution ────────────────────────────────────────────────────

/// Resolve `$(params.NAME)` and `$(tasks.TASK.results.RESULT)` references.
pub fn resolve_param_string(
    template: &str,
    params: &HashMap<String, ParamValue>,
    task_results: &HashMap<String, HashMap<String, ParamValue>>,
) -> String {
    let mut result = template.to_string();

    // $(params.NAME)
    for (name, val) in params {
        let placeholder = format!("$(params.{})", name);
        if let Some(s) = val.as_str() {
            result = result.replace(&placeholder, s);
        }
    }

    // $(tasks.TASK.results.RESULT)
    for (task_name, results) in task_results {
        for (result_name, val) in results {
            let placeholder = format!("$(tasks.{}.results.{})", task_name, result_name);
            if let Some(s) = val.as_str() {
                result = result.replace(&placeholder, s);
            }
        }
    }

    result
}

/// Validate parameter values against their specs.
pub fn validate_params(
    specs: &[ParamSpec],
    provided: &[Param],
) -> Vec<String> {
    let mut errors = Vec::new();
    let provided_map: HashMap<&str, &Param> =
        provided.iter().map(|p| (p.name.as_str(), p)).collect();

    for spec in specs {
        match provided_map.get(spec.name.as_str()) {
            None => {
                if spec.default.is_none() {
                    errors.push(format!("Required parameter '{}' not provided", spec.name));
                }
            }
            Some(param) => {
                // Type check
                let type_ok = match (&spec.param_type, &param.value) {
                    (ParamType::String, ParamValue::String(_)) => true,
                    (ParamType::Array, ParamValue::Array(_)) => true,
                    (ParamType::Object, ParamValue::Object(_)) => true,
                    _ => false,
                };
                if !type_ok {
                    errors.push(format!(
                        "Parameter '{}' has wrong type (expected {:?})",
                        spec.name, spec.param_type
                    ));
                }

                // Enum check
                if let Some(enum_vals) = &spec.enum_values {
                    if let ParamValue::String(s) = &param.value {
                        if !enum_vals.contains(s) {
                            errors.push(format!(
                                "Parameter '{}' value '{}' not in allowed values {:?}",
                                spec.name, s, enum_vals
                            ));
                        }
                    }
                }
            }
        }
    }
    errors
}

// ─── Step execution result ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StepExecResult {
    pub step_name: String,
    pub exit_code: i32,
    pub results: Vec<(String, String)>,
    pub stdout: String,
    pub stderr: String,
}

// ─── Finally task handling ────────────────────────────────────────────────────

/// Returns true if all finally tasks should run (they always run regardless
/// of pipeline success/failure).
pub fn should_run_finally(_phase: &RunPhase) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{PipelineTask, ParamSpec, ParamType, ParamValue, Param};

    fn make_task(name: &str, deps: &[&str]) -> PipelineTask {
        PipelineTask {
            name: name.to_string(),
            task_ref: Some(PipelineTaskRef {
                name: name.to_string(),
                catalog: false,
                version: None,
            }),
            task_spec: None,
            run_after: deps.iter().map(|s| s.to_string()).collect(),
            params: vec![],
            workspaces: vec![],
            when: vec![],
            matrix: None,
            retry_policy: None,
            timeout: None,
            custom_task_ref: None,
        }
    }

    #[test]
    fn dag_linear_chain() {
        let tasks = vec![
            make_task("clone", &[]),
            make_task("build", &["clone"]),
            make_task("test", &["build"]),
            make_task("deploy", &["test"]),
        ];
        let dag = Dag::from_spec(&tasks);
        let waves = dag.execution_waves().unwrap();
        assert_eq!(waves.len(), 4);
        assert_eq!(waves[0], vec!["clone"]);
        assert_eq!(waves[1], vec!["build"]);
    }

    #[test]
    fn dag_parallel_fan_out() {
        let tasks = vec![
            make_task("clone", &[]),
            make_task("test-unit", &["clone"]),
            make_task("test-lint", &["clone"]),
            make_task("test-security", &["clone"]),
            make_task("deploy", &["test-unit", "test-lint", "test-security"]),
        ];
        let dag = Dag::from_spec(&tasks);
        let waves = dag.execution_waves().unwrap();
        assert_eq!(waves[0], vec!["clone"]);
        // Second wave: all three test tasks run in parallel
        assert_eq!(waves[1].len(), 3);
        assert_eq!(waves[2], vec!["deploy"]);
    }

    #[test]
    fn dag_cycle_detected() {
        let tasks = vec![
            make_task("a", &["b"]),
            make_task("b", &["a"]),
        ];
        let dag = Dag::from_spec(&tasks);
        assert!(matches!(dag.execution_waves(), Err(DagError::CycleDetected)));
    }

    #[test]
    fn dag_unknown_dep() {
        let tasks = vec![make_task("build", &["nonexistent"])];
        let dag = Dag::from_spec(&tasks);
        let err = dag.execution_waves().unwrap_err();
        assert!(matches!(err, DagError::UnknownDependency { .. }));
    }

    #[test]
    fn dag_ancestors() {
        let tasks = vec![
            make_task("clone", &[]),
            make_task("build", &["clone"]),
            make_task("test", &["build"]),
        ];
        let dag = Dag::from_spec(&tasks);
        let ancestors = dag.ancestors("test");
        assert!(ancestors.contains("build"));
        assert!(ancestors.contains("clone"));
        assert!(!ancestors.contains("test"));
    }

    #[test]
    fn resolve_param_string_basic() {
        let mut params = HashMap::new();
        params.insert("image".to_string(), ParamValue::String("alpine:3.18".to_string()));
        let task_results = HashMap::new();
        let out = resolve_param_string("docker pull $(params.image)", &params, &task_results);
        assert_eq!(out, "docker pull alpine:3.18");
    }

    #[test]
    fn resolve_task_result_reference() {
        let params = HashMap::new();
        let mut task_results = HashMap::new();
        let mut build_results = HashMap::new();
        build_results.insert("digest".to_string(), ParamValue::String("sha256:abc123".to_string()));
        task_results.insert("build".to_string(), build_results);
        let out = resolve_param_string(
            "digest=$(tasks.build.results.digest)",
            &params,
            &task_results,
        );
        assert_eq!(out, "digest=sha256:abc123");
    }

    #[test]
    fn validate_params_missing_required() {
        let specs = vec![ParamSpec {
            name: "env".to_string(),
            param_type: ParamType::String,
            description: None,
            default: None,
            enum_values: None,
        }];
        let errors = validate_params(&specs, &[]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("env"));
    }

    #[test]
    fn validate_params_enum_violation() {
        let specs = vec![ParamSpec {
            name: "env".to_string(),
            param_type: ParamType::String,
            description: None,
            default: None,
            enum_values: Some(vec!["staging".to_string(), "prod".to_string()]),
        }];
        let provided = vec![Param {
            name: "env".to_string(),
            value: ParamValue::String("dev".to_string()),
        }];
        let errors = validate_params(&specs, &provided);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("dev"));
    }

    #[test]
    fn validate_params_wrong_type() {
        let specs = vec![ParamSpec {
            name: "tags".to_string(),
            param_type: ParamType::Array,
            description: None,
            default: None,
            enum_values: None,
        }];
        let provided = vec![Param {
            name: "tags".to_string(),
            value: ParamValue::String("not-array".to_string()),
        }];
        let errors = validate_params(&specs, &provided);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn validate_params_with_default_ok() {
        let specs = vec![ParamSpec {
            name: "env".to_string(),
            param_type: ParamType::String,
            description: None,
            default: Some(ParamValue::String("staging".to_string())),
            enum_values: None,
        }];
        let errors = validate_params(&specs, &[]);
        assert!(errors.is_empty());
    }

    #[test]
    fn should_run_finally_always_true() {
        assert!(should_run_finally(&RunPhase::Succeeded));
        assert!(should_run_finally(&RunPhase::Failed));
        assert!(should_run_finally(&RunPhase::Cancelled));
    }
}
