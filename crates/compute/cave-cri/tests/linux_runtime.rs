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
//!
//! The other tests cover `get_container_logs`, which reads the container's
//! real log file on Linux, and `restore_container`, whose result decides
//! which process `stop`, `kill` and `pause` send real signals to.
//!
//! `start`, `stop`, `kill`, `pause` and `unpause` are run against real
//! processes as well: children of the test process, which any user may
//! signal. On Linux `start_container` currently forks and execs the command
//! without entering any namespace, cgroup or root filesystem, so that is
//! all these tests can claim about it.

#![cfg(target_os = "linux")]

use cave_cri::error::CriResult;
use cave_cri::models::{
    Container, ContainerSpec, ContainerStatus, ExecRequest, ExecResult, NetworkMode, RestartPolicy,
};
use cave_cri::runtime::{
    exec_in_container, get_container_logs, kill_container, pause_container, restore_container,
    start_container, stop_container, unpause_container,
};
use cave_cri::store::ContainerStore;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A running container whose process is this test process.
fn running_container(store: &ContainerStore) -> Uuid {
    insert_container(store, ContainerStatus::Running, Some(std::process::id()))
}

fn insert_container(store: &ContainerStore, status: ContainerStatus, pid: Option<u32>) -> Uuid {
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
        status,
        pid,
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

/// A directory can be opened for reading, but every read(2) from it fails
/// with `EISDIR`. A log that cannot be read ends the listing; it must not
/// be retried forever.
#[test]
fn get_container_logs_ends_when_the_log_file_cannot_be_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = ContainerStore::new();
    let id = running_container(&store);
    let mut container = store.get(&id).unwrap();
    container.log_path = dir.path().to_path_buf();
    store.update(container);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(get_container_logs(id, None, &store));
    });
    let entries = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("get_container_logs did not return within 10 s")
        .unwrap();

    assert!(entries.is_empty());
}

/// `restore_container` verifies the checkpoint manifest and marks the
/// container running; it does not start a process. So there is no pid to
/// record, and on Linux a made-up one is dangerous: `stop_container`,
/// `kill_container` and `pause_container` would send real signals to
/// whichever process happens to have that number.
#[tokio::test]
async fn a_restored_container_records_no_pid_it_did_not_start() {
    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Stopped, None);
    let checkpoint = tempfile::tempdir().unwrap();
    cave_cri::criu::write_manifest(
        checkpoint.path(),
        &cave_cri::criu::CheckpointManifest {
            container_id: id,
            container_name: "linux-runtime-test".into(),
            image_reference: "test:latest".into(),
            runtime_handler: None,
            created_at: chrono::Utc::now(),
            criu_version: "3.19".into(),
            options: Default::default(),
        },
    )
    .unwrap();

    restore_container(id, checkpoint.path().to_str().unwrap(), &store)
        .await
        .unwrap();

    let restored = store.get(&id).unwrap();
    assert_eq!(restored.status, ContainerStatus::Running);
    assert_eq!(restored.pid, None);

    // With no pid there is nobody to signal: stopping only updates state.
    stop_container(id, 0, &store).await.unwrap();
    assert_eq!(store.get(&id).unwrap().status, ContainerStatus::Stopped);
}

// ── signals to real processes ────────────────────────────────────────────────

const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;

/// A real process standing in for a container's init process: a child of
/// the test, killed and reaped when the test ends, whatever its outcome.
struct ContainerProcess(Child);

impl ContainerProcess {
    fn spawn(script: &str) -> Self {
        let child = Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        Self(child)
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }

    /// Wait up to 10 s for the process to end.
    async fn exit_status(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "process {} is still running",
                self.pid()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

impl Drop for ContainerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The state letter from `/proc/<pid>/stat` (proc(5)): `R` running, `S`
/// sleeping, `T` stopped by a signal, `Z` zombie. `None` if there is no
/// such process.
fn process_state(pid: u32) -> Option<char> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit(')').next()?.trim_start().chars().next()
}

/// The parent pid from `/proc/<pid>/stat`.
fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit(')')
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Poll until `condition` holds, for at most 10 s.
async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn pause_and_unpause_stop_and_continue_the_container_process() {
    let process = ContainerProcess::spawn("exec sleep 30");
    let pid = process.pid();
    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Running, Some(pid));

    pause_container(id, &store).await.unwrap();

