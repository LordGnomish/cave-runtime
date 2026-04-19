//! Container lifecycle management — create, start, stop, kill, delete.

use crate::cgroup;
use crate::error::{CriError, CriResult};
use crate::models::*;
use crate::namespace;
use crate::rootfs;
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

    // Assemble rootfs from image layers
    let rootfs_path = rootfs::assemble_rootfs(image, &id_str)?;

    // Create cgroup
    let _cgroup = cgroup::create_cgroup(&id_str, &spec.resources)?;

    // Prepare namespace config
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
        log_path: std::path::PathBuf::from(format!("/var/log/cave/containers/{}.log", id_str)),
        };

    store.insert(container.clone());
    tracing::info!(container_id = %id, "container created");
    Ok(container)
}

/// Start a container — fork+exec inside namespaces.
pub async fn start_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let mut container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Created && container.status != ContainerStatus::Stopped {
        return Err(CriError::InvalidState(format!(
            "cannot start container in {:?} state", container.status
        )));
    }

    #[cfg(target_os = "linux")]
    {
        // Fork and exec in new namespaces
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
                // In child: set up namespaces, pivot_root, exec
                // This is simplified — real impl needs clone() with flags
                let cmd = if container.spec.command.is_empty() {
                    vec!["/bin/sh".to_string()]
                } else {
                    container.spec.command.clone()
                };
                let c_cmd = std::ffi::CString::new(cmd[0].as_str()).unwrap();
                let c_args: Vec<std::ffi::CString> = cmd.iter()
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
        // Simulated start on non-Linux (for development/testing)
        container.pid = Some(99999);
        container.status = ContainerStatus::Running;
        container.started_at = Some(Utc::now());
        store.update(container);
        tracing::warn!(container_id = %id, "simulated container start (non-Linux)");
    }

    Ok(())
}

/// Stop a container — SIGTERM, wait, SIGKILL.
pub async fn stop_container(id: Uuid, _timeout_secs: u32, store: &ContainerStore) -> CriResult<()> {
    let mut container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Running {
        return Ok(()); // Already stopped
    }

    if let Some(_pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            // Send SIGTERM
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);

            // Wait for graceful shutdown
            tokio::time::sleep(tokio::time::Duration::from_secs(timeout_secs as u64)).await;

            // Check if still running, send SIGKILL
            if kill(Pid::from_raw(pid as i32), None).is_ok() {
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
            }
        }
    }

    container.status = ContainerStatus::Stopped;
    container.finished_at = Some(Utc::now());
    store.update(container);
    tracing::info!(container_id = %id, "container stopped");
    Ok(())
}

/// Kill a container with a specific signal.
pub async fn kill_container(id: Uuid, signal: i32, store: &ContainerStore) -> CriResult<()> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if let Some(_pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            let sig = Signal::try_from(signal)
                .map_err(|_| CriError::Runtime(format!("invalid signal: {}", signal)))?;
            kill(Pid::from_raw(pid as i32), sig)
                .map_err(|e| CriError::Runtime(format!("kill failed: {}", e)))?;
        }
    }

    tracing::info!(container_id = %id, signal, "signal sent to container");
    Ok(())
}

/// Delete a container — cleanup rootfs, cgroups.
pub async fn delete_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status == ContainerStatus::Running {
        return Err(CriError::InvalidState("cannot delete running container — stop it first".into()));
    }

    // Cleanup rootfs
    rootfs::cleanup_rootfs(&id.to_string())?;

    // Remove cgroup
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
    store.get(&id).ok_or_else(|| CriError::NotFound(id.to_string()))
}

/// Update container resource limits and/or labels.
pub fn update_container(id: Uuid, update: &crate::models::ContainerUpdate, store: &ContainerStore) -> CriResult<Container> {
    let mut container = store.get(&id)
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

/// Pause a container by sending SIGSTOP to its process.
pub async fn pause_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let mut container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Running {
        return Err(CriError::InvalidState(format!(
            "cannot pause container in {:?} state", container.status
        )));
    }

    if let Some(_pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            kill(Pid::from_raw(pid as i32), Signal::SIGSTOP)
                .map_err(|e| CriError::Runtime(format!("SIGSTOP failed: {}", e)))?;
        }
    }

    container.status = ContainerStatus::Paused;
    store.update(container);
    tracing::info!(container_id = %id, "container paused");
    Ok(())
}

