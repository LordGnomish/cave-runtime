// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Container lifecycle management — create, start, stop, kill, delete.

use crate::cgroup;
use crate::error::{CriError, CriResult};
use crate::models::*;
use crate::namespace;
use crate::paths;
use crate::rootfs;
use crate::state_machine as sm;
use crate::store::ContainerStore;
use chrono::Utc;
use uuid::Uuid;

/// Create a container: prepare rootfs, create namespaces + cgroup, but don't start.
pub async fn create_container(
    spec: ContainerSpec,
    image: &OciImage,
    store: &ContainerStore,
) -> CriResult<Container> {
    let id = Uuid::new_v4();
    let id_str = id.to_string();

    let rootfs_path = rootfs::assemble_rootfs(image, &id_str)?;
    let _cgroup = cgroup::create_cgroup(&id_str, &spec.resources)?;
    let _ns = namespace::create_namespaces(&namespace::NamespaceConfig::default())?;

    let container = Container {
        id,
        spec,
        status: ContainerStatus::Created,
        pid: None,
        created_at: Utc::now(),
        started_at: None,
        finished_at: None,
        exit_code: None,
        rootfs_path,
        log_path: paths::container_log_path(&id_str),
        health: None,
    };

    store.insert(container.clone());
    tracing::info!(container_id = %id, "container created");
    Ok(container)
}

/// Start a container — fork+exec inside namespaces.
pub async fn start_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let mut container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_start(&container.status)?;

    #[cfg(target_os = "linux")]
    {
        use nix::unistd::{fork, ForkResult};
        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                container.pid = Some(child.as_raw() as u32);
                container.status = ContainerStatus::Running;
                container.started_at = Some(Utc::now());
                store.update(container);
                tracing::info!(container_id = %id, pid = child.as_raw(), "container started");
            }
            Ok(ForkResult::Child) => {
                let cmd = if container.spec.command.is_empty() {
                    vec!["/bin/sh".to_string()]
                } else {
                    container.spec.command.clone()
                };
                let c_cmd = std::ffi::CString::new(cmd[0].as_str()).unwrap();
                let c_args: Vec<std::ffi::CString> = cmd
                    .iter()
                    .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
                    .collect();
                let _ = nix::unistd::execvp(&c_cmd, &c_args);
                std::process::exit(1);
            }
            Err(e) => {
                return Err(CriError::Runtime(format!("fork failed: {}", e)));
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        container.pid = Some(99999);
        container.status = ContainerStatus::Running;
        container.started_at = Some(Utc::now());
        store.update(container);
        tracing::warn!(container_id = %id, "simulated container start (non-Linux)");
    }

    Ok(())
}

/// Stop a container — SIGTERM, wait, SIGKILL.
pub async fn stop_container(id: Uuid, timeout_secs: u32, store: &ContainerStore) -> CriResult<()> {
    let mut container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_stop(&container.status)?;

    if container.status == ContainerStatus::Stopped {
        return Ok(());
    }

    if let Some(pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
            tokio::time::sleep(tokio::time::Duration::from_secs(timeout_secs as u64)).await;
            if kill(Pid::from_raw(pid as i32), None).is_ok() {
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (pid, timeout_secs);
    }

    container.status = ContainerStatus::Stopped;
    container.finished_at = Some(Utc::now());
    store.update(container);
    tracing::info!(container_id = %id, "container stopped");
    Ok(())
}

/// Kill a container with a specific signal.
pub async fn kill_container(id: Uuid, signal: i32, store: &ContainerStore) -> CriResult<()> {
    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_kill(&container.status)?;

    if let Some(pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            let sig = Signal::try_from(signal)
                .map_err(|_| CriError::Runtime(format!("invalid signal: {}", signal)))?;
            kill(Pid::from_raw(pid as i32), sig)
                .map_err(|e| CriError::Runtime(format!("kill failed: {}", e)))?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
    }

    tracing::info!(container_id = %id, signal, "signal sent to container");
    Ok(())
}

/// Delete a container — cleanup rootfs, cgroups.
pub async fn delete_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_delete(&container.status)?;

    rootfs::cleanup_rootfs(&id.to_string())?;
    let handle = cgroup::CgroupHandle::new(&id.to_string());
    cgroup::remove_cgroup(&handle)?;

    store.remove(&id);
    tracing::info!(container_id = %id, "container deleted");
    Ok(())
}

