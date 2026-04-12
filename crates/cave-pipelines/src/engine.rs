//! DAG-based pipeline execution engine.
//!
//! Handles task ordering, parallel layer detection, conditional execution,
//! and parameter interpolation.

use crate::models::{ParameterValue, TaskSpec, WhenExpression, WhenOperator};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Cycle detected in pipeline DAG involving task: {0}")]
    CycleDetected(String),
    #[error("Unknown task dependency: {0} depends on {1}")]
    UnknownDependency(String, String),
}

pub type EngineResult<T> = Result<T, EngineError>;

// ---------------------------------------------------------------------------
// DAG construction
// ---------------------------------------------------------------------------

/// Build successor list and in-degree map from task specs.
pub fn build_dag(
    tasks: &[TaskSpec],
) -> EngineResult<(HashMap<String, Vec<String>>, HashMap<String, usize>)> {
    let task_names: HashSet<String> = tasks.iter().map(|t| t.name.clone()).collect();

    let mut successors: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    for task in tasks {
        in_degree.entry(task.name.clone()).or_insert(0);
        successors.entry(task.name.clone()).or_default();

        for dep in &task.run_after {
            if !task_names.contains(dep) {
                return Err(EngineError::UnknownDependency(
                    task.name.clone(),
                    dep.clone(),
                ));
            }
            successors.entry(dep.clone()).or_default().push(task.name.clone());
            *in_degree.entry(task.name.clone()).or_insert(0) += 1;
        }
    }

    Ok((successors, in_degree))
}

/// Topological sort via Kahn's algorithm.
///
/// Returns layers where each layer is a set of tasks that can run in parallel
/// (all their dependencies have been satisfied by prior layers).
pub fn topological_layers(tasks: &[TaskSpec]) -> EngineResult<Vec<Vec<String>>> {
    let (successors, mut in_degree) = build_dag(tasks)?;

    let mut layers: Vec<Vec<String>> = Vec::new();
    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|&(_, deg)| *deg == 0)
        .map(|(name, _)| name.clone())
        .collect();

    // Sort the initial queue for deterministic ordering in tests.
    let mut q_vec: Vec<String> = queue.drain(..).collect();
    q_vec.sort();
    queue.extend(q_vec);

    let mut visited = 0;

    while !queue.is_empty() {
        let mut layer: Vec<String> = queue.drain(..).collect();
        layer.sort();
        visited += layer.len();

        for task in &layer {
            if let Some(deps) = successors.get(task) {
                let mut next: Vec<String> = Vec::new();
                for dep in deps {
                    let deg = in_degree.get_mut(dep).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        next.push(dep.clone());
                    }
                }
                next.sort();
                queue.extend(next);
            }
        }

        layers.push(layer);
    }

    if visited != tasks.len() {
        let cycle_task = in_degree
            .iter()
            .filter(|&(_, deg)| *deg > 0)
            .map(|(name, _)| name.clone())
            .next()
            .unwrap_or_else(|| "unknown".to_string());
        return Err(EngineError::CycleDetected(cycle_task));
    }

    Ok(layers)
}

// ---------------------------------------------------------------------------
// Conditional execution
// ---------------------------------------------------------------------------

/// Evaluate a single when expression against the current parameter set.
pub fn evaluate_when(when: &WhenExpression, params: &[ParameterValue]) -> bool {
    let input_value = if when.input.starts_with("$(params.") {
        let param_name = when.input
            .trim_start_matches("$(params.")
            .trim_end_matches(')');
        params
            .iter()
            .find(|p| p.name == param_name)
            .map(|p| p.value.as_str())
            .unwrap_or("")
    } else {
        when.input.as_str()
    };

    match when.operator {
        WhenOperator::In => when.values.iter().any(|v| v == input_value),
        WhenOperator::NotIn => !when.values.iter().any(|v| v == input_value),
    }
}

/// Returns true if all when expressions pass (empty → always run).
pub fn should_run_task(task: &TaskSpec, params: &[ParameterValue]) -> bool {
    task.when.is_empty() || task.when.iter().all(|w| evaluate_when(w, params))
}

// ---------------------------------------------------------------------------
// Parameter interpolation
// ---------------------------------------------------------------------------

