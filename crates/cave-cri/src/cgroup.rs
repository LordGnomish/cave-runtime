//! cgroup v2 resource limit management.
//!
//! Creates and manages cgroup directories under /sys/fs/cgroup/ for
//! container resource isolation (CPU, memory, PIDs).

use crate::error::CriResult;
use crate::models::{CgroupStats, ResourceLimits};
use std::path::PathBuf;

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const CAVE_CGROUP_PREFIX: &str = "cave";

/// Handle to a container's cgroup.
#[derive(Debug, Clone)]
pub struct CgroupHandle {
    pub path: PathBuf,
    pub container_id: String,
}

impl CgroupHandle {
    pub fn new(container_id: &str) -> Self {
        Self {
            path: PathBuf::from(CGROUP_ROOT)
                .join(CAVE_CGROUP_PREFIX)
                .join(container_id),
            container_id: container_id.to_string(),
        }
    }
}

/// Create a cgroup v2 directory and apply resource limits.
pub fn create_cgroup(container_id: &str, _limits: &ResourceLimits) -> CriResult<CgroupHandle> {
    let handle = CgroupHandle::new(container_id);

    #[cfg(target_os = "linux")]
    {
        std::fs::create_dir_all(&handle.path).map_err(|e| {
            CriError::Cgroup(format!("failed to create cgroup {}: {}", handle.path.display(), e))
        })?;
        apply_limits(&handle, limits)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!("cgroups not supported on this OS — resource limits not enforced");
    }

    Ok(handle)
}

/// Update resource limits on an existing cgroup.
pub fn update_cgroup(handle: &CgroupHandle, limits: &ResourceLimits) -> CriResult<()> {
    apply_limits(handle, limits)
}

/// Remove cgroup directory.
pub fn remove_cgroup(_handle: &CgroupHandle) -> CriResult<()> {
    #[cfg(target_os = "linux")]
    {
        if handle.path.exists() {
            std::fs::remove_dir(&handle.path).map_err(|e| {
                CriError::Cgroup(format!("failed to remove cgroup: {}", e))
            })?;
        }
    }
    Ok(())
}

/// Read current resource usage from cgroup.
pub fn read_stats(_handle: &CgroupHandle) -> CriResult<CgroupStats> {
    let stats = CgroupStats::default();

    #[cfg(target_os = "linux")]
    {
        stats.cpu_usage_usec = read_u64(&handle.path.join("cpu.stat"), "usage_usec").unwrap_or(0);
        stats.memory_current = read_file_u64(&handle.path.join("memory.current")).unwrap_or(0);
        stats.memory_peak = read_file_u64(&handle.path.join("memory.peak")).unwrap_or(0);
        stats.pids_current = read_file_u64(&handle.path.join("pids.current")).unwrap_or(0);
    }

    Ok(stats)
}

fn apply_limits(handle: &CgroupHandle, limits: &ResourceLimits) -> CriResult<()> {
    #[cfg(target_os = "linux")]
    {
        if let Some(cpu_shares) = limits.cpu_shares {
            write_file(&handle.path.join("cpu.weight"), &cpu_shares.to_string())?;
        }
        if let Some(cpu_quota) = limits.cpu_quota {
            write_file(&handle.path.join("cpu.max"), &format!("{} 100000", cpu_quota))?;
        }
        if let Some(mem) = limits.memory_limit {
            write_file(&handle.path.join("memory.max"), &mem.to_string())?;
        }
        if let Some(pids) = limits.pids_limit {
            write_file(&handle.path.join("pids.max"), &pids.to_string())?;
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (handle, limits);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_file(path: &Path, content: &str) -> CriResult<()> {
    std::fs::write(path, content).map_err(|e| {
        CriError::Cgroup(format!("write {} failed: {}", path.display(), e))
    })
}

#[cfg(target_os = "linux")]
fn read_file_u64(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn read_u64(path: &Path, key: &str) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix(key) {
            return val.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cgroup_handle_path() {
        let h = CgroupHandle::new("abc123");
        assert!(h.path.to_string_lossy().contains("cave"));
        assert!(h.path.to_string_lossy().contains("abc123"));
    }

    #[test]
    fn test_read_stats_non_linux() {
        let h = CgroupHandle::new("test");
        let stats = read_stats(&h).unwrap();
        assert_eq!(stats.cpu_usage_usec, 0);
    }
}