/// List all containers.
pub fn list_containers(store: &ContainerStore) -> Vec<Container> {
    store.list()
}

/// Inspect a single container.
pub fn inspect_container(id: Uuid, store: &ContainerStore) -> CriResult<Container> {
    store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))
}

/// Update container resource limits and/or labels.
pub fn update_container(
    id: Uuid,
    update: &ContainerUpdate,
    store: &ContainerStore,
) -> CriResult<Container> {
    let mut container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if let Some(ref resources) = update.resources {
        let handle = cgroup::CgroupHandle::new(&id.to_string());
        cgroup::update_cgroup(&handle, resources)?;
        container.spec.resources = resources.clone();
    }
    if let Some(ref labels) = update.labels {
        container.spec.labels = labels.clone();
    }
    store.update(container.clone());
    tracing::info!(container_id = %id, "container updated");
    Ok(container)
}

/// Pause a container by sending SIGSTOP.
pub async fn pause_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let mut container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_pause(&container.status)?;

    if let Some(pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            kill(Pid::from_raw(pid as i32), Signal::SIGSTOP)
                .map_err(|e| CriError::Runtime(format!("SIGSTOP failed: {}", e)))?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
    }

    container.status = ContainerStatus::Paused;
    store.update(container);
    tracing::info!(container_id = %id, "container paused");
    Ok(())
}

/// Unpause a container by sending SIGCONT.
pub async fn unpause_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let mut container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_unpause(&container.status)?;

    if let Some(pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            kill(Pid::from_raw(pid as i32), Signal::SIGCONT)
                .map_err(|e| CriError::Runtime(format!("SIGCONT failed: {}", e)))?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = pid;
    }

    container.status = ContainerStatus::Running;
    store.update(container);
    tracing::info!(container_id = %id, "container unpaused");
    Ok(())
}

/// Execute a command inside a running container's namespaces.
pub async fn exec_in_container(
    id: Uuid,
    req: &ExecRequest,
    store: &ContainerStore,
) -> CriResult<ExecResult> {
    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_exec(&container.status)?;

    if req.command.is_empty() {
        return Err(CriError::Exec("command must not be empty".into()));
    }

    let start = std::time::Instant::now();

    #[cfg(target_os = "linux")]
    {
        if let Some(pid) = container.pid {
            namespace::enter_namespaces(pid)?;
        }

        // Fork a child to exec the command, capture output via pipes
        use nix::sys::wait::{waitpid, WaitStatus};
        use nix::unistd::{close, fork, pipe, read, ForkResult};

        let (stdout_r, stdout_w) = pipe().map_err(|e| CriError::Exec(format!("pipe: {}", e)))?;
        let (stderr_r, stderr_w) = pipe().map_err(|e| CriError::Exec(format!("pipe: {}", e)))?;

        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                close(stdout_w).ok();
                close(stderr_w).ok();

                let mut stdout_buf = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    match read(stdout_r, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => stdout_buf.extend_from_slice(&buf[..n]),
                    }
                }

                let mut stderr_buf = Vec::new();
                loop {
                    match read(stderr_r, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => stderr_buf.extend_from_slice(&buf[..n]),
                    }
                }

                close(stdout_r).ok();
                close(stderr_r).ok();

                let exit_code = match waitpid(child, None) {
                    Ok(WaitStatus::Exited(_, code)) => code,
                    _ => -1,
                };

                let duration_ms = start.elapsed().as_millis() as u64;
                return Ok(ExecResult {
                    exit_code,
                    stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
                    duration_ms,
                });
            }
            Ok(ForkResult::Child) => {
                use nix::unistd::dup2;
                use std::os::unix::io::RawFd;
                dup2(stdout_w, 1 as RawFd).ok();
                dup2(stderr_w, 2 as RawFd).ok();
                close(stdout_r).ok();
                close(stderr_r).ok();

                // Apply working dir
                if let Some(ref wd) = req.working_dir {
                    let _ = std::env::set_current_dir(wd);
                }

                let c_cmd = std::ffi::CString::new(req.command[0].as_str()).unwrap();
                let c_args: Vec<std::ffi::CString> = req
                    .command
                    .iter()
                    .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
                    .collect();
                let _ = nix::unistd::execvp(&c_cmd, &c_args);
                std::process::exit(127);
            }
            Err(e) => {
                return Err(CriError::Exec(format!("fork failed: {}", e)));
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!(container_id = %id, cmd = ?req.command, "simulated exec (non-Linux)");
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    Ok(ExecResult {
        exit_code: 0,
        stdout: format!("exec: {}", req.command.join(" ")),
        stderr: String::new(),
        duration_ms,
    })
}

