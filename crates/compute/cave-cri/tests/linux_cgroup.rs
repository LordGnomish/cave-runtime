// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Linux code paths of `cave_cri::cgroup`, exercised without privileges.
//!
//! `update_cgroup`, `remove_cgroup`, `read_stats` and `read_stats_v2` only do
//! real work under `#[cfg(target_os = "linux")]`. Every test here roots the
//! `CgroupHandle` at a tempdir through the existing `CgroupHandle::with_root`
//! seam, so the functions do plain file I/O: no privileges are needed and the
//! host's `/sys/fs/cgroup` is never touched.
//!
//! What this cannot show is how a real cgroup v2 hierarchy reacts (controller
//! delegation through `cgroup.subtree_control`, value validation by the
//! kernel, `EBUSY` on a populated cgroup). The file names and value formats
//! asserted below are the ones documented in the kernel's
//! `Documentation/admin-guide/cgroup-v2.rst`:
//!
//! - `cpu.weight` — a single integer in `[1, 10000]`;
//! - `cpu.max` — `$MAX $PERIOD`, both in microseconds;
//! - `memory.max`, `pids.max` — a single integer (or `max`);
//! - `cpu.stat`, `pids.events` — flat-keyed `key value` lines;
//! - `io.stat` — nested-keyed `MAJ:MIN key=value ...` lines.

#![cfg(target_os = "linux")]

use cave_cri::cgroup::{read_stats, read_stats_v2, remove_cgroup, update_cgroup, CgroupHandle};
use cave_cri::error::CriError;
use cave_cri::models::ResourceLimits;
use std::fs;

const TENANT: &str = "tenant-linux-paths";

/// A handle whose directory exists, like a cgroup that was created earlier.
fn existing_cgroup(root: &tempfile::TempDir, container_id: &str) -> CgroupHandle {
    let handle = CgroupHandle::with_root(container_id, TENANT, root.path());
    fs::create_dir_all(&handle.path).unwrap();
    handle
}

#[test]
fn update_cgroup_writes_each_limit_to_its_interface_file() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-update");
    let limits = ResourceLimits {
        cpu_shares: Some(512),
        cpu_quota: Some(50_000),
        memory_limit: Some(256 * 1024 * 1024),
        pids_limit: Some(128),
    };

    update_cgroup(&handle, &limits).unwrap();

    assert_eq!(
        fs::read_to_string(handle.path.join("cpu.weight")).unwrap(),
        "512"
    );
    assert_eq!(
        fs::read_to_string(handle.path.join("cpu.max")).unwrap(),
        "50000 100000"
    );
    assert_eq!(
        fs::read_to_string(handle.path.join("memory.max")).unwrap(),
        "268435456"
    );
    assert_eq!(
        fs::read_to_string(handle.path.join("pids.max")).unwrap(),
        "128"
    );
}

#[test]
fn update_cgroup_leaves_unset_limits_untouched() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-partial");

    update_cgroup(&handle, &ResourceLimits::default()).unwrap();
    assert_eq!(
        fs::read_dir(&handle.path).unwrap().count(),
        0,
        "no limit set, so no interface file may be written"
    );

    let only_memory = ResourceLimits {
        memory_limit: Some(4096),
        ..Default::default()
    };
    update_cgroup(&handle, &only_memory).unwrap();
    assert_eq!(
        fs::read_to_string(handle.path.join("memory.max")).unwrap(),
        "4096"
    );
    assert!(!handle.path.join("cpu.weight").exists());
    assert!(!handle.path.join("cpu.max").exists());
    assert!(!handle.path.join("pids.max").exists());
}

/// Limits can only be applied to a cgroup that exists. Updating a missing
/// one must fail and must not create the directory as a side effect: on a
/// real hierarchy `mkdir` is what creates a new cgroup.
#[test]
fn update_cgroup_fails_for_a_cgroup_that_does_not_exist() {
    let root = tempfile::tempdir().unwrap();
    let handle = CgroupHandle::with_root("ctr-missing", TENANT, root.path());
    let limits = ResourceLimits {
        pids_limit: Some(16),
        ..Default::default()
    };

    let err = update_cgroup(&handle, &limits).unwrap_err();

    assert!(
        matches!(err, CriError::Cgroup(_)),
        "expected a cgroup error, got {err:?}"
    );
    assert!(!handle.path.exists());
}

#[test]
fn remove_cgroup_removes_an_empty_cgroup_directory() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-remove");

    remove_cgroup(&handle).unwrap();

    assert!(!handle.path.exists());
    assert!(
        handle.path.parent().unwrap().exists(),
        "only the container's own cgroup is removed, not the tenant level"
    );
}