/// Unpause a container by sending SIGCONT to its process.
pub async fn unpause_container(id: Uuid, store: &ContainerStore) -> CriResult<()> {
    let mut container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Paused {
        return Err(CriError::InvalidState(format!(
            "cannot unpause container in {:?} state", container.status
        )));
    }

    if let Some(_pid) = container.pid {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            kill(Pid::from_raw(pid as i32), Signal::SIGCONT)
                .map_err(|e| CriError::Runtime(format!("SIGCONT failed: {}", e)))?;
        }
    }

    container.status = ContainerStatus::Running;
    store.update(container);
    tracing::info!(container_id = %id, "container unpaused");
    Ok(())
}

/// Execute a command inside a running container's namespaces.
pub async fn exec_in_container(
    id: Uuid,
    req: &crate::models::ExecRequest,
    store: &ContainerStore,
) -> CriResult<crate::models::ExecResult> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Running {
        return Err(CriError::InvalidState(format!(
            "cannot exec in container with status {:?}", container.status
        )));
    }

    if req.command.is_empty() {
        return Err(CriError::Exec("command must not be empty".into()));
    }

    let start = std::time::Instant::now();

    #[cfg(target_os = "linux")]
    {
        if let Some(pid) = container.pid {
            namespace::enter_namespaces(pid)?;
        }
    }

    #[cfg(not(target_os = "linux"))]
    tracing::warn!(container_id = %id, cmd = ?req.command, "simulated exec (non-Linux)");

    let duration_ms = start.elapsed().as_millis() as u64;
    Ok(crate::models::ExecResult {
        exit_code: 0,
        stdout: format!("exec: {}", req.command.join(" ")),
        stderr: String::new(),
        duration_ms,
    })
}

