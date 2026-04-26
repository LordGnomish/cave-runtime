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
        stats.cpu_usage_usec = read_cpu_stat(&handle.path.join("cpu.stat"), "usage_usec").unwrap_or(0);
        stats.memory_current = read_file_u64(&handle.path.join("memory.current")).unwrap_or(0);
        stats.memory_peak = read_file_u64(&handle.path.join("memory.peak")).unwrap_or(0);
        stats.pids_current = read_file_u64(&handle.path.join("pids.current")).unwrap_or(0);
    }

    Ok(stats)
}

/// Read extended cgroup v2 stats including io.stat, user/sys usec, and throttle info.
pub fn read_stats_v2(_handle: &CgroupHandle) -> CriResult<crate::models::CgroupStatsV2> {
    let stats = crate::models::CgroupStatsV2::default();

    #[cfg(target_os = "linux")]
    {
        let cpu_stat_path = handle.path.join("cpu.stat");
        stats.cpu_usage_usec  = read_cpu_stat(&cpu_stat_path, "usage_usec").unwrap_or(0);
        stats.cpu_user_usec   = read_cpu_stat(&cpu_stat_path, "user_usec").unwrap_or(0);
        stats.cpu_system_usec = read_cpu_stat(&cpu_stat_path, "system_usec").unwrap_or(0);
        stats.cpu_nr_throttled = read_cpu_stat(&cpu_stat_path, "nr_throttled").unwrap_or(0);

        stats.memory_current      = read_file_u64(&handle.path.join("memory.current")).unwrap_or(0);
        stats.memory_peak         = read_file_u64(&handle.path.join("memory.peak")).unwrap_or(0);
        stats.memory_swap_current = read_file_u64(&handle.path.join("memory.swap.current")).unwrap_or(0);

        stats.pids_current    = read_file_u64(&handle.path.join("pids.current")).unwrap_or(0);
        stats.pids_max_reached = read_pids_events(&handle.path.join("pids.events")).unwrap_or(0);

        let (rbytes, wbytes) = read_io_stat(&handle.path.join("io.stat")).unwrap_or((0, 0));
        stats.io_read_bytes  = rbytes;
        stats.io_write_bytes = wbytes;
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
fn write_file(path: &std::path::Path, content: &str) -> CriResult<()> {
    std::fs::write(path, content).map_err(|e| {
        crate::error::CriError::Cgroup(format!("write {} failed: {}", path.display(), e))
    })
}

#[cfg(target_os = "linux")]
fn read_file_u64(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Read a `key value` line from cpu.stat.
#[cfg(target_os = "linux")]
fn read_cpu_stat(path: &std::path::Path, key: &str) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let mut parts = line.splitn(2, ' ');
        if parts.next()? == key {
            return parts.next()?.trim().parse().ok();
        }
    }
    None
}

/// Parse pids.events for "max" (number of times pids.max was hit).
#[cfg(target_os = "linux")]
fn read_pids_events(path: &std::path::Path) -> Option<u64> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let mut parts = line.splitn(2, ' ');
        if parts.next()? == "max" {
            return parts.next()?.trim().parse().ok();
        }
    }
    None
}

/// Parse io.stat — sum rbytes and wbytes across all devices.
/// Format: `8:0 rbytes=... wbytes=... rios=... wios=... dbytes=... dios=...`
#[cfg(target_os = "linux")]
fn read_io_stat(path: &std::path::Path) -> Option<(u64, u64)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut total_rbytes = 0u64;
    let mut total_wbytes = 0u64;
    for line in content.lines() {
        // Skip device column
        let fields = line.split_whitespace().skip(1);
        for field in fields {
            if let Some(v) = field.strip_prefix("rbytes=") {
                total_rbytes += v.parse::<u64>().unwrap_or(0);
            } else if let Some(v) = field.strip_prefix("wbytes=") {
                total_wbytes += v.parse::<u64>().unwrap_or(0);
            }
        }
    }
    Some((total_rbytes, total_wbytes))
}

