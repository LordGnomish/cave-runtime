// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave-workflows`, the pure-function
//! reducer port of the Argo Workflows controller scheduling core
//! (`argoproj/argo-workflows` @ `v4.0.5`).
//!
//! Each test exercises an already-implemented public `cave_workflows` fn that
//! the in-crate unit tests do not yet assert behaviorally: Steps group
//! sequencing/fan-out, aggregate-phase precedence (Suspended/Error), the full
//! `retry_decision` policy matrix (Always / OnTransientError / None / unknown),
//! indefinite Suspend, Resource dispatch, `record_success` read-back,
//! `template_index`, per-driver `ArtifactRepository` serde, and namespace
//! filtering in `WorkflowStore::list`.

use cave_workflows::executor::{
    aggregate_phase, next_actions, record_success, retry_decision, template_index, NodeAction,
};
use cave_workflows::store::WorkflowStore;
use cave_workflows::workflow_crd::{
    Arguments, Artifact, ArtifactRepository, ContainerTemplate, DagTask, DagTemplate, Inputs,
    NodeStatus, Outputs, ResourceTemplate, RetryStrategy, StepsTemplate, SuspendTemplate, Template,
    TemplateBody, Workflow, WorkflowPhase, WorkflowSpec, WorkflowStep,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// builders
// ---------------------------------------------------------------------------

fn cont_template(name: &str) -> Template {
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

fn spec(entrypoint: &str, templates: Vec<Template>) -> WorkflowSpec {
    WorkflowSpec {
        entrypoint: entrypoint.into(),
        templates,
        arguments: Arguments::default(),
        service_account_name: None,
        on_exit: None,
        parallelism: None,
        workflow_template_ref: None,
    }
}

fn step(name: &str, template: &str) -> WorkflowStep {
    WorkflowStep {
        name: name.into(),
        template: template.into(),
        arguments: Arguments::default(),
        when: None,
    }
}

/// Build a Steps workflow whose entrypoint `"s"` runs `groups` (each an inner
/// `Vec<WorkflowStep>`), all referencing the single container template `"t"`.
fn steps_wf(groups: Vec<Vec<WorkflowStep>>) -> Workflow {
    Workflow::new(
        "wf",
        "argo",
        spec(
            "s",
            vec![
                cont_template("t"),
                Template {
                    name: "s".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Steps(StepsTemplate { steps: groups }),
                    retry_strategy: None,
                    timeout: None,
                },
            ],
        ),
    )
}

fn scheduled_ids(actions: &[NodeAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            NodeAction::Schedule { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect()
}

/// Insert a NodeStatus with an arbitrary phase (terminal-state setup helper).
fn put_node(wf: &mut Workflow, id: &str, phase: WorkflowPhase) {
    wf.status.nodes.insert(
        id.to_string(),
        NodeStatus {
            id: id.to_string(),
            template_name: "t".into(),
            phase,
            message: None,
            started_at: None,
            finished_at: None,
            outputs: None,
            children: vec![],
        },
    );
}

// ---------------------------------------------------------------------------
// executor::next_actions — Steps sequencing (group N+1 waits for group N)
// ---------------------------------------------------------------------------

#[test]
fn steps_groups_run_sequentially() {
    // Two sequential groups: [a], then [b]. node_id = "s.g{idx}-{name}".
    let mut wf = steps_wf(vec![vec![step("a", "t")], vec![step("b", "t")]]);

    // Tick 1: only group 0's "a" is offered; "b" is gated behind group 0.
    let first = scheduled_ids(&next_actions(&wf));
    assert_eq!(first, vec!["s.g0-a".to_string()]);

    // Complete group 0, then re-tick: group 1's "b" unblocks.
    record_success(&mut wf, "s.g0-a", "t", Outputs::default());
    let second = scheduled_ids(&next_actions(&wf));
    assert_eq!(second, vec!["s.g1-b".to_string()]);
}

#[test]
fn steps_fan_out_schedules_whole_group_together() {
    // Single group with two members → both scheduled in one tick.
    let wf = steps_wf(vec![vec![step("a", "t"), step("b", "t")]]);
    let ids = scheduled_ids(&next_actions(&wf));
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"s.g0-a".to_string()));
    assert!(ids.contains(&"s.g0-b".to_string()));
}

// ---------------------------------------------------------------------------
// executor::aggregate_phase — Suspended + Error precedence
// ---------------------------------------------------------------------------

#[test]
fn aggregate_phase_suspended_when_any_node_suspended() {
    // Suspended outranks every other phase including Succeeded.
    let mut wf = steps_wf(vec![vec![step("a", "t")]]);
    put_node(&mut wf, "n1", WorkflowPhase::Succeeded);
    put_node(&mut wf, "n2", WorkflowPhase::Suspended);
    assert_eq!(aggregate_phase(&wf), WorkflowPhase::Suspended);
}

#[test]
fn aggregate_phase_error_beats_failed() {
    // Precedence (after Suspended): Error > Failed. A mixed Error+Failed set
    // resolves to Error.
    let mut wf = steps_wf(vec![vec![step("a", "t")]]);
    put_node(&mut wf, "n1", WorkflowPhase::Failed);
    put_node(&mut wf, "n2", WorkflowPhase::Error);
    assert_eq!(aggregate_phase(&wf), WorkflowPhase::Error);

    // And an Error alone still aggregates to Error.
    let mut only_err = steps_wf(vec![vec![step("a", "t")]]);
    put_node(&mut only_err, "n1", WorkflowPhase::Error);
    assert_eq!(aggregate_phase(&only_err), WorkflowPhase::Error);
}

// ---------------------------------------------------------------------------
// executor::retry_decision — policy matrix
// ---------------------------------------------------------------------------

#[test]
fn retry_decision_always_retries_regardless_of_phase() {
    let s = RetryStrategy {
        limit: 3,
        retry_policy: "Always".into(),
        backoff: None,
    };
    // "Always" retries on any last phase, incrementing the attempt counter.
    assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Succeeded), Some(1));
    assert_eq!(retry_decision(Some(&s), 1, WorkflowPhase::Failed), Some(2));
    assert_eq!(retry_decision(Some(&s), 2, WorkflowPhase::Error), Some(3));
    // Limit boundary: current_attempts >= limit forbids further retry.
    assert_eq!(retry_decision(Some(&s), 3, WorkflowPhase::Error), None);
}

