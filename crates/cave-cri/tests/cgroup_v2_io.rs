// SPDX-License-Identifier: AGPL-3.0-or-later
//! deeper-002: real cgroup v2 file I/O against an arbitrary root.
//!
//! Upstream: containerd v2.2.3 `pkg/cri/server/container_create_linux.go`
//! (cgroup path layout) + runc v1.4.2 `libcontainer/cgroups/fs2/{cpu,
//! memory,pids,io,freezer}.go` (per-knob writers).

use cave_cri::cgroup::{
    apply_limits_in, attach_pid, create_cgroup_in, read_stats_in, remove_cgroup_in,
    set_freezer, CgroupHandle, FreezerState,
};
use cave_cri::models::ResourceLimits;
use std::fs;

const TENANT: &str = "tenant-acme-prod";

/// Cite: containerd v2.2.3 cgroup layout — path is
/// `<root>/cave/<tenant_id>/<container_id>`. Tenant segment hard-isolates
/// two tenants holding identically-named containers from sharing limits.
#[test]
fn handle_path_includes_tenant_and_container_segments() {
    let root = tempfile::tempdir().unwrap();
    let h = CgroupHandle::with_root("ctr-001", TENANT, root.path());
    let s = h.path.to_string_lossy();
    assert!(s.contains("/cave/"));
    assert!(s.contains(TENANT));
    assert!(s.ends_with("/ctr-001"));
    assert_eq!(h.tenant_id, TENANT);
    assert_eq!(h.container_id, "ctr-001");
}

/// Cite: runc v1.4.2 `libcontainer/cgroups/fs2/cpu.go::setCpuWeight`,
/// `setCpuMax`, `fs2/memory.go::setMemory`, `fs2/pids.go::setPids` —
/// each non-None limit results in a real write to the corresponding
/// cgroup file. Bytes written must match the upstream exact format.
#[test]
fn apply_limits_writes_real_bytes_to_canonical_files() {
    let root = tempfile::tempdir().unwrap();
    let limits = ResourceLimits {
        cpu_shares:   Some(512),
        cpu_quota:    Some(50_000),
        memory_limit: Some(256 * 1024 * 1024),
        pids_limit:   Some(128),
    };
    let h = create_cgroup_in("ctr-002", TENANT, root.path(), &limits).unwrap();

    // Files are real, contents match the runc fs2 writers exactly.
    assert_eq!(fs::read_to_string(h.path.join("cpu.weight")).unwrap(), "512");
    assert_eq!(fs::read_to_string(h.path.join("cpu.max")).unwrap(), "50000 100000");
    assert_eq!(fs::read_to_string(h.path.join("memory.max")).unwrap(),
        (256 * 1024 * 1024).to_string());
    assert_eq!(fs::read_to_string(h.path.join("pids.max")).unwrap(), "128");
}

/// Cite: runc v1.4.2 fs2 — None-valued limits MUST NOT touch the
/// corresponding cgroup file (so the kernel default stays in force).
#[test]
fn apply_limits_only_writes_set_knobs() {
    let root = tempfile::tempdir().unwrap();
    let h = create_cgroup_in("ctr-003", TENANT, root.path(), &ResourceLimits::default()).unwrap();
    assert!(!h.path.join("cpu.weight").exists());
    assert!(!h.path.join("cpu.max").exists());
    assert!(!h.path.join("memory.max").exists());
    assert!(!h.path.join("pids.max").exists());

    // Subsequent partial update only touches the named knob.
    apply_limits_in(&h, &ResourceLimits {
        cpu_shares: Some(1024),
        ..Default::default()
    }).unwrap();
    assert_eq!(fs::read_to_string(h.path.join("cpu.weight")).unwrap(), "1024");
    assert!(!h.path.join("memory.max").exists());
}

/// Cite: runc v1.4.2 `libcontainer/cgroups/fs2/fs2.go::Apply` — moving a
/// process into the cgroup is a single write to `cgroup.procs`.
#[test]
fn attach_pid_writes_to_cgroup_procs() {
    let root = tempfile::tempdir().unwrap();
    let h = create_cgroup_in("ctr-004", TENANT, root.path(), &ResourceLimits::default()).unwrap();
    attach_pid(&h, 4242).unwrap();
    assert_eq!(fs::read_to_string(h.path.join("cgroup.procs")).unwrap(), "4242");
}

/// Cite: runc v1.4.2 `libcontainer/cgroups/fs2/freezer.go` — freeze /
/// thaw flips `cgroup.freeze` between "1" and "0".
#[test]
fn freezer_state_toggles_cgroup_freeze_file() {
    let root = tempfile::tempdir().unwrap();
    let h = create_cgroup_in("ctr-005", TENANT, root.path(), &ResourceLimits::default()).unwrap();

    set_freezer(&h, FreezerState::Frozen).unwrap();
    assert_eq!(fs::read_to_string(h.path.join("cgroup.freeze")).unwrap(), "1");

    set_freezer(&h, FreezerState::Thawed).unwrap();
    assert_eq!(fs::read_to_string(h.path.join("cgroup.freeze")).unwrap(), "0");
}

/// Cite: containerd v2.2.3 `pkg/cri/server/container_stats_list_linux.go`
/// — stats parser pulls `cpu.stat`, `memory.{current,peak,swap.current}`,
/// `pids.{current,events}`, and `io.stat` (rbytes / wbytes summed across
/// all devices). cave's `read_stats_in` mirrors that parser exactly.
#[test]
fn read_stats_parses_real_kernel_format_files() {
    let root = tempfile::tempdir().unwrap();
    let h = create_cgroup_in("ctr-006", TENANT, root.path(), &ResourceLimits::default()).unwrap();

    fs::write(h.path.join("cpu.stat"), "\
usage_usec 7000000
user_usec  4000000
system_usec 3000000
nr_periods 100
nr_throttled 7
throttled_usec 50000
").unwrap();
    fs::write(h.path.join("memory.current"),       "104857600").unwrap();
    fs::write(h.path.join("memory.peak"),          "209715200").unwrap();
    fs::write(h.path.join("memory.swap.current"),  "16384").unwrap();
    fs::write(h.path.join("pids.current"),         "37").unwrap();
    fs::write(h.path.join("pids.events"),
        "max 2\nmax.imposed 0\n").unwrap();
    fs::write(h.path.join("io.stat"), "\
8:0 rbytes=1024 wbytes=2048 rios=10 wios=20
259:0 rbytes=4096 wbytes=8192 rios=2 wios=4
").unwrap();

    let s = read_stats_in(&h).unwrap();
    assert_eq!(s.cpu_usage_usec,    7_000_000);
    assert_eq!(s.cpu_user_usec,     4_000_000);
    assert_eq!(s.cpu_system_usec,   3_000_000);
    assert_eq!(s.cpu_nr_throttled,  7);
    assert_eq!(s.memory_current,    104_857_600);
    assert_eq!(s.memory_peak,       209_715_200);
    assert_eq!(s.memory_swap_current, 16_384);
    assert_eq!(s.pids_current,      37);
    assert_eq!(s.pids_max_reached,  2);
    assert_eq!(s.io_read_bytes,     1024 + 4096);
    assert_eq!(s.io_write_bytes,    2048 + 8192);

    // Cleanup deletes the directory and everything in it.
    remove_cgroup_in(&h).unwrap();
    assert!(!h.path.exists());
}
