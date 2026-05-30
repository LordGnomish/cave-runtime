// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD RED for the Tekton entrypoint step-ordering port.
//!
//! Ports the pure step-sequencing algorithm from
//! tektoncd/pipeline v0.55.0 `pkg/pod/entrypoint.go` `orderContainers`,
//! which rewrites each step's command to wrap it with the Tekton
//! entrypoint binary, injecting `-wait_file`/`-post_file` flags so step N
//! only starts after step N-1 has signalled completion. This is pure
//! in-memory argv computation (NOT k8s pod scheduling / volume mounting,
//! which remains cave-cri's concern).

use cave_pipelines::entrypoint::{order_containers, EntrypointStep, ENTRYPOINT_BINARY};

fn step(name: &str, command: Vec<&str>, args: Vec<&str>) -> EntrypointStep {
    EntrypointStep {
        name: name.to_string(),
        command: command.into_iter().map(String::from).collect(),
        args: args.into_iter().map(String::from).collect(),
        results: vec![],
    }
}

#[test]
fn step_zero_has_no_wait_file_by_default() {
    let steps = vec![step("clone", vec!["git"], vec!["clone"])];
    let out = order_containers(&steps, &[]);
    assert_eq!(out[0].command, vec![ENTRYPOINT_BINARY.to_string()]);
    // First step (no waitForReady) must NOT wait on any prior file.
    assert!(!out[0].args.iter().any(|a| a == "-wait_file"));
    // Every step posts its completion file at /tekton/run/0/out.
    let post_idx = out[0].args.iter().position(|a| a == "-post_file").unwrap();
    assert_eq!(out[0].args[post_idx + 1], "/tekton/run/0/out");
}

#[test]
fn step_n_waits_on_previous_step_post_file() {
    let steps = vec![
        step("clone", vec!["git"], vec!["clone"]),
        step("build", vec!["go"], vec!["build"]),
        step("test", vec!["go"], vec!["test"]),
    ];
    let out = order_containers(&steps, &[]);

    // Step 1 waits on step 0's out file.
    let w1 = out[1].args.iter().position(|a| a == "-wait_file").unwrap();
    assert_eq!(out[1].args[w1 + 1], "/tekton/run/0/out");
    // Step 1 posts to its own index.
    let p1 = out[1].args.iter().position(|a| a == "-post_file").unwrap();
    assert_eq!(out[1].args[p1 + 1], "/tekton/run/1/out");

    // Step 2 waits on step 1's out file.
    let w2 = out[2].args.iter().position(|a| a == "-wait_file").unwrap();
    assert_eq!(out[2].args[w2 + 1], "/tekton/run/1/out");
}

#[test]
fn termination_and_metadata_paths_injected() {
    let steps = vec![step("clone", vec!["git"], vec!["clone"])];
    let out = order_containers(&steps, &[]);
    let t = out[0].args.iter().position(|a| a == "-termination_path").unwrap();
    assert_eq!(out[0].args[t + 1], "/tekton/termination");
    let m = out[0].args.iter().position(|a| a == "-step_metadata_dir").unwrap();
    assert_eq!(out[0].args[m + 1], "/tekton/run/0/status");
}

#[test]
fn original_command_preserved_after_separator() {
    // cmd[0] becomes -entrypoint; cmd[1..] + args follow the `--` separator.
    let steps = vec![step("run", vec!["sh", "-c"], vec!["echo hi"])];
    let out = order_containers(&steps, &[]);
    let e = out[0].args.iter().position(|a| a == "-entrypoint").unwrap();
    assert_eq!(out[0].args[e + 1], "sh");
    let sep = out[0].args.iter().position(|a| a == "--").unwrap();
    // After `--`: remaining cmd elements ("-c") then the original args.
    assert_eq!(&out[0].args[sep + 1..], &["-c".to_string(), "echo hi".to_string()]);
}

#[test]
fn step_results_flag_emitted_comma_joined() {
    let mut s = step("emit", vec!["sh"], vec![]);
    s.results = vec!["digest".to_string(), "url".to_string()];
    let out = order_containers(&[s], &[]);
    let r = out[0].args.iter().position(|a| a == "-step_results").unwrap();
    assert_eq!(out[0].args[r + 1], "digest,url");
}

#[test]
fn task_results_flag_emitted_comma_joined() {
    let steps = vec![step("emit", vec!["sh"], vec![])];
    let out = order_containers(&steps, &["commit".to_string(), "tag".to_string()]);
    let r = out[0].args.iter().position(|a| a == "-results").unwrap();
    assert_eq!(out[0].args[r + 1], "commit,tag");
}
