// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Workflow executor — pure-function reducer for Argo Workflows
//! (`argoproj/argo-workflows v4.0.5`). Walks the DAG / Steps graph,
//! threads parameters + artifacts through node statuses, and emits the
//! state transitions a downstream `cave-cri` runtime would apply.

use crate::workflow_crd::{
    topo_order, Arguments, Artifact, DagTemplate, NodeStatus, Outputs, Parameter, RetryStrategy,
    StepsTemplate, Template, TemplateBody, Workflow, WorkflowPhase, WorkflowStep, WorkflowStatus,
};
use chrono::Utc;
use std::collections::HashMap;

/// One scheduling tick the executor wants the runtime to perform.
#[derive(Clone, Debug)]
pub enum NodeAction {
    Schedule {
        node_id: String,
        template_name: String,
        arguments: ArgumentBindings,
    },
    Retry {
        node_id: String,
        attempt: u32,
    },
    Suspend {
        node_id: String,
        duration_seconds: Option<u64>,
    },
    Complete {
        node_id: String,
        phase: WorkflowPhase,
    },
}

/// Resolved parameter + artifact bindings for one node — passed into the
/// underlying container at run time.
#[derive(Clone, Debug, Default)]
pub struct ArgumentBindings {
    pub parameters: Vec<(String, String)>,
    pub artifacts: Vec<Artifact>,
}

impl From<&Arguments> for ArgumentBindings {
    fn from(a: &Arguments) -> Self {
        Self {
            parameters: a
                .parameters
                .iter()
                .filter_map(|p| {
                    let v = p.value.clone().or_else(|| p.default.clone())?;
                    Some((p.name.clone(), v))
                })
                .collect(),
            artifacts: a.artifacts.clone(),
        }
    }
}

/// Render the next batch of NodeAction calls the runtime should execute.
/// Each call advances the workflow by one scheduling tick.
pub fn next_actions(wf: &Workflow) -> Vec<NodeAction> {
    let mut out = Vec::new();
    let Some(entry) = wf.find_template(&wf.spec.entrypoint) else {
        return out;
    };
    match &entry.body {
        TemplateBody::Dag(dag) => schedule_dag(wf, &entry.name, dag, &mut out),
        TemplateBody::Steps(steps) => schedule_steps(wf, &entry.name, steps, &mut out),
        TemplateBody::Container(_) | TemplateBody::Script(_) | TemplateBody::Resource(_) => {
            let id = node_id(&entry.name, "0");
            if !wf.status.nodes.contains_key(&id) {
                out.push(NodeAction::Schedule {
                    node_id: id,
                    template_name: entry.name.clone(),
                    arguments: (&wf.spec.arguments).into(),
                });
            }
        }
        TemplateBody::Suspend(s) => {
            let id = node_id(&entry.name, "0");
            if !wf.status.nodes.contains_key(&id) {
                out.push(NodeAction::Suspend {
                    node_id: id,
                    duration_seconds: s
                        .duration
                        .as_deref()
                        .and_then(parse_duration_seconds),
                });
            }
        }
    }
    out
}

fn schedule_dag(wf: &Workflow, dag_name: &str, dag: &DagTemplate, out: &mut Vec<NodeAction>) {
    let Ok(order) = topo_order(&dag.tasks) else {
        return;
    };
    for task_name in &order {
        let Some(task) = dag.tasks.iter().find(|t| &t.name == task_name) else {
            continue;
        };
        let nid = node_id(dag_name, task_name);
        if wf.status.nodes.contains_key(&nid) {
            continue;
        }
        let deps_ready = task.dependencies.iter().all(|d| {
            let did = node_id(dag_name, d);
            matches!(
                wf.status.nodes.get(&did).map(|n| n.phase),
                Some(WorkflowPhase::Succeeded)
            )
        });
        if deps_ready {
            out.push(NodeAction::Schedule {
                node_id: nid,
                template_name: task.template.clone(),
                arguments: (&task.arguments).into(),
            });
        }
    }
}

