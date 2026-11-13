// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Linux code paths of `cave_cri::runtime`, exercised without privileges.
//!
//! On Linux `exec_in_container` runs a real process. The container in these
//! tests has the test process's own pid: it shares every namespace with the
//! caller, so there is no namespace to join and the command simply runs as
//! a child of the test, with its stdout and stderr captured over pipes.
//! That needs no privileges. What it cannot show is an exec into a
//! container that really has namespaces of its own.
//!
//! The commands are `/bin/sh` scripts using only POSIX utilities.

#![cfg(target_os = "linux")]

use cave_cri::error::CriResult;
use cave_cri::models::{
    Container, ContainerSpec, ContainerStatus, ExecRequest, ExecResult, NetworkMode, RestartPolicy,
};
use cave_cri::runtime::exec_in_container;
use cave_cri::store::ContainerStore;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A running container whose process is this test process.
fn running_container(store: &ContainerStore) -> Uuid {
    let id = Uuid::new_v4();
    store.insert(Container {
        id,
        spec: ContainerSpec {
            name: "linux-runtime-test".into(),
            image: "test:latest".into(),
            command: vec!["/bin/sh".into()],
            args: vec![],
            env: Default::default(),
            mounts: vec![],
            resources: Default::default(),
            labels: Default::default(),
            working_dir: None,
            user: None,
            hostname: None,
            network_mode: NetworkMode::Bridge,
            restart_policy: RestartPolicy::Never,
        },
        status: ContainerStatus::Running,
        pid: Some(std::process::id()),
        created_at: chrono::Utc::now(),
        started_at: Some(chrono::Utc::now()),
        finished_at: None,
        exit_code: None,
        rootfs_path: "/".into(),
        log_path: "/nonexistent/linux-runtime-test.log".into(),
        health: None,
    });
    id
}

fn sh(script: &str) -> ExecRequest {
    ExecRequest {
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        env: Default::default(),
        working_dir: None,
        user: None,
        tty: false,
    }
}

/// Run one exec on a thread and runtime of its own and wait at most `limit`
/// for it. `None` means it did not finish: a blocked exec cannot be timed
/// out from inside its own runtime, and a test must fail rather than hang.
fn exec_within(limit: Duration, req: ExecRequest) -> Option<CriResult<ExecResult>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let store = ContainerStore::new();
        let id = running_container(&store);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = tx.send(runtime.block_on(exec_in_container(id, &req, &store)));
    });
    rx.recv_timeout(limit).ok()
}

#[tokio::test]
async fn exec_captures_stdout_stderr_and_the_exit_code() {
    let store = ContainerStore::new();
    let id = running_container(&store);

    let result = exec_in_container(id, &sh("printf out; printf err >&2; exit 3"), &store)
        .await
        .unwrap();

    assert_eq!(result.stdout, "out");
    assert_eq!(result.stderr, "err");
    assert_eq!(result.exit_code, 3);
}

#[tokio::test]
async fn exec_reports_minus_one_for_a_command_killed_by_a_signal() {
    let store = ContainerStore::new();
    let id = running_container(&store);

    let result = exec_in_container(id, &sh("kill -KILL $$"), &store)
        .await
        .unwrap();

    assert_eq!(result.exit_code, -1);
}

#[tokio::test]
async fn exec_runs_the_command_in_the_requested_working_directory() {
    let store = ContainerStore::new();
    let id = running_container(&store);
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let mut req = sh("pwd -P");
    req.working_dir = Some(dir_path.to_str().unwrap().to_string());

    let result = exec_in_container(id, &req, &store).await.unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout.trim_end(), dir_path.to_str().unwrap());
}

