// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Step-sequencing (entrypoint ordering) — pure argv-rewriting algorithm.
//!
//! Faithful line-port of the pure portion of
//! tektoncd/pipeline v0.55.0 `pkg/pod/entrypoint.go` `orderContainers`
//! (and its helpers `resultArgument`, `stepResultArgument`,
//! `collectResultsName`).
//!
//! Tekton serialises the steps of a TaskRun by rewriting every step's
//! command so it is wrapped by the `/tekton/bin/entrypoint` binary. Each
//! step is given a `-wait_file` pointing at the previous step's
//! `/tekton/run/{i-1}/out` post-file, and a `-post_file` at its own
//! `/tekton/run/{i}/out`. Because step N blocks on step N-1's post-file,
//! the steps execute strictly in order even though they share one pod.
//!
//! This computation is pure in-memory argv assembly — it does NOT schedule
//! pods, mount volumes, or talk to the kubelet. The k8s pod lifecycle
//! (init-container sidecars, workspace volume mounts, runtime class) stays
//! in cave-cri per ADR-RUNTIME-PARITY-100-PCT-001 §5. Only the
//! step-ordering algorithm is ported here.

/// The entrypoint binary that wraps every step.
/// Upstream: `entrypointBinary = "/tekton/bin/entrypoint"`.
pub const ENTRYPOINT_BINARY: &str = "/tekton/bin/entrypoint";

/// Upstream constant `RunDir`.
const RUN_DIR: &str = "/tekton/run";
/// Upstream constant `terminationPath`.
const TERMINATION_PATH: &str = "/tekton/termination";

/// One step to be ordered. Mirrors the subset of `corev1.Container` that
/// `orderContainers` reads/writes: name, command, args, and the step's
/// declared result names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrypointStep {
    pub name: String,
    /// Original entrypoint command (`s.Command`). `command[0]` becomes the
    /// `-entrypoint` value; `command[1..]` is prepended to `args`.
    pub command: Vec<String>,
    /// Original args (`s.Args`).
    pub args: Vec<String>,
    /// Names of step-level results this step emits (`s.Results`).
    pub results: Vec<String>,
}

/// `stepResultArgument` — emit `-step_results <comma-joined>` or nothing.
fn step_result_argument(step_results: &[String]) -> Vec<String> {
    if step_results.is_empty() {
        return Vec::new();
    }
    vec!["-step_results".to_string(), step_results.join(",")]
}

/// `resultArgument` / `collectResultsName` — emit `-results <comma-joined>`
/// task-result names, or nothing when there are none.
fn result_argument(task_results: &[String]) -> Vec<String> {
    if task_results.is_empty() {
        return Vec::new();
    }
    vec!["-results".to_string(), task_results.join(",")]
}

/// Port of `orderContainers` (the pure step-ordering core).
///
/// Rewrites each step so it is launched through the entrypoint binary and
/// blocks on the previous step's post-file. `task_results` are the names of
/// the TaskSpec-level results (used for the `-results` flag on every step;
/// upstream passes the full TaskResult slice — here we accept the resolved
/// names since the value-presence filtering of `collectResultsName` is done
/// by the caller that owns the TaskSpec).
///
/// `waitForReadyAnnotation` and the breakpoint/debug, on-error, timeout,
/// stdout/stderr-path flags are caller-driven runtime concerns that are not
/// part of the deterministic ordering core; they are intentionally not
/// modelled here (those live with cave-cri's container plumbing).
pub fn order_containers(
    steps: &[EntrypointStep],
    task_results: &[String],
) -> Vec<EntrypointStep> {
    let mut out: Vec<EntrypointStep> = Vec::with_capacity(steps.len());

    for (i, s) in steps.iter().enumerate() {
        let idx = i.to_string();
        let mut args_for_entrypoint: Vec<String> = Vec::new();

        // Step 0 (without the wait-for-ready annotation) waits on nothing;
        // every later step waits on the prior step's post-file.
        if i > 0 {
            args_for_entrypoint.push("-wait_file".to_string());
            args_for_entrypoint.push(format!("{}/{}/out", RUN_DIR, i - 1));
        }

        args_for_entrypoint.push("-post_file".to_string());
        args_for_entrypoint.push(format!("{}/{}/out", RUN_DIR, idx));
        args_for_entrypoint.push("-termination_path".to_string());
        args_for_entrypoint.push(TERMINATION_PATH.to_string());
        args_for_entrypoint.push("-step_metadata_dir".to_string());
        args_for_entrypoint.push(format!("{}/{}/status", RUN_DIR, idx));

        // -step_results (this step's declared results)
        args_for_entrypoint.extend(step_result_argument(&s.results));
        // -results (TaskSpec-level results)
        args_for_entrypoint.extend(result_argument(task_results));

        // Original command/args reassembly: cmd[0] -> -entrypoint,
        // cmd[1..] prepended to args, separated from flags by `--`.
        let cmd = &s.command;
        let mut args = s.args.clone();
        if !cmd.is_empty() {
            args_for_entrypoint.push("-entrypoint".to_string());
            args_for_entrypoint.push(cmd[0].clone());
        }
        if cmd.len() > 1 {
            let mut prepended: Vec<String> = cmd[1..].to_vec();
            prepended.extend(args);
            args = prepended;
        }
        args_for_entrypoint.push("--".to_string());
        args_for_entrypoint.extend(args);

        out.push(EntrypointStep {
            name: s.name.clone(),
            command: vec![ENTRYPOINT_BINARY.to_string()],
            args: args_for_entrypoint,
            results: s.results.clone(),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_step_no_wait_file() {
        let steps = vec![EntrypointStep {
            name: "a".into(),
            command: vec!["true".into()],
            args: vec![],
            results: vec![],
        }];
        let out = order_containers(&steps, &[]);
        assert_eq!(out[0].command, vec![ENTRYPOINT_BINARY.to_string()]);
        assert!(!out[0].args.iter().any(|a| a == "-wait_file"));
    }

    #[test]
    fn three_steps_chain_wait_post() {
        let steps: Vec<EntrypointStep> = ["a", "b", "c"]
            .iter()
            .map(|n| EntrypointStep {
                name: n.to_string(),
                command: vec!["sh".into()],
                args: vec![],
                results: vec![],
            })
            .collect();
        let out = order_containers(&steps, &[]);
        // chain: step i waits on i-1's out, posts to its own out.
        for i in 0..3 {
            let p = out[i].args.iter().position(|a| a == "-post_file").unwrap();
            assert_eq!(out[i].args[p + 1], format!("/tekton/run/{i}/out"));
            if i > 0 {
                let w = out[i].args.iter().position(|a| a == "-wait_file").unwrap();
                assert_eq!(out[i].args[w + 1], format!("/tekton/run/{}/out", i - 1));
            }
        }
    }
}