fn schedule_steps(
    wf: &Workflow,
    steps_name: &str,
    steps: &StepsTemplate,
    out: &mut Vec<NodeAction>,
) {
    for (group_idx, group) in steps.steps.iter().enumerate() {
        let prior_done = group_idx == 0 || group_complete(wf, steps_name, group_idx - 1, &steps.steps[group_idx - 1]);
        if !prior_done {
            return;
        }
        let mut any_pending = false;
        for s in group {
            let nid = node_id(steps_name, &format!("g{group_idx}-{}", s.name));
            if wf.status.nodes.contains_key(&nid) {
                continue;
            }
            any_pending = true;
            out.push(NodeAction::Schedule {
                node_id: nid,
                template_name: s.template.clone(),
                arguments: (&s.arguments).into(),
            });
        }
        if any_pending {
            return;
        }
    }
}

fn group_complete(wf: &Workflow, steps_name: &str, group_idx: usize, group: &[WorkflowStep]) -> bool {
    group.iter().all(|s| {
        let nid = node_id(steps_name, &format!("g{group_idx}-{}", s.name));
        matches!(
            wf.status.nodes.get(&nid).map(|n| n.phase),
            Some(WorkflowPhase::Succeeded)
        )
    })
}

fn node_id(parent: &str, child: &str) -> String {
    format!("{parent}.{child}")
}

/// Compute the overall workflow phase from per-node statuses + spec.
pub fn aggregate_phase(wf: &Workflow) -> WorkflowPhase {
    if wf.status.nodes.is_empty() {
        return WorkflowPhase::Pending;
    }
    if wf
        .status
        .nodes
        .values()
        .any(|n| matches!(n.phase, WorkflowPhase::Suspended))
    {
        return WorkflowPhase::Suspended;
    }
    if wf
        .status
        .nodes
        .values()
        .any(|n| matches!(n.phase, WorkflowPhase::Error))
    {
        return WorkflowPhase::Error;
    }
    if wf
        .status
        .nodes
        .values()
        .any(|n| matches!(n.phase, WorkflowPhase::Failed))
    {
        return WorkflowPhase::Failed;
    }
    let all_done = wf
        .status
        .nodes
        .values()
        .all(|n| matches!(n.phase, WorkflowPhase::Succeeded));
    if all_done {
        WorkflowPhase::Succeeded
    } else {
        WorkflowPhase::Running
    }
}

/// Compute the next retry attempt for a node, respecting [RetryStrategy].
/// Returns `None` if the policy forbids further retries.
pub fn retry_decision(
    strategy: Option<&RetryStrategy>,
    current_attempts: u32,
    last_phase: WorkflowPhase,
) -> Option<u32> {
    let strat = strategy?;
    if current_attempts >= strat.limit {
        return None;
    }
    let policy = strat.retry_policy.as_str();
    let retry = matches!(
        (policy, last_phase),
        ("Always", _)
            | ("OnFailure", WorkflowPhase::Failed)
            | ("OnError", WorkflowPhase::Error)
            | ("OnTransientError", WorkflowPhase::Error)
    );
    if retry { Some(current_attempts + 1) } else { None }
}

/// Mark a node as `Succeeded` with its outputs and rebuild aggregate phase.
pub fn record_success(
    wf: &mut Workflow,
    node_id: &str,
    template_name: &str,
    outputs: Outputs,
) -> WorkflowPhase {
    wf.status.nodes.insert(
        node_id.to_string(),
        NodeStatus {
            id: node_id.to_string(),
            template_name: template_name.to_string(),
            phase: WorkflowPhase::Succeeded,
            message: None,
            started_at: wf
                .status
                .nodes
                .get(node_id)
                .and_then(|n| n.started_at)
                .or(Some(Utc::now())),
            finished_at: Some(Utc::now()),
            outputs: Some(outputs),
            children: Vec::new(),
        },
    );
    let phase = aggregate_phase(wf);
    wf.status.phase = phase;
    if matches!(phase, WorkflowPhase::Succeeded | WorkflowPhase::Failed | WorkflowPhase::Error) {
        wf.status.finished_at.get_or_insert(Utc::now());
    }
    phase
}

/// Parse `"30s"` / `"5m"` / `"2h"` to seconds. Minimal — what Argo accepts.
pub fn parse_duration_seconds(s: &str) -> Option<u64> {
    let s = s.trim();
    let (digits, suffix) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = digits.parse().ok()?;
    let mult = match suffix {
        "s" | "" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return None,
    };
    Some(n * mult)
}