/// A pipe holds 64 KiB by default (pipe(7)). A command that writes more than
/// that to stderr before it closes stdout blocks until someone reads
/// stderr, so both pipes have to be drained at the same time.
#[test]
fn exec_drains_stdout_and_stderr_concurrently() {
    let megabyte = 1024 * 1024;
    let req = sh("head -c 1048576 /dev/zero | tr '\\0' e >&2; printf done");

    let result = exec_within(Duration::from_secs(30), req)
        .expect("exec did not finish within 30 s: the command is blocked on a full pipe")
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "done");
    assert_eq!(result.stderr.len(), megabyte);
    assert!(result.stderr.bytes().all(|b| b == b'e'));
}

/// The command must not read whatever the runtime's own stdin happens to
/// be. It gets an empty stdin: `cat` sees end-of-file at once.
#[test]
fn exec_gives_the_command_an_empty_stdin() {
    let result = exec_within(Duration::from_secs(30), sh("cat; printf eof"))
        .expect("exec did not finish within 30 s: the command is reading the runtime's stdin")
        .unwrap();

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "eof");
}

/// `exec_in_container` is an `async fn` that axum calls on a runtime
/// worker. While the command runs, that thread has to stay free for other
/// tasks: here a 50 ms timer shares a single-threaded runtime with an exec
/// of `sleep 2` and must fire long before the exec is done.
#[tokio::test]
async fn exec_does_not_block_the_runtime_thread() {
    let store = ContainerStore::new();
    let id = running_container(&store);
    let started = Instant::now();
    let req = sh("sleep 2");

    let exec = exec_in_container(id, &req, &store);
    let timer = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        started.elapsed()
    };
    let (result, timer_fired_after) = tokio::join!(exec, timer);

    assert_eq!(result.unwrap().exit_code, 0);
    assert!(
        timer_fired_after < Duration::from_secs(1),
        "a 50 ms timer fired after {timer_fired_after:?}: the exec blocked the runtime thread"
    );
}

/// A command that cannot be started is an error that says why, not a
/// result with an exit code and no output.
#[tokio::test]
async fn exec_fails_for_a_command_that_cannot_be_started() {
    let store = ContainerStore::new();
    let id = running_container(&store);
    let req = ExecRequest {
        command: vec!["/nonexistent/cave-cri-no-such-command".into()],
        env: Default::default(),
        working_dir: None,
        user: None,
        tty: false,
    };

    let err = exec_in_container(id, &req, &store).await.unwrap_err();

    let message = err.to_string();
    assert!(
        message.starts_with("exec error:"),
        "expected an exec error, got: {message}"
    );
    assert!(
        message.contains("/nonexistent/cave-cri-no-such-command"),
        "the error must name the command: {message}"
    );
    assert!(
        message.contains("No such file or directory"),
        "the error must say why: {message}"
    );
}

/// When the caller stops waiting (a request that was cancelled or timed
/// out), nothing will ever collect the command's output. The command must
/// not be left running in the container.
#[tokio::test]
async fn exec_kills_the_command_when_the_caller_stops_waiting() {
    let store = ContainerStore::new();
    let id = running_container(&store);
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    // `exec` keeps the shell's pid for `sleep`, so the pid in the file is
    // the pid of the process that has to go away.
    let req = sh(&format!("echo $$ > {}; exec sleep 20", pid_file.display()));
    let started = Instant::now();

    let exec = exec_in_container(id, &req, &store);
    let command_started = async {
        loop {
            let written = std::fs::read_to_string(&pid_file).unwrap_or_default();
            if let Ok(pid) = written.trim().parse::<i32>() {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    // Whichever branch loses is dropped: once the command has started, the
    // caller walks away from the exec.
    let pid = tokio::select! {
        result = exec => panic!("the 20 s command was waited for: {result:?}"),
        pid = command_started => pid,
    };

    // The process is gone, or a zombie that only waits to be reaped.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| stat.rsplit(')').next().map(str::to_owned))
            .and_then(|rest| rest.split_whitespace().next().map(str::to_owned));
        match state.as_deref() {
            None | Some("Z") | Some("X") => break,
            Some(other) => assert!(
                Instant::now() < deadline,
                "process {pid} is still running (state {other}) after the exec was abandoned"
            ),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}, the command ran to its end",
        started.elapsed()
    );
}