    eventually("the process is stopped", || process_state(pid) == Some('T')).await;
    assert_eq!(store.get(&id).unwrap().status, ContainerStatus::Paused);

    unpause_container(id, &store).await.unwrap();

    eventually("the process runs again", || {
        matches!(process_state(pid), Some('S' | 'R'))
    })
    .await;
    assert_eq!(store.get(&id).unwrap().status, ContainerStatus::Running);
}

#[tokio::test]
async fn kill_container_delivers_the_requested_signal() {
    let mut process = ContainerProcess::spawn("exec sleep 30");
    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Running, Some(process.pid()));

    kill_container(id, SIGTERM, &store).await.unwrap();

    assert_eq!(process.exit_status().await.signal(), Some(SIGTERM));
}

#[tokio::test]
async fn kill_container_rejects_a_number_that_is_no_signal() {
    let process = ContainerProcess::spawn("exec sleep 30");
    let pid = process.pid();
    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Running, Some(pid));

    let err = kill_container(id, 4096, &store).await.unwrap_err();

    assert!(
        err.to_string().contains("invalid signal: 4096"),
        "got: {err}"
    );
    assert!(matches!(process_state(pid), Some('S' | 'R')));
}

#[tokio::test]
async fn stop_container_terminates_the_container_process() {
    let mut process = ContainerProcess::spawn("exec sleep 30");
    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Running, Some(process.pid()));

    stop_container(id, 0, &store).await.unwrap();

    // SIGTERM is sent first and is fatal for `sleep`. With a grace period
    // of 0 s the SIGKILL follows at once, so either may be reported.
    let signal = process.exit_status().await.signal();
    assert!(
        signal == Some(SIGTERM) || signal == Some(SIGKILL),
        "expected death by SIGTERM or SIGKILL, got {signal:?}"
    );
    let stopped = store.get(&id).unwrap();
    assert_eq!(stopped.status, ContainerStatus::Stopped);
    assert!(stopped.finished_at.is_some());
}

/// A process that ignores SIGTERM is killed once the grace period is over.
#[tokio::test]
async fn stop_container_kills_a_process_that_ignores_sigterm() {
    // An ignored signal stays ignored across exec (signal(7)).
    let mut process = ContainerProcess::spawn("trap '' TERM; exec sleep 30");
    let pid = process.pid();
    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Running, Some(pid));
    // `SigIgn` in /proc/<pid>/status is a hex mask, bit (n - 1) for signal n.
    eventually("the process ignores SIGTERM", || {
        std::fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|status| {
                let line = status.lines().find(|l| l.starts_with("SigIgn:"))?;
                u64::from_str_radix(line.split_whitespace().nth(1)?, 16).ok()
            })
            .is_some_and(|ignored| ignored & (1 << (SIGTERM - 1)) != 0)
    })
    .await;
    let started = Instant::now();

    stop_container(id, 1, &store).await.unwrap();

    assert_eq!(process.exit_status().await.signal(), Some(SIGKILL));
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "SIGKILL must wait for the grace period, took {:?}",
        started.elapsed()
    );
    assert_eq!(store.get(&id).unwrap().status, ContainerStatus::Stopped);
}

/// On Linux `start_container` forks and execs the container's command as a
/// child of the runtime and records its pid.
#[tokio::test]
async fn start_container_runs_the_command_as_a_child_process() {
    use nix::sys::signal::Signal;
    use nix::sys::wait::{waitpid, WaitStatus};
    use nix::unistd::Pid;

    let store = ContainerStore::new();
    let id = insert_container(&store, ContainerStatus::Created, None);
    let mut container = store.get(&id).unwrap();
    container.spec.command = vec!["sleep".into(), "30".into()];
    store.update(container);

    start_container(id, &store).await.unwrap();

    let started = store.get(&id).unwrap();
    assert_eq!(started.status, ContainerStatus::Running);
    let pid = started.pid.expect("a started container has a pid");
    assert_eq!(parent_pid(pid), Some(std::process::id()));
    // Between fork and exec the child still carries the test's name.
    eventually("the child has exec'd the command", || {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|comm| comm == "sleep\n")
    })
    .await;

    // Clean up through the API under test, then reap our child.
    kill_container(id, SIGKILL, &store).await.unwrap();
    let child = Pid::from_raw(pid as i32);
    assert_eq!(
        waitpid(child, None).unwrap(),
        WaitStatus::Signaled(child, Signal::SIGKILL, false)
    );
}