/// Apply default parameter values when the caller did not pass one.
pub fn apply_parameter_defaults(declared: &[Parameter], supplied: &mut Vec<Parameter>) {
    let supplied_names: std::collections::HashSet<_> =
        supplied.iter().map(|p| p.name.clone()).collect();
    for d in declared {
        if !supplied_names.contains(&d.name) {
            if let Some(def) = &d.default {
                supplied.push(Parameter {
                    name: d.name.clone(),
                    value: Some(def.clone()),
                    default: Some(def.clone()),
                    value_from: None,
                    description: d.description.clone(),
                });
            }
        }
    }
}

/// Indexed lookup of templates — useful for downstream callers.
pub fn template_index(wf: &Workflow) -> HashMap<String, &Template> {
    wf.spec
        .templates
        .iter()
        .map(|t| (t.name.clone(), t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_crd::{
        ContainerTemplate, DagTask, Inputs, ScriptTemplate, SuspendTemplate, WorkflowSpec,
    };

    fn cont_tpl(name: &str) -> Template {
        Template {
            name: name.into(),
            inputs: Inputs::default(),
            outputs: Outputs::default(),
            body: TemplateBody::Container(ContainerTemplate {
                image: "alpine".into(),
                command: vec!["sh".into()],
                args: vec!["-c".into(), "echo".into()],
                env: HashMap::new(),
                working_dir: None,
            }),
            retry_strategy: None,
            timeout: None,
        }
    }

    fn dag_wf(tasks: Vec<DagTask>) -> Workflow {
        Workflow::new(
            "wf",
            "argo",
            WorkflowSpec {
                entrypoint: "d".into(),
                templates: vec![
                    cont_tpl("t"),
                    Template {
                        name: "d".into(),
                        inputs: Inputs::default(),
                        outputs: Outputs::default(),
                        body: TemplateBody::Dag(DagTemplate { tasks, fail_fast: None }),
                        retry_strategy: None,
                        timeout: None,
                    },
                ],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        )
    }

    #[test]
    fn next_actions_schedules_dag_roots_first() {
        let wf = dag_wf(vec![
            DagTask {
                name: "a".into(),
                template: "t".into(),
                dependencies: vec![],
                arguments: Arguments::default(),
                when: None,
            },
            DagTask {
                name: "b".into(),
                template: "t".into(),
                dependencies: vec!["a".into()],
                arguments: Arguments::default(),
                when: None,
            },
        ]);
        let actions = next_actions(&wf);
        // Only `a` is ready — `b` is gated by `a`.
        let scheduled: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                NodeAction::Schedule { node_id, .. } => Some(node_id.clone()),
                _ => None,
            })
            .collect();
        assert!(scheduled.iter().any(|s| s.ends_with(".a")));
        assert!(!scheduled.iter().any(|s| s.ends_with(".b")));
    }

    #[test]
    fn next_actions_unblocks_b_after_a_succeeds() {
        let mut wf = dag_wf(vec![
            DagTask {
                name: "a".into(),
                template: "t".into(),
                dependencies: vec![],
                arguments: Arguments::default(),
                when: None,
            },
            DagTask {
                name: "b".into(),
                template: "t".into(),
                dependencies: vec!["a".into()],
                arguments: Arguments::default(),
                when: None,
            },
        ]);
        record_success(&mut wf, "d.a", "t", Outputs::default());
        let actions = next_actions(&wf);
        let scheduled: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                NodeAction::Schedule { node_id, .. } => Some(node_id.clone()),
                _ => None,
            })
            .collect();
        assert!(scheduled.iter().any(|s| s == "d.b"));
    }

    #[test]
    fn aggregate_phase_pending_on_empty_nodes() {
        let wf = dag_wf(vec![]);
        assert_eq!(aggregate_phase(&wf), WorkflowPhase::Pending);
    }

    #[test]
    fn aggregate_phase_succeeds_when_all_succeed() {
        let mut wf = dag_wf(vec![DagTask {
            name: "a".into(),
            template: "t".into(),
            dependencies: vec![],
            arguments: Arguments::default(),
            when: None,
        }]);
        record_success(&mut wf, "d.a", "t", Outputs::default());
        assert_eq!(aggregate_phase(&wf), WorkflowPhase::Succeeded);
        assert!(wf.status.finished_at.is_some());
    }

    #[test]
    fn aggregate_phase_failed_propagates() {
        let mut wf = dag_wf(vec![DagTask {
            name: "a".into(),
            template: "t".into(),
            dependencies: vec![],
            arguments: Arguments::default(),
            when: None,
        }]);
        wf.status.nodes.insert(
            "d.a".into(),
            NodeStatus {
                id: "d.a".into(),
                template_name: "t".into(),
                phase: WorkflowPhase::Failed,
                message: Some("bad".into()),
                started_at: None,
                finished_at: None,
                outputs: None,
                children: vec![],
            },
        );
        assert_eq!(aggregate_phase(&wf), WorkflowPhase::Failed);
    }

    #[test]
    fn retry_decision_respects_limit() {
        let s = RetryStrategy {
            limit: 2,
            retry_policy: "OnFailure".into(),
            backoff: None,
        };
        assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Failed), Some(1));
        assert_eq!(retry_decision(Some(&s), 1, WorkflowPhase::Failed), Some(2));
        assert_eq!(retry_decision(Some(&s), 2, WorkflowPhase::Failed), None);
    }

    #[test]
    fn retry_decision_filters_by_policy() {
        let s = RetryStrategy {
            limit: 5,
            retry_policy: "OnError".into(),
            backoff: None,
        };
        assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Failed), None);
        assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Error), Some(1));
    }

    #[test]
    fn parse_duration_handles_units() {
        assert_eq!(parse_duration_seconds("30s"), Some(30));
        assert_eq!(parse_duration_seconds("5m"), Some(300));
        assert_eq!(parse_duration_seconds("2h"), Some(7200));
        assert_eq!(parse_duration_seconds("1d"), Some(86_400));
        assert_eq!(parse_duration_seconds("nonsense"), None);
    }

    #[test]
    fn argument_bindings_inherit_defaults() {
        let args = Arguments {
            parameters: vec![Parameter {
                name: "msg".into(),
                value: None,
                default: Some("hello".into()),
                value_from: None,
                description: None,
            }],
            artifacts: vec![],
        };
        let b: ArgumentBindings = (&args).into();
        assert_eq!(b.parameters.first().unwrap().1, "hello");
    }

    #[test]
    fn apply_parameter_defaults_fills_missing() {
        let declared = vec![
            Parameter {
                name: "a".into(),
                value: None,
                default: Some("x".into()),
                value_from: None,
                description: None,
            },
            Parameter {
                name: "b".into(),
                value: None,
                default: Some("y".into()),
                value_from: None,
                description: None,
            },
        ];
        let mut supplied = vec![Parameter {
            name: "a".into(),
            value: Some("supplied".into()),
            default: None,
            value_from: None,
            description: None,
        }];
        apply_parameter_defaults(&declared, &mut supplied);
        assert_eq!(supplied.len(), 2);
        let b = supplied.iter().find(|p| p.name == "b").unwrap();
        assert_eq!(b.value.as_deref(), Some("y"));
    }

    #[test]
    fn suspend_template_emits_suspend_action() {
        let wf = Workflow::new(
            "w",
            "argo",
            WorkflowSpec {
                entrypoint: "s".into(),
                templates: vec![Template {
                    name: "s".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Suspend(SuspendTemplate {
                        duration: Some("5m".into()),
                    }),
                    retry_strategy: None,
                    timeout: None,
                }],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        );
        let actions = next_actions(&wf);
        assert!(matches!(
            actions.first(),
            Some(NodeAction::Suspend { duration_seconds: Some(300), .. })
        ));
    }

    #[test]
    fn script_template_compiles_into_container_dispatch() {
        let wf = Workflow::new(
            "w",
            "argo",
            WorkflowSpec {
                entrypoint: "scr".into(),
                templates: vec![Template {
                    name: "scr".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Script(ScriptTemplate {
                        image: "alpine".into(),
                        source: "echo hi".into(),
                        command: vec!["sh".into()],
                    }),
                    retry_strategy: None,
                    timeout: None,
                }],
                arguments: Arguments::default(),
                service_account_name: None,
                on_exit: None,
                parallelism: None,
                workflow_template_ref: None,
            },
        );
        let actions = next_actions(&wf);
        assert!(matches!(actions.first(), Some(NodeAction::Schedule { .. })));
    }
}