#[test]
fn remove_cgroup_is_ok_when_the_cgroup_is_already_gone() {
    let root = tempfile::tempdir().unwrap();
    let handle = CgroupHandle::with_root("ctr-never-created", TENANT, root.path());

    remove_cgroup(&handle).unwrap();

    assert!(!handle.path.exists());
}

/// A cgroup is removed with `rmdir(2)`, which the kernel refuses while the
/// cgroup still has child cgroups (or processes). A child directory in the
/// tempdir stands in for that: the error must surface and nothing below the
/// cgroup may be deleted recursively.
#[test]
fn remove_cgroup_reports_a_cgroup_that_still_has_children() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-busy");
    let child = handle.path.join("child");
    fs::create_dir(&child).unwrap();

    let err = remove_cgroup(&handle).unwrap_err();

    assert!(
        matches!(err, CriError::Cgroup(_)),
        "expected a cgroup error, got {err:?}"
    );
    assert!(child.exists());
}

#[test]
fn read_stats_parses_cpu_memory_and_pids_files() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-stats");
    fs::write(
        handle.path.join("cpu.stat"),
        "usage_usec 7000000\nuser_usec 4000000\nsystem_usec 3000000\n\
         nr_periods 100\nnr_throttled 7\nthrottled_usec 50000\n",
    )
    .unwrap();
    fs::write(handle.path.join("memory.current"), "104857600\n").unwrap();
    fs::write(handle.path.join("memory.peak"), "209715200\n").unwrap();
    fs::write(handle.path.join("pids.current"), "37\n").unwrap();

    let stats = read_stats(&handle).unwrap();

    assert_eq!(stats.cpu_usage_usec, 7_000_000);
    assert_eq!(stats.memory_current, 104_857_600);
    assert_eq!(stats.memory_peak, 209_715_200);
    assert_eq!(stats.pids_current, 37);
}

#[test]
fn read_stats_v2_parses_cpu_memory_pids_and_io_files() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-stats-v2");
    fs::write(
        handle.path.join("cpu.stat"),
        "usage_usec 7000000\nuser_usec 4000000\nsystem_usec 3000000\n\
         nr_periods 100\nnr_throttled 7\nthrottled_usec 50000\n",
    )
    .unwrap();
    fs::write(handle.path.join("memory.current"), "104857600\n").unwrap();
    fs::write(handle.path.join("memory.peak"), "209715200\n").unwrap();
    fs::write(handle.path.join("memory.swap.current"), "16384\n").unwrap();
    fs::write(handle.path.join("pids.current"), "37\n").unwrap();
    fs::write(handle.path.join("pids.events"), "max 2\n").unwrap();
    fs::write(
        handle.path.join("io.stat"),
        "8:0 rbytes=1024 wbytes=2048 rios=10 wios=20 dbytes=0 dios=0\n\
         259:0 rbytes=4096 wbytes=8192 rios=2 wios=4 dbytes=0 dios=0\n",
    )
    .unwrap();

    let stats = read_stats_v2(&handle).unwrap();

    assert_eq!(stats.cpu_usage_usec, 7_000_000);
    assert_eq!(stats.cpu_user_usec, 4_000_000);
    assert_eq!(stats.cpu_system_usec, 3_000_000);
    assert_eq!(stats.cpu_nr_throttled, 7);
    assert_eq!(stats.memory_current, 104_857_600);
    assert_eq!(stats.memory_peak, 209_715_200);
    assert_eq!(stats.memory_swap_current, 16_384);
    assert_eq!(stats.pids_current, 37);
    assert_eq!(stats.pids_max_reached, 2);
    assert_eq!(stats.io_read_bytes, 1024 + 4096);
    assert_eq!(stats.io_write_bytes, 2048 + 8192);
}

/// A controller that is not enabled for the cgroup has no interface files.
/// The readers report zero for what they cannot read instead of failing.
#[test]
fn read_stats_reports_zero_for_missing_interface_files() {
    let root = tempfile::tempdir().unwrap();
    let handle = existing_cgroup(&root, "ctr-no-controllers");

    let stats = read_stats(&handle).unwrap();
    assert_eq!(stats.cpu_usage_usec, 0);
    assert_eq!(stats.memory_current, 0);
    assert_eq!(stats.memory_peak, 0);
    assert_eq!(stats.pids_current, 0);

    let stats_v2 = read_stats_v2(&handle).unwrap();
    assert_eq!(stats_v2.cpu_user_usec, 0);
    assert_eq!(stats_v2.memory_swap_current, 0);
    assert_eq!(stats_v2.pids_max_reached, 0);
    assert_eq!(stats_v2.io_read_bytes, 0);
    assert_eq!(stats_v2.io_write_bytes, 0);
}