#[test]
fn retry_decision_on_transient_error_retries_only_on_error() {
    let s = RetryStrategy {
        limit: 5,
        retry_policy: "OnTransientError".into(),
        backoff: None,
    };
    // Retries on Error, declines on Failed/Succeeded.
    assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Error), Some(1));
    assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Failed), None);
    assert_eq!(retry_decision(Some(&s), 0, WorkflowPhase::Succeeded), None);
}

#[test]
fn retry_decision_none_strategy_and_unknown_policy_never_retry() {
    // No strategy at all → None.
    assert_eq!(retry_decision(None, 0, WorkflowPhase::Failed), None);
    // Unknown policy string falls through the match → None even under limit.
    let bogus = RetryStrategy {
        limit: 10,
        retry_policy: "Whenever".into(),
        backoff: None,
    };
    assert_eq!(retry_decision(Some(&bogus), 0, WorkflowPhase::Error), None);
    assert_eq!(retry_decision(Some(&bogus), 0, WorkflowPhase::Failed), None);
}

// ---------------------------------------------------------------------------
// executor::next_actions — Suspend (indefinite) + Resource dispatch
// ---------------------------------------------------------------------------

#[test]
fn suspend_template_without_duration_is_indefinite() {
    let wf = Workflow::new(
        "w",
        "argo",
        spec(
            "s",
            vec![Template {
                name: "s".into(),
                inputs: Inputs::default(),
                outputs: Outputs::default(),
                body: TemplateBody::Suspend(SuspendTemplate { duration: None }),
                retry_strategy: None,
                timeout: None,
            }],
        ),
    );
    let actions = next_actions(&wf);
    assert!(matches!(
        actions.first(),
        Some(NodeAction::Suspend {
            duration_seconds: None,
            ..
        })
    ));
}