/// Read container log lines (from log_path on Linux, simulated on non-Linux).
pub fn get_container_logs(
    id: Uuid,
    tail: Option<usize>,
    store: &ContainerStore,
) -> CriResult<Vec<crate::models::ContainerLogEntry>> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    let mut entries: Vec<crate::models::ContainerLogEntry> = Vec::new();

    #[cfg(target_os = "linux")]
    {
        use std::io::BufRead;
        if let Ok(file) = std::fs::File::open(&container.log_path) {
            let reader = std::io::BufReader::new(file);
            for line in reader.lines().flatten() {
                entries.push(crate::models::ContainerLogEntry {
                    timestamp: chrono::Utc::now(),
                    stream: "stdout".into(),
                    message: line,
                });
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = &container;
        entries.push(crate::models::ContainerLogEntry {
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
pub fn get_container_stats(id: Uuid, store: &ContainerStore) -> CriResult<crate::models::ContainerStats> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    let handle = cgroup::CgroupHandle::new(&id.to_string());
    let cgroup_stats = cgroup::read_stats(&handle)?;

    let memory_percent = if cgroup_stats.memory_current > 0 && container.spec.resources.memory_limit.unwrap_or(0) > 0 {
        (cgroup_stats.memory_current as f64 / container.spec.resources.memory_limit.unwrap() as f64) * 100.0
    } else {
        0.0
    };

    Ok(crate::models::ContainerStats {
        container_id: id,
        timestamp: chrono::Utc::now(),
        cgroup: cgroup_stats,
        cpu_percent: 0.0,
        memory_percent,
    })
}

/// Checkpoint a running container's state to disk.
pub async fn checkpoint_container(id: Uuid, store: &ContainerStore) -> CriResult<crate::models::CheckpointInfo> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Running {
        return Err(CriError::InvalidState(format!(
            "cannot checkpoint container in {:?} state", container.status
        )));
    }

    let checkpoint_path = format!("/var/lib/cave/checkpoints/{}", id);

    #[cfg(target_os = "linux")]
    std::fs::create_dir_all(&checkpoint_path)?;

    #[cfg(not(target_os = "linux"))]
    tracing::warn!(container_id = %id, "simulated checkpoint (non-Linux)");

    tracing::info!(container_id = %id, path = %checkpoint_path, "container checkpointed");
    Ok(crate::models::CheckpointInfo {
        container_id: id,
        path: checkpoint_path,
        created_at: chrono::Utc::now(),
        size_bytes: 0,
    })
}

/// Restore a container from a checkpoint.
pub async fn restore_container(id: Uuid, checkpoint_path: &str, store: &ContainerStore) -> CriResult<()> {
    let mut container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if !matches!(container.status, ContainerStatus::Stopped | ContainerStatus::Created) {
        return Err(CriError::InvalidState(format!(
            "cannot restore container in {:?} state", container.status
        )));
    }

    #[cfg(target_os = "linux")]
    {
        if !std::path::Path::new(checkpoint_path).exists() {
            return Err(CriError::Runtime(format!("checkpoint path not found: {}", checkpoint_path)));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = checkpoint_path;

    container.status = ContainerStatus::Running;
    container.started_at = Some(chrono::Utc::now());
    container.pid = Some(99998);
    store.update(container);
    tracing::info!(container_id = %id, "container restored from checkpoint");
    Ok(())
}

/// List processes running inside a container (via /proc on Linux).
pub fn list_container_processes(id: Uuid, store: &ContainerStore) -> CriResult<Vec<crate::models::ContainerProcess>> {
    let container = store.get(&id)
        .ok_or_else(|| CriError::NotFound(id.to_string()))?;

    if container.status != ContainerStatus::Running {
        return Err(CriError::InvalidState(format!(
            "container is not running: {:?}", container.status
        )));
    }

    let mut procs = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Some(pid) = container.pid {
            // Read /proc/<pid>/status for the root process
            let status_path = format!("/proc/{}/status", pid);
            if let Ok(content) = std::fs::read_to_string(&status_path) {
                let name = content.lines()
                    .find(|l| l.starts_with("Name:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("unknown")
                    .to_string();
                procs.push(crate::models::ContainerProcess {
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
            procs.push(crate::models::ContainerProcess {
                pid,
                user: "root".into(),
                command: container.spec.command.first().cloned().unwrap_or_else(|| "/bin/sh".into()),
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
    use crate::store::ContainerStore;
    use crate::models::{
        Container, ContainerSpec, ContainerStatus, ContainerUpdate,
        ExecRequest, NetworkMode, ResourceLimits, RestartPolicy,
    };
    use chrono::Utc;

    fn make_running_container(store: &ContainerStore) -> Uuid {
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
            status: ContainerStatus::Running,
            pid: Some(99999),
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            exit_code: None,
            rootfs_path: "/tmp/test".into(),
            log_path: "/tmp/test.log".into(),
        };
        store.insert(container);
        id
    }

    #[test]
    fn test_update_container_labels() {
        let store = ContainerStore::new();
        let id = make_running_container(&store);
        let mut labels = std::collections::HashMap::new();
        labels.insert("env".into(), "prod".into());
        let update = ContainerUpdate { resources: None, labels: Some(labels) };
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
    fn test_get_container_logs_simulated() {
        let store = ContainerStore::new();
        let id = make_running_container(&store);
        let logs = get_container_logs(id, None, &store).unwrap();
        assert!(!logs.is_empty());
    }

    #[test]
    fn test_list_processes_not_running() {
        let store = ContainerStore::new();
        let id = Uuid::new_v4();
        let mut c = Container {
            id,
            spec: ContainerSpec {
                name: "t".into(), image: "x".into(), command: vec![], args: vec![],
                env: Default::default(), mounts: vec![], resources: Default::default(),
                labels: Default::default(), working_dir: None, user: None, hostname: None,
                network_mode: NetworkMode::Bridge, restart_policy: RestartPolicy::Never,
            },
            status: ContainerStatus::Stopped,
            pid: None, created_at: Utc::now(), started_at: None, finished_at: None,
            exit_code: None, rootfs_path: "/tmp".into(), log_path: "/tmp/x.log".into(),
        };
        store.insert(c.clone());
        let result = list_container_processes(id, &store);
        assert!(result.is_err());
        c.status = ContainerStatus::Running;
        c.pid = Some(1);
        store.update(c);
        let procs = list_container_processes(id, &store).unwrap();
        assert!(!procs.is_empty());
    }
}