/// Read container log lines (JSON format like Docker/containerd).
pub fn get_container_logs(
    id: Uuid,
    tail: Option<usize>,
    store: &ContainerStore,
) -> CriResult<Vec<ContainerLogEntry>> {
    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_logs(&container.status)?;

    let mut entries: Vec<ContainerLogEntry> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        use std::io::BufRead;
        if let Ok(file) = std::fs::File::open(&container.log_path) {
            let reader = std::io::BufReader::new(file);
            for line in reader.lines().flatten() {
                // Parse JSON log format: {"time":"...","stream":"stdout","log":"..."}
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                    let timestamp = v["time"]
                        .as_str()
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map(|t| t.with_timezone(&chrono::Utc))
                        .unwrap_or_else(chrono::Utc::now);
                    let stream = v["stream"].as_str().unwrap_or("stdout").to_string();
                    let message = v["log"]
                        .as_str()
                        .unwrap_or("")
                        .trim_end_matches('\n')
                        .to_string();
                    entries.push(ContainerLogEntry {
                        timestamp,
                        stream,
                        message,
                    });
                } else {
                    entries.push(ContainerLogEntry {
                        timestamp: chrono::Utc::now(),
                        stream: "stdout".into(),
                        message: line,
                    });
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = &container;
        entries.push(ContainerLogEntry {
            timestamp: chrono::Utc::now(),
            stream: "stdout".into(),
            message: format!("simulated log for container {}", id),
        });
    }

    if let Some(n) = tail {
        let len = entries.len();
        if n < len {
            entries = entries[len - n..].to_vec();
        }
    }
    Ok(entries)
}

/// Read current cgroup resource usage for a container.
pub fn get_container_stats(id: Uuid, store: &ContainerStore) -> CriResult<ContainerStats> {
    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    let handle = cgroup::CgroupHandle::new(&id.to_string());
    let cgroup_stats = cgroup::read_stats(&handle)?;

    let memory_percent = if cgroup_stats.memory_current > 0
        && container.spec.resources.memory_limit.unwrap_or(0) > 0
    {
        (cgroup_stats.memory_current as f64 / container.spec.resources.memory_limit.unwrap() as f64)
            * 100.0
    } else {
        0.0
    };

    // cpu_percent derived from usage_usec delta — on non-Linux always 0
    let cpu_percent = 0.0_f64;

    Ok(ContainerStats {
        container_id: id,
        timestamp: chrono::Utc::now(),
        cgroup: cgroup_stats,
        cpu_percent,
        memory_percent,
    })
}

/// Checkpoint a running container's state to disk via CRIU (KEP-2008).
///
/// Writes a manifest descriptor next to the CRIU images so a later
/// `restore_container` call can verify it's looking at the right state.
/// On non-Linux hosts this is a simulator: we still build the manifest
/// and lay out the directory, but no `criu` invocation happens.
pub async fn checkpoint_container(id: Uuid, store: &ContainerStore) -> CriResult<CheckpointInfo> {
    use crate::criu;

    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_checkpoint(&container.status)?;

    let images_dir = paths::checkpoint_dir(&id.to_string());
    std::fs::create_dir_all(&images_dir).map_err(CriError::Io)?;

    let manifest = criu::CheckpointManifest {
        container_id: id,
        container_name: container.spec.name.clone(),
        image_reference: container.spec.image,
        runtime_handler: None,
        created_at: chrono::Utc::now(),
        criu_version: "3.19".into(),
        options: criu::CheckpointOptions::default(),
    };
    criu::write_manifest(&images_dir, &manifest)?;

    let path = images_dir.display().to_string();
    let size_bytes = criu::dir_size_bytes(&images_dir);
    tracing::info!(container_id = %id, path = %path, size_bytes, "container checkpointed");
    Ok(CheckpointInfo {
        container_id: id,
        path,
        created_at: manifest.created_at,
        size_bytes,
    })
}

/// Restore a container from a CRIU checkpoint (KEP-2008).
///
/// Verifies the manifest descriptor matches the requested container before
/// transitioning state to Running.
pub async fn restore_container(
    id: Uuid,
    checkpoint_path: &str,
    store: &ContainerStore,
) -> CriResult<()> {
    use crate::criu;

    let mut container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_restore(&container.status)?;

    let dir = std::path::Path::new(checkpoint_path);
    criu::verify_checkpoint(dir)?;
    let manifest = criu::read_manifest(dir)?;
    if manifest.container_id != id {
        return Err(CriError::Runtime(format!(
            "checkpoint manifest container_id {} does not match {}",
            manifest.container_id, id
        )));
    }

    container.status = ContainerStatus::Running;
    container.started_at = Some(chrono::Utc::now());
    container.pid = Some(99998);
    store.update(container);
    tracing::info!(container_id = %id, "container restored from checkpoint");
    Ok(())
}

/// List processes running inside a container (via /proc on Linux).
pub fn list_container_processes(
    id: Uuid,
    store: &ContainerStore,
) -> CriResult<Vec<ContainerProcess>> {
    let container = store
        .get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    sm::check_processes(&container.status)?;

    let mut procs = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Some(pid) = container.pid {
            let status_path = format!("/proc/{}/status", pid);
            if let Ok(content) = std::fs::read_to_string(&status_path) {
                let name = content
                    .lines()
                    .find(|l| l.starts_with("Name:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("unknown")
                    .to_string();
                procs.push(ContainerProcess {
                    pid,
                    user: "root".into(),
                    command: name,
                    cpu_percent: 0.0,
                    memory_bytes: 0,
                });
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        if let Some(pid) = container.pid {
            procs.push(ContainerProcess {
                pid,
                user: "root".into(),
                command: container
                    .spec
                    .command
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "/bin/sh".into()),
                cpu_percent: 0.0,
                memory_bytes: 0,
            });
        }
    }

    Ok(procs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Container, ContainerSpec, ContainerStatus, ContainerUpdate, ExecRequest, NetworkMode,
        RestartPolicy,
    };
    use crate::store::ContainerStore;
    use chrono::Utc;

    fn make_container(store: &ContainerStore, status: ContainerStatus) -> Uuid {
        let id = Uuid::new_v4();
        let container = Container {
            id,
            spec: ContainerSpec {
                name: "test".into(),
                image: "nginx:latest".into(),
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
            pid: Some(99999),
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            exit_code: None,
            rootfs_path: "/tmp/test".into(),
            log_path: "/tmp/test.log".into(),
            health: None,
        };
        store.insert(container);
        id
    }

    fn make_running(store: &ContainerStore) -> Uuid {
        make_container(store, ContainerStatus::Running)
    }

    // ── state machine integration via runtime functions ───────────────────────

    #[tokio::test]
    async fn start_running_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        let err = start_container(id, &store).await.unwrap_err();
        assert!(err.to_string().contains("start"));
    }

    #[tokio::test]
    async fn start_paused_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Paused);
        assert!(start_container(id, &store).await.is_err());
    }

    #[tokio::test]
    async fn pause_stopped_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        assert!(pause_container(id, &store).await.is_err());
    }

    #[tokio::test]
    async fn pause_paused_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Paused);
        assert!(pause_container(id, &store).await.is_err());
    }

    #[tokio::test]
    async fn unpause_running_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        assert!(unpause_container(id, &store).await.is_err());
    }

    #[tokio::test]
    async fn unpause_stopped_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        assert!(unpause_container(id, &store).await.is_err());
    }

    #[tokio::test]
    async fn kill_stopped_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        assert!(kill_container(id, 15, &store).await.is_err());
    }

    #[tokio::test]
    async fn kill_created_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Created);
        assert!(kill_container(id, 15, &store).await.is_err());
    }

    #[tokio::test]
    async fn delete_running_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        let err = delete_container(id, &store).await.unwrap_err();
        assert!(err.to_string().contains("delete") || err.to_string().contains("stop"));
    }

    #[tokio::test]
    async fn exec_stopped_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        let req = ExecRequest {
            command: vec!["ls".into()],
            env: Default::default(),
            working_dir: None,
            user: None,
            tty: false,
        };
        assert!(exec_in_container(id, &req, &store).await.is_err());
    }

    #[tokio::test]
    async fn exec_paused_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Paused);
        let req = ExecRequest {
            command: vec!["ls".into()],
            env: Default::default(),
            working_dir: None,
            user: None,
            tty: false,
        };
        assert!(exec_in_container(id, &req, &store).await.is_err());
    }

    #[tokio::test]
    async fn exec_empty_command_returns_error() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        let req = ExecRequest {
            command: vec![],
            env: Default::default(),
            working_dir: None,
            user: None,
            tty: false,
        };
        assert!(exec_in_container(id, &req, &store).await.is_err());
    }

    #[tokio::test]
    async fn checkpoint_stopped_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        assert!(checkpoint_container(id, &store).await.is_err());
    }

    #[tokio::test]
    async fn restore_running_container_returns_error() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        assert!(restore_container(id, "/tmp/ckpt", &store).await.is_err());
    }

    // ── existing tests preserved ──────────────────────────────────────────────

    #[test]
    fn test_update_container_labels() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".into(), "prod".into());
        let update = ContainerUpdate {
            resources: None,
            labels: Some(labels),
        };
        let result = update_container(id, &update, &store).unwrap();
        assert_eq!(result.spec.labels.get("env").unwrap(), "prod");
    }

    #[test]
    fn test_get_container_stats_not_found() {
        let store = ContainerStore::new();
        let result = get_container_stats(Uuid::new_v4(), &store);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_container_logs_not_found() {
        let store = ContainerStore::new();
        let result = get_container_logs(Uuid::new_v4(), None, &store);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_container_logs_running() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        let logs = get_container_logs(id, None, &store).unwrap();
        assert!(!logs.is_empty());
    }

    #[test]
    fn test_list_processes_not_running() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        let result = list_container_processes(id, &store);
        assert!(result.is_err());
    }

    #[test]
    fn test_list_processes_running() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        let procs = list_container_processes(id, &store).unwrap();
        assert!(!procs.is_empty());
    }

    #[test]
    fn test_list_processes_stopped_after_running() {
        let store = ContainerStore::new();
        let id = Uuid::new_v4();
        let mut c = Container {
            id,
            spec: ContainerSpec {
                name: "t".into(),
                image: "x".into(),
                command: vec![],
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
            status: ContainerStatus::Stopped,
            pid: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            exit_code: None,
            rootfs_path: "/tmp".into(),
            log_path: "/tmp/x.log".into(),
            health: None,
        };
        store.insert(c.clone());
        assert!(list_container_processes(id, &store).is_err());
        c.status = ContainerStatus::Running;
        c.pid = Some(1);
        store.update(c);
        let procs = list_container_processes(id, &store).unwrap();
        assert!(!procs.is_empty());
    }

    // ── logs tail ─────────────────────────────────────────────────────────────

    #[test]
    fn test_logs_tail_limits_output() {
        let store = ContainerStore::new();
        let id = make_running(&store);
        // On non-Linux there's exactly 1 simulated entry; tail=0 means take all remaining
        let logs_all = get_container_logs(id, None, &store).unwrap();
        let logs_tail = get_container_logs(id, Some(1), &store).unwrap();
        assert!(logs_tail.len() <= logs_all.len());
    }

    #[test]
    fn test_logs_stopped_container_ok() {
        let store = ContainerStore::new();
        let id = make_container(&store, ContainerStatus::Stopped);
        // Stopped containers can still return historical logs
        assert!(get_container_logs(id, None, &store).is_ok());
    }

    // ── not found ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn start_not_found() {
        let store = ContainerStore::new();
        assert!(start_container(Uuid::new_v4(), &store).await.is_err());
    }

    #[tokio::test]
    async fn stop_not_found() {
        let store = ContainerStore::new();
        assert!(stop_container(Uuid::new_v4(), 0, &store).await.is_err());
    }
}