#[test]
fn resource_template_entrypoint_emits_schedule() {
    let wf = Workflow::new(
        "w",
        "argo",
        spec(
            "r",
            vec![Template {
                name: "r".into(),
                inputs: Inputs::default(),
                outputs: Outputs::default(),
                body: TemplateBody::Resource(ResourceTemplate {
                    action: "create".into(),
                    manifest: "kind: Pod".into(),
                    success_condition: None,
                    failure_condition: None,
                }),
                retry_strategy: None,
                timeout: None,
            }],
        ),
    );
    let actions = next_actions(&wf);
    // Resource dispatches like Container/Script: a single Schedule of "r.0".
    match actions.first() {
        Some(NodeAction::Schedule {
            node_id,
            template_name,
            ..
        }) => {
            assert_eq!(node_id, "r.0");
            assert_eq!(template_name, "r");
        }
        other => panic!("expected Schedule, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// executor::record_success — node + workflow read-back
// ---------------------------------------------------------------------------

#[test]
fn record_success_writes_node_outputs_and_finished_at() {
    // DAG with a single root task "a"; completing it is terminal → wf finishes.
    let mut wf = Workflow::new(
        "wf",
        "argo",
        spec(
            "d",
            vec![
                cont_template("t"),
                Template {
                    name: "d".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Dag(DagTemplate {
                        tasks: vec![cave_workflows::workflow_crd::DagTask {
                            name: "a".into(),
                            template: "t".into(),
                            dependencies: vec![],
                            arguments: Arguments::default(),
                            when: None,
                        }],
                        fail_fast: None,
                    }),
                    retry_strategy: None,
                    timeout: None,
                },
            ],
        ),
    );

    let mut outs = Outputs::default();
    outs.result = Some("done".into());
    let phase = record_success(&mut wf, "d.a", "t", outs);
    assert_eq!(phase, WorkflowPhase::Succeeded);

    let node = wf.status.nodes.get("d.a").expect("node recorded");
    assert_eq!(node.phase, WorkflowPhase::Succeeded);
    assert!(node.finished_at.is_some());
    assert!(node.outputs.is_some());
    assert_eq!(node.outputs.as_ref().unwrap().result.as_deref(), Some("done"));

    // Terminal aggregate → workflow finished_at stamped, phase mirrored.
    assert_eq!(wf.status.phase, WorkflowPhase::Succeeded);
    assert!(wf.status.finished_at.is_some());
}

#[test]
fn record_success_nonterminal_leaves_workflow_unfinished() {
    // Two independent roots; finishing only one leaves the wf Running and
    // therefore unstamped (finished_at set only on Succeeded/Failed/Error).
    let mk = |name: &str| cave_workflows::workflow_crd::DagTask {
        name: name.into(),
        template: "t".into(),
        dependencies: vec![],
        arguments: Arguments::default(),
        when: None,
    };
    let mut wf = Workflow::new(
        "wf",
        "argo",
        spec(
            "d",
            vec![
                cont_template("t"),
                Template {
                    name: "d".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Dag(DagTemplate {
                        tasks: vec![mk("a"), mk("b")],
                        fail_fast: None,
                    }),
                    retry_strategy: None,
                    timeout: None,
                },
            ],
        ),
    );
    // "b" is still pending/Running before "a" completes.
    put_node(&mut wf, "d.b", WorkflowPhase::Running);
    let phase = record_success(&mut wf, "d.a", "t", Outputs::default());
    assert_eq!(phase, WorkflowPhase::Running);
    assert!(wf.status.finished_at.is_none());
}

// ---------------------------------------------------------------------------
// executor::template_index — name → Template map
// ---------------------------------------------------------------------------

#[test]
fn template_index_maps_every_template_by_name() {
    let wf = Workflow::new(
        "wf",
        "argo",
        spec("first", vec![cont_template("first"), cont_template("second")]),
    );
    let idx = template_index(&wf);
    assert_eq!(idx.len(), 2);
    assert_eq!(idx.get("first").unwrap().name, "first");
    assert_eq!(idx.get("second").unwrap().name, "second");
    assert!(idx.get("missing").is_none());
}

// ---------------------------------------------------------------------------
// workflow_crd::Artifact serde — per-driver repository roundtrip
// ---------------------------------------------------------------------------

fn artifact_with(repo: ArtifactRepository) -> Artifact {
    Artifact {
        name: "a".into(),
        path: None,
        from: None,
        repository: Some(repo),
        archive: None,
    }
}

#[test]
fn artifact_repository_git_oss_hdfs_raw_roundtrip() {
    // Git: { kind: "git", repo, revision, depth }
    let git = artifact_with(ArtifactRepository::Git {
        repo: "https://example/r.git".into(),
        revision: "main".into(),
        depth: Some(1),
    });
    let j = serde_json::to_string(&git).unwrap();
    assert!(j.contains("\"kind\":\"git\""));
    match serde_json::from_str::<Artifact>(&j).unwrap().repository {
        Some(ArtifactRepository::Git {
            repo,
            revision,
            depth,
        }) => {
            assert_eq!(repo, "https://example/r.git");
            assert_eq!(revision, "main");
            assert_eq!(depth, Some(1));
        }
        other => panic!("expected Git, got {other:?}"),
    }

    // Oss: { kind: "oss", bucket, key, endpoint }
    let oss = artifact_with(ArtifactRepository::Oss {
        bucket: "b".into(),
        key: "k".into(),
        endpoint: "oss-cn".into(),
    });
    let j = serde_json::to_string(&oss).unwrap();
    assert!(j.contains("\"kind\":\"oss\""));
    match serde_json::from_str::<Artifact>(&j).unwrap().repository {
        Some(ArtifactRepository::Oss {
            bucket,
            key,
            endpoint,
        }) => {
            assert_eq!(bucket, "b");
            assert_eq!(key, "k");
            assert_eq!(endpoint, "oss-cn");
        }
        other => panic!("expected Oss, got {other:?}"),
    }

    // Hdfs: { kind: "hdfs", addresses, path }
    let hdfs = artifact_with(ArtifactRepository::Hdfs {
        addresses: vec!["nn1:8020".into(), "nn2:8020".into()],
        path: "/data".into(),
    });
    let j = serde_json::to_string(&hdfs).unwrap();
    assert!(j.contains("\"kind\":\"hdfs\""));
    match serde_json::from_str::<Artifact>(&j).unwrap().repository {
        Some(ArtifactRepository::Hdfs { addresses, path }) => {
            assert_eq!(addresses, vec!["nn1:8020".to_string(), "nn2:8020".to_string()]);
            assert_eq!(path, "/data");
        }
        other => panic!("expected Hdfs, got {other:?}"),
    }

    // Raw: { kind: "raw", data }
    let raw = artifact_with(ArtifactRepository::Raw {
        data: "literal".into(),
    });
    let j = serde_json::to_string(&raw).unwrap();
    assert!(j.contains("\"kind\":\"raw\""));
    match serde_json::from_str::<Artifact>(&j).unwrap().repository {
        Some(ArtifactRepository::Raw { data }) => assert_eq!(data, "literal"),
        other => panic!("expected Raw, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// store::WorkflowStore::list — namespace filter excludes other namespaces
// ---------------------------------------------------------------------------

#[test]
fn list_filters_by_namespace() {
    fn nsd_wf(name: &str, ns: &str) -> Workflow {
        Workflow::new(name, ns, spec("main", vec![cont_template("main")]))
    }
    let store = WorkflowStore::new();
    store.create(nsd_wf("w1", "argo")).unwrap();
    store.create(nsd_wf("w2", "argo")).unwrap();
    store.create(nsd_wf("w3", "prod")).unwrap();

    let argo = store.list(Some("argo"));
    assert_eq!(argo.len(), 2);
    assert!(argo.iter().all(|w| w.namespace == "argo"));
    assert!(!argo.iter().any(|w| w.name == "w3"));

    assert_eq!(store.list(Some("prod")).len(), 1);
    // None → all namespaces.
    assert_eq!(store.list(None).len(), 3);
}

// ---------------------------------------------------------------------------
// executor::next_actions — `when` conditional gating (Skip / Skipped)
// argoproj/argo-workflows workflow/controller/operator.go shouldExecute
// ---------------------------------------------------------------------------

fn dag_task(name: &str, deps: &[&str], when: Option<&str>) -> DagTask {
    DagTask {
        name: name.into(),
        template: "t".into(),
        dependencies: deps.iter().map(|d| d.to_string()).collect(),
        arguments: Arguments::default(),
        when: when.map(|w| w.to_string()),
    }
}

fn dag_wf_when(tasks: Vec<DagTask>) -> Workflow {
    Workflow::new(
        "wf",
        "argo",
        spec(
            "d",
            vec![
                cont_template("t"),
                Template {
                    name: "d".into(),
                    inputs: Inputs::default(),
                    outputs: Outputs::default(),
                    body: TemplateBody::Dag(DagTemplate { tasks, fail_fast: None }),
                    retry_strategy: None,
                    timeout: None,
                },
            ],
        ),
    )
}

fn skipped_ids(actions: &[NodeAction]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            NodeAction::Skip { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect()
}

/// Record `a` succeeded with a result, then b's `when` references it.
fn complete_a_with_result(wf: &mut Workflow, result: &str) {
    record_success(
        wf,
        "d.a",
        "t",
        Outputs {
            result: Some(result.into()),
            ..Outputs::default()
        },
    );
}

#[test]
fn dag_skips_task_when_condition_false() {
    let mut wf = dag_wf_when(vec![
        dag_task("a", &[], None),
        dag_task("b", &["a"], Some("{{tasks.a.outputs.result}} == go")),
    ]);
    complete_a_with_result(&mut wf, "stop");
    let actions = next_actions(&wf);
    // `b`'s when is false → it is offered as a Skip, never a Schedule.
    assert!(scheduled_ids(&actions).iter().all(|s| s != "d.b"));
    assert_eq!(skipped_ids(&actions), vec!["d.b".to_string()]);
}

#[test]
fn dag_schedules_task_when_condition_true() {
    let mut wf = dag_wf_when(vec![
        dag_task("a", &[], None),
        dag_task("b", &["a"], Some("{{tasks.a.outputs.result}} == go")),
    ]);
    complete_a_with_result(&mut wf, "go");
    let actions = next_actions(&wf);
    assert!(scheduled_ids(&actions).contains(&"d.b".to_string()));
    assert!(skipped_ids(&actions).is_empty());
}

#[test]
fn skipped_dependency_satisfies_dependents_and_aggregate() {
    // a → b → c. `b` is Skipped; `c` must still become eligible, and a graph
    // of {Succeeded, Skipped} aggregates to Succeeded (Skipped is fulfilled).
    let mut wf = dag_wf_when(vec![
        dag_task("a", &[], None),
        dag_task("b", &["a"], Some("false")),
        dag_task("c", &["b"], None),
    ]);
    record_success(&mut wf, "d.a", "t", Outputs::default());
    put_node(&mut wf, "d.b", WorkflowPhase::Skipped);
    let actions = next_actions(&wf);
    assert!(scheduled_ids(&actions).contains(&"d.c".to_string()));

    record_success(&mut wf, "d.c", "t", Outputs::default());
    assert_eq!(aggregate_phase(&wf), WorkflowPhase::Succeeded);
}
