// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: line-ported types adapted from
// opencontainers/runtime-spec/specs-go/config.go (Apache-2.0).
//! OCI runtime spec — `runtime-spec/specs-go/config.go` port.
//!
//! Models the JSON `config.json` consumed by all three runtimes
//! (gVisor `runsc`, kata `kata-runtime`, Firecracker via its own boot spec).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level OCI spec — mirrors `Spec` in upstream `config.go`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Spec {
    /// Spec version (e.g. `1.2.0`).
    pub oci_version: String,
    pub process: Option<Process>,
    pub root: Option<Root>,
    pub hostname: Option<String>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    pub linux: Option<Linux>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

/// `Process` block — what to run inside the sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Process {
    pub terminal: bool,
    pub user: User,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    pub cwd: String,
    pub capabilities: Option<LinuxCapabilities>,
    pub no_new_privileges: bool,
    pub apparmor_profile: Option<String>,
    pub selinux_label: Option<String>,
    #[serde(default)]
    pub rlimits: Vec<Rlimit>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    #[serde(default)]
    pub additional_gids: Vec<u32>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Rlimit {
    pub kind: String,
    pub hard: u64,
    pub soft: u64,
}

/// `Root` — rootfs path + readonly flag.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Root {
    pub path: String,
    pub readonly: bool,
}

