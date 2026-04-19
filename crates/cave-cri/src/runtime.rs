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
