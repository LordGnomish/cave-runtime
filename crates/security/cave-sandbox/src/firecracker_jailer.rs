// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: jailer params adapted from firecracker-microvm/firecracker
// src/jailer/src/main.rs (Apache-2.0).
//! Firecracker `jailer` — chroot wrapper around the VMM.
//!
//! Models the jailer CLI flags. The actual `clone()` of namespaces +
//! `setns` + cgroups setup is OUT OF SCOPE (kernel-FFI policy).

use serde::{Deserialize, Serialize};

/// `jailer` CLI parameters — `src/jailer/src/main.rs::Cli`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JailerParams {
    /// Required UUID identifying the VM (`--id`).
    pub id: String,
    /// User to drop privs to (`--uid`).
    pub uid: u32,
    /// Group (`--gid`).
    pub gid: u32,
    /// Absolute path to the firecracker binary (`--exec-file`).
    pub exec_file: String,
    /// Where to mount the new root (`--chroot-base-dir`).
    pub chroot_base_dir: String,
    /// Daemonize (`--daemonize`).
    pub daemonize: bool,
    /// Numa node binding (`--node`).
    pub numa_node: Option<u32>,
    /// Cgroup v2 setup base path (`--cgroup-version`).
    pub cgroup_version: u8,
    /// Per-cgroup limits — `--cgroup <ctrl>.<key>=<val>`.
    pub cgroups: Vec<CgroupLimit>,
    /// Filesystem resource limit (`--resource-limit`).
    pub resource_limits: Vec<ResourceLimit>,
    /// Wait for child to exit before exiting (`--parent-cgroup`).
    pub parent_cgroup: Option<String>,
    /// New PID namespace flag (`--new-pid-ns`).
    pub new_pid_ns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CgroupLimit {
    pub controller: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceLimit {
    /// `no-file`, `fsize`.
    pub kind: String,
    pub value: u64,
}

impl JailerParams {
    pub fn new(id: impl Into<String>, exec_file: impl Into<String>) -> Self {
        JailerParams {
            id: id.into(),
            uid: 1234,
            gid: 1234,
            exec_file: exec_file.into(),
            chroot_base_dir: "/srv/jailer".into(),
            daemonize: true,
            numa_node: None,
            cgroup_version: 2,
            cgroups: Vec::new(),
            resource_limits: Vec::new(),
            parent_cgroup: None,
            new_pid_ns: true,
        }
    }

    /// Render argv for `exec()` — used for diagnostic CLI surface.
    pub fn to_argv(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![
            "--id".into(), self.id.clone(),
            "--exec-file".into(), self.exec_file.clone(),
            "--uid".into(), self.uid.to_string(),
            "--gid".into(), self.gid.to_string(),
            "--chroot-base-dir".into(), self.chroot_base_dir.clone(),
            "--cgroup-version".into(), self.cgroup_version.to_string(),
        ];
        if self.daemonize { v.push("--daemonize".into()); }
        if self.new_pid_ns { v.push("--new-pid-ns".into()); }
        if let Some(n) = self.numa_node { v.push("--node".into()); v.push(n.to_string()); }
        if let Some(p) = &self.parent_cgroup { v.push("--parent-cgroup".into()); v.push(p.clone()); }
        for c in &self.cgroups {
            v.push("--cgroup".into());
            v.push(format!("{}.{}={}", c.controller, c.key, c.value));
        }
        for r in &self.resource_limits {
            v.push("--resource-limit".into());
            v.push(format!("{}={}", r.kind, r.value));
        }
        v
    }

    /// Path where the chroot root lives — `{base}/{exec_basename}/{id}/root`.
    pub fn chroot_root(&self) -> String {
        let basename = std::path::Path::new(&self.exec_file)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("firecracker");
        format!("{}/{}/{}/root", self.chroot_base_dir, basename, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_has_required_flags() {
        let p = JailerParams::new("vm-1", "/usr/bin/firecracker");
        let argv = p.to_argv();
        assert!(argv.iter().any(|a| a == "--id"));
        assert!(argv.iter().any(|a| a == "--exec-file"));
        assert!(argv.iter().any(|a| a == "--uid"));
        assert!(argv.iter().any(|a| a == "--gid"));
        assert!(argv.iter().any(|a| a == "--daemonize"));
        assert!(argv.iter().any(|a| a == "--new-pid-ns"));
    }

    #[test]
    fn chroot_root_layout() {
        let p = JailerParams::new("vm-1", "/usr/bin/firecracker");
        assert_eq!(p.chroot_root(), "/srv/jailer/firecracker/vm-1/root");
    }

    #[test]
    fn cgroup_limit_flag_encoding() {
        let mut p = JailerParams::new("v", "/x/fc");
        p.cgroups.push(CgroupLimit { controller: "cpu".into(), key: "max".into(), value: "200000 100000".into() });
        let argv = p.to_argv();
        let idx = argv.iter().position(|a| a == "--cgroup").unwrap();
        assert_eq!(argv[idx + 1], "cpu.max=200000 100000");
    }

    #[test]
    fn resource_limit_encoded() {
        let mut p = JailerParams::new("v", "/x/fc");
        p.resource_limits.push(ResourceLimit { kind: "no-file".into(), value: 1024 });
        let argv = p.to_argv();
        let idx = argv.iter().position(|a| a == "--resource-limit").unwrap();
        assert_eq!(argv[idx + 1], "no-file=1024");
    }

    #[test]
    fn numa_node_flag() {
        let mut p = JailerParams::new("v", "/x/fc");
        p.numa_node = Some(2);
        let argv = p.to_argv();
        let idx = argv.iter().position(|a| a == "--node").unwrap();
        assert_eq!(argv[idx + 1], "2");
    }

    #[test]
    fn defaults_use_non_root() {
        let p = JailerParams::new("v", "/x/fc");
        assert_ne!(p.uid, 0);
        assert_ne!(p.gid, 0);
        assert_eq!(p.cgroup_version, 2);
    }

    #[test]
    fn parent_cgroup_optional() {
        let mut p = JailerParams::new("v", "/x/fc");
        p.parent_cgroup = Some("cave-sandbox".into());
        let argv = p.to_argv();
        let idx = argv.iter().position(|a| a == "--parent-cgroup").unwrap();
        assert_eq!(argv[idx + 1], "cave-sandbox");
    }
}