// Non-Linux stubs — defined so that #[cfg(target_os = "linux")] blocks inside
// read_stats_v2 still reference real names at the call site on Linux.
// On macOS/Windows these are never called, so allow dead_code.
#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn read_cpu_stat(_path: &std::path::Path, _key: &str) -> Option<u64> { None }

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn read_file_u64(_path: &std::path::Path) -> Option<u64> { None }

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn read_pids_events(_path: &std::path::Path) -> Option<u64> { None }

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
fn read_io_stat(_path: &std::path::Path) -> Option<(u64, u64)> { None }

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

    #[test]
    fn test_cgroup_handle_path_with_hyphens() {
        let id = "abc-def-123-456";
        let h = CgroupHandle::new(id);
        assert!(h.path.to_string_lossy().contains("abc-def-123-456"));
        assert_eq!(h.container_id, id);
    }

    #[test]
    fn test_cgroup_handle_path_structure() {
        let h = CgroupHandle::new("mycontainer");
        let path = h.path.to_string_lossy();
        assert!(path.starts_with("/sys/fs/cgroup"));
        assert!(path.contains("cave"));
        assert!(path.ends_with("mycontainer"));
    }

    #[test]
    fn test_cgroup_handle_empty_id() {
        let h = CgroupHandle::new("");
        // Empty id should still produce a valid path under cave/
        assert!(h.path.to_string_lossy().contains("cave"));
        assert_eq!(h.container_id, "");
    }

    #[test]
    fn test_create_cgroup_returns_handle() {
        let limits = ResourceLimits::default();
        let h = create_cgroup("create-test-id", &limits).unwrap();
        assert!(h.path.to_string_lossy().contains("create-test-id"));
    }

    #[test]
    fn test_create_cgroup_zero_limits() {
        let limits = ResourceLimits {
            cpu_shares: Some(0),
            cpu_quota: Some(0),
            memory_limit: Some(0),
            pids_limit: Some(0),
        };
        let h = create_cgroup("zero-limits-id", &limits).unwrap();
        assert_eq!(h.container_id, "zero-limits-id");
    }

    #[test]
    fn test_update_cgroup_default_limits() {
        let h = CgroupHandle::new("update-test");
        let limits = ResourceLimits::default();
        assert!(update_cgroup(&h, &limits).is_ok());
    }

    #[test]
    fn test_update_cgroup_all_limits_set() {
        let h = CgroupHandle::new("update-all");
        let limits = ResourceLimits {
            cpu_shares: Some(1024),
            cpu_quota: Some(50000),
            memory_limit: Some(512 * 1024 * 1024),
            pids_limit: Some(100),
        };
        assert!(update_cgroup(&h, &limits).is_ok());
    }

    #[test]
    fn test_remove_cgroup_nonexistent_path() {
        let h = CgroupHandle::new("nonexistent-xyz-999");
        // Should succeed because on non-linux it's a no-op
        // and on linux it checks path.exists() first
        assert!(remove_cgroup(&h).is_ok());
    }

    #[test]
    fn test_read_stats_all_fields_zero_on_non_linux() {
        let h = CgroupHandle::new("stats-zero-test");
        let stats = read_stats(&h).unwrap();
        assert_eq!(stats.cpu_usage_usec, 0);
        assert_eq!(stats.memory_current, 0);
        assert_eq!(stats.memory_peak, 0);
        assert_eq!(stats.pids_current, 0);
    }

    // ── v2 stats ──────────────────────────────────────────────────────────────

    #[test]
    fn read_stats_v2_all_zero_on_non_linux() {
        let h = CgroupHandle::new("v2-zero-test");
        let stats = read_stats_v2(&h).unwrap();
        assert_eq!(stats.cpu_usage_usec, 0);
        assert_eq!(stats.cpu_user_usec, 0);
        assert_eq!(stats.cpu_system_usec, 0);
        assert_eq!(stats.cpu_nr_throttled, 0);
        assert_eq!(stats.memory_current, 0);
        assert_eq!(stats.memory_peak, 0);
        assert_eq!(stats.memory_swap_current, 0);
        assert_eq!(stats.pids_current, 0);
        assert_eq!(stats.pids_max_reached, 0);
        assert_eq!(stats.io_read_bytes, 0);
        assert_eq!(stats.io_write_bytes, 0);
    }

    #[test]
    fn read_stats_v2_returns_ok() {
        let h = CgroupHandle::new("v2-ok-test");
        assert!(read_stats_v2(&h).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_cpu_stat_from_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cpu.stat");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "usage_usec 123456").unwrap();
        writeln!(f, "user_usec 80000").unwrap();
        writeln!(f, "system_usec 43456").unwrap();
        writeln!(f, "nr_periods 100").unwrap();
        writeln!(f, "nr_throttled 5").unwrap();
        assert_eq!(read_cpu_stat(&path, "usage_usec"), Some(123456));
        assert_eq!(read_cpu_stat(&path, "user_usec"), Some(80000));
        assert_eq!(read_cpu_stat(&path, "nr_throttled"), Some(5));
        assert_eq!(read_cpu_stat(&path, "missing_key"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_io_stat_from_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("io.stat");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "8:0 rbytes=1024 wbytes=2048 rios=10 wios=20 dbytes=0 dios=0").unwrap();
        writeln!(f, "8:16 rbytes=512 wbytes=256 rios=5 wios=3 dbytes=0 dios=0").unwrap();
        let (r, w) = read_io_stat(&path).unwrap();
        assert_eq!(r, 1536);  // 1024 + 512
        assert_eq!(w, 2304);  // 2048 + 256
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_pids_events_from_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pids.events");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "max 3").unwrap();
        assert_eq!(read_pids_events(&path), Some(3));
    }
}