/// Replace `$(params.name)` placeholders with their resolved values.
pub fn interpolate_params(template: &str, params: &[ParameterValue]) -> String {
    let mut result = template.to_string();
    for param in params {
        let placeholder = format!("$(params.{})", param.name);
        result = result.replace(&placeholder, &param.value);
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str, run_after: &[&str]) -> TaskSpec {
        let mut t = TaskSpec::new(name);
        t.run_after = run_after.iter().map(|s| s.to_string()).collect();
        t
    }

    fn param(name: &str, value: &str) -> ParameterValue {
        ParameterValue { name: name.to_string(), value: value.to_string() }
    }

    // --- DAG ordering ---

    #[test]
    fn test_dag_linear_ordering() {
        let tasks = vec![
            task("clone", &[]),
            task("build", &["clone"]),
            task("test", &["build"]),
            task("deploy", &["test"]),
        ];
        let layers = topological_layers(&tasks).unwrap();
        assert_eq!(layers.len(), 4);
        assert_eq!(layers[0], vec!["clone"]);
        assert_eq!(layers[1], vec!["build"]);
        assert_eq!(layers[2], vec!["test"]);
        assert_eq!(layers[3], vec!["deploy"]);
    }

    #[test]
    fn test_dag_parallel_tasks() {
        let tasks = vec![
            task("clone", &[]),
            task("lint", &["clone"]),
            task("test", &["clone"]),
            task("deploy", &["lint", "test"]),
        ];
        let layers = topological_layers(&tasks).unwrap();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec!["clone"]);
        assert_eq!(layers[1], vec!["lint", "test"]); // sorted
        assert_eq!(layers[2], vec!["deploy"]);
    }

    #[test]
    fn test_dag_diamond_shape() {
        let tasks = vec![
            task("a", &[]),
            task("b", &["a"]),
            task("c", &["a"]),
            task("d", &["b", "c"]),
        ];
        let layers = topological_layers(&tasks).unwrap();
        assert_eq!(layers[0], vec!["a"]);
        assert_eq!(layers[1], vec!["b", "c"]);
        assert_eq!(layers[2], vec!["d"]);
    }

    #[test]
    fn test_dag_cycle_detection() {
        let tasks = vec![
            task("a", &["c"]),
            task("b", &["a"]),
            task("c", &["b"]),
        ];
        let result = topological_layers(&tasks);
        assert!(matches!(result, Err(EngineError::CycleDetected(_))));
    }

    #[test]
    fn test_dag_unknown_dependency() {
        let tasks = vec![task("build", &["missing"])];
        let result = topological_layers(&tasks);
        assert!(matches!(result, Err(EngineError::UnknownDependency(_, _))));
    }

    #[test]
    fn test_dag_no_tasks() {
        let layers = topological_layers(&[]).unwrap();
        assert!(layers.is_empty());
    }

    #[test]
    fn test_dag_single_task() {
        let tasks = vec![task("lone", &[])];
        let layers = topological_layers(&tasks).unwrap();
        assert_eq!(layers, vec![vec!["lone"]]);
    }

    // --- Conditional execution ---

    #[test]
    fn test_when_in_operator_match() {
        let when = WhenExpression {
            input: "$(params.env)".to_string(),
            operator: WhenOperator::In,
            values: vec!["production".to_string(), "staging".to_string()],
        };
        assert!(evaluate_when(&when, &[param("env", "production")]));
        assert!(!evaluate_when(&when, &[param("env", "dev")]));
    }

    #[test]
    fn test_when_not_in_operator() {
        let when = WhenExpression {
            input: "$(params.skip)".to_string(),
            operator: WhenOperator::NotIn,
            values: vec!["true".to_string()],
        };
        assert!(evaluate_when(&when, &[param("skip", "false")]));
        assert!(!evaluate_when(&when, &[param("skip", "true")]));
    }

    #[test]
    fn test_should_run_task_no_when() {
        let t = TaskSpec::new("always");
        assert!(should_run_task(&t, &[]));
    }

    #[test]
    fn test_should_run_task_all_pass() {
        let mut t = TaskSpec::new("conditional");
        t.when = vec![WhenExpression {
            input: "$(params.deploy)".to_string(),
            operator: WhenOperator::In,
            values: vec!["true".to_string()],
        }];
        assert!(should_run_task(&t, &[param("deploy", "true")]));
        assert!(!should_run_task(&t, &[param("deploy", "false")]));
    }

    // --- Parameter interpolation ---

    #[test]
    fn test_interpolate_single_param() {
        let result = interpolate_params("hello $(params.name)", &[param("name", "world")]);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_interpolate_multiple_params() {
        let result = interpolate_params(
            "docker build -t $(params.image):$(params.tag) .",
            &[param("image", "myapp"), param("tag", "v1.0")],
        );
        assert_eq!(result, "docker build -t myapp:v1.0 .");
    }

    #[test]
    fn test_interpolate_missing_param_unchanged() {
        let result = interpolate_params("$(params.missing)", &[]);
        assert_eq!(result, "$(params.missing)");
    }

    #[test]
    fn test_interpolate_literal_no_params() {
        let result = interpolate_params("no placeholders here", &[]);
        assert_eq!(result, "no placeholders here");
    }
}