/// `Mount` — mount entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Mount {
    pub destination: String,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "type", default)]
    pub fs_type: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// `Linux` block — Linux-specific config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Linux {
    #[serde(default)]
    pub namespaces: Vec<LinuxNamespace>,
    #[serde(default)]
    pub uid_mappings: Vec<LinuxIdMapping>,
    #[serde(default)]
    pub gid_mappings: Vec<LinuxIdMapping>,
    pub resources: Option<LinuxResources>,
    pub seccomp: Option<LinuxSeccomp>,
    pub cgroups_path: Option<String>,
    #[serde(default)]
    pub readonly_paths: Vec<String>,
    #[serde(default)]
    pub masked_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxNamespace {
    /// `pid`, `network`, `mount`, `ipc`, `uts`, `user`, `cgroup`.
    #[serde(rename = "type")]
    pub ns_type: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxIdMapping {
    pub container_id: u32,
    pub host_id: u32,
    pub size: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxResources {
    pub cpu: Option<LinuxCpu>,
    pub memory: Option<LinuxMemory>,
    pub pids: Option<LinuxPids>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxCpu {
    pub shares: Option<u64>,
    pub quota: Option<i64>,
    pub period: Option<u64>,
    pub cpus: Option<String>,
    pub mems: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxMemory {
    pub limit: Option<i64>,
    pub reservation: Option<i64>,
    pub swap: Option<i64>,
    pub kernel: Option<i64>,
    pub disable_oom_killer: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxPids {
    pub limit: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxCapabilities {
    #[serde(default)]
    pub bounding: Vec<String>,
    #[serde(default)]
    pub effective: Vec<String>,
    #[serde(default)]
    pub inheritable: Vec<String>,
    #[serde(default)]
    pub permitted: Vec<String>,
    #[serde(default)]
    pub ambient: Vec<String>,
}

/// Seccomp profile — list of syscall rules + default action.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxSeccomp {
    pub default_action: String,
    #[serde(default)]
    pub architectures: Vec<String>,
    #[serde(default)]
    pub syscalls: Vec<LinuxSyscall>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LinuxSyscall {
    pub names: Vec<String>,
    pub action: String,
}

impl Spec {
    /// Minimum bootable spec for a `/bin/sh` container.
    pub fn minimal_shell(rootfs: impl Into<String>) -> Self {
        Spec {
            oci_version: "1.2.0".into(),
            process: Some(Process {
                terminal: false,
                user: User::default(),
                args: vec!["/bin/sh".into()],
                env: vec!["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into()],
                cwd: "/".into(),
                capabilities: Some(LinuxCapabilities::default_minimal()),
                no_new_privileges: true,
                apparmor_profile: None,
                selinux_label: None,
                rlimits: Vec::new(),
            }),
            root: Some(Root { path: rootfs.into(), readonly: true }),
            hostname: Some("cave-sandbox".into()),
            mounts: default_mounts(),
            linux: Some(Linux::default_namespaces()),
            annotations: BTreeMap::new(),
        }
    }

    /// Validate invariants — used by `runsc create` / `kata create`.
    pub fn validate(&self) -> Result<(), String> {
        if self.oci_version.is_empty() {
            return Err("oci_version must be set".into());
        }
        match &self.root {
            None => return Err("root missing".into()),
            Some(r) if r.path.is_empty() => return Err("root.path empty".into()),
            _ => {}
        }
        match &self.process {
            None => return Err("process missing".into()),
            Some(p) if p.args.is_empty() => return Err("process.args empty".into()),
            _ => {}
        }
        Ok(())
    }
}

impl LinuxCapabilities {
    /// Default minimal cap set — matches `runc` defaults.
    pub fn default_minimal() -> Self {
        let caps = vec![
            "CAP_AUDIT_WRITE".into(),
            "CAP_KILL".into(),
            "CAP_NET_BIND_SERVICE".into(),
        ];
        LinuxCapabilities {
            bounding: caps.clone(),
            effective: caps.clone(),
            inheritable: caps.clone(),
            permitted: caps.clone(),
            ambient: caps,
        }
    }
}

impl Linux {
    /// Default namespace set (pid/network/mount/ipc/uts).
    pub fn default_namespaces() -> Self {
        Linux {
            namespaces: vec!["pid", "network", "mount", "ipc", "uts"]
                .into_iter()
                .map(|t| LinuxNamespace { ns_type: t.into(), path: None })
                .collect(),
            ..Linux::default()
        }
    }
}

fn default_mounts() -> Vec<Mount> {
    vec![
        Mount {
            destination: "/proc".into(),
            source: "proc".into(),
            fs_type: "proc".into(),
            options: vec![],
        },
        Mount {
            destination: "/dev".into(),
            source: "tmpfs".into(),
            fs_type: "tmpfs".into(),
            options: vec!["nosuid".into(), "strictatime".into(), "mode=755".into(), "size=65536k".into()],
        },
        Mount {
            destination: "/sys".into(),
            source: "sysfs".into(),
            fs_type: "sysfs".into(),
            options: vec!["nosuid".into(), "noexec".into(), "nodev".into(), "ro".into()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_spec_validates() {
        let s = Spec::minimal_shell("/var/lib/cave/rootfs/1");
        assert!(s.validate().is_ok());
        assert_eq!(s.oci_version, "1.2.0");
        assert_eq!(s.process.as_ref().unwrap().args, vec!["/bin/sh"]);
    }

    #[test]
    fn empty_spec_fails_validation() {
        let s = Spec::default();
        assert!(s.validate().is_err());
    }

    #[test]
    fn root_path_empty_fails() {
        let mut s = Spec::minimal_shell("rootfs");
        s.root.as_mut().unwrap().path.clear();
        assert!(s.validate().is_err());
    }

    #[test]
    fn process_args_empty_fails() {
        let mut s = Spec::minimal_shell("rootfs");
        s.process.as_mut().unwrap().args.clear();
        assert!(s.validate().is_err());
    }

    #[test]
    fn default_namespaces_five() {
        let l = Linux::default_namespaces();
        assert_eq!(l.namespaces.len(), 5);
    }

    #[test]
    fn default_caps_three() {
        let c = LinuxCapabilities::default_minimal();
        assert_eq!(c.bounding.len(), 3);
        assert!(c.bounding.iter().any(|s| s == "CAP_KILL"));
    }

    #[test]
    fn json_roundtrip() {
        let s = Spec::minimal_shell("/rfs");
        let json = serde_json::to_string(&s).unwrap();
        let back: Spec = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn seccomp_parsable() {
        let raw = r#"{"default_action":"SCMP_ACT_ERRNO","architectures":["SCMP_ARCH_X86_64"],"syscalls":[{"names":["read","write"],"action":"SCMP_ACT_ALLOW"}]}"#;
        let sc: LinuxSeccomp = serde_json::from_str(raw).unwrap();
        assert_eq!(sc.default_action, "SCMP_ACT_ERRNO");
        assert_eq!(sc.syscalls[0].names, vec!["read", "write"]);
    }

    #[test]
    fn default_mounts_includes_proc() {
        let m = default_mounts();
        assert!(m.iter().any(|x| x.destination == "/proc"));
    }
}
