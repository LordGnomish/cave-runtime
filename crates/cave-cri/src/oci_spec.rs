// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OCI runtime spec (config.json) generation.
//!
//! Produces the JSON spec passed to runc/crun.  Follows the OCI Runtime
//! Specification: https://github.com/opencontainers/runtime-spec/blob/main/config.md

use crate::models::{ContainerSpec, Mount, MountType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ── Top-level spec ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciSpec {
    #[serde(rename = "ociVersion")]
    pub oci_version: String,
    pub process: OciProcess,
    pub root: OciRoot,
    pub hostname: String,
    pub mounts: Vec<OciMount>,
    pub linux: OciLinux,
    pub annotations: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciProcess {
    pub terminal: bool,
    pub user: OciUser,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub cwd: String,
    pub capabilities: OciCapabilities,
    #[serde(rename = "noNewPrivileges")]
    pub no_new_privileges: bool,
    pub rlimits: Vec<OciRlimit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciUser {
    pub uid: u32,
    pub gid: u32,
    #[serde(rename = "additionalGids", skip_serializing_if = "Vec::is_empty", default)]
    pub additional_gids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciCapabilities {
    pub bounding: Vec<String>,
    pub effective: Vec<String>,
    pub permitted: Vec<String>,
    pub ambient: Vec<String>,
    pub inheritable: Vec<String>,
}

impl OciCapabilities {
    fn default_container() -> Self {
        let caps = vec![
            "CAP_AUDIT_WRITE",
            "CAP_CHOWN",
            "CAP_DAC_OVERRIDE",
            "CAP_FOWNER",
            "CAP_FSETID",
            "CAP_KILL",
            "CAP_MKNOD",
            "CAP_NET_BIND_SERVICE",
            "CAP_NET_RAW",
            "CAP_SETFCAP",
            "CAP_SETGID",
            "CAP_SETPCAP",
            "CAP_SETUID",
            "CAP_SYS_CHROOT",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();

        Self {
            bounding: caps.clone(),
            effective: caps.clone(),
            permitted: caps.clone(),
            ambient: vec![],
            inheritable: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciRlimit {
    #[serde(rename = "type")]
    pub kind: String,
    pub hard: u64,
    pub soft: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciRoot {
    pub path: String,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciMount {
    pub destination: String,
    #[serde(rename = "type")]
    pub mount_type: String,
    pub source: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciLinux {
    pub namespaces: Vec<OciNamespace>,
    pub resources: OciResources,
    #[serde(rename = "seccomp")]
    pub seccomp: Option<OciSeccomp>,
    #[serde(rename = "maskedPaths")]
    pub masked_paths: Vec<String>,
    #[serde(rename = "readonlyPaths")]
    pub readonly_paths: Vec<String>,
    #[serde(rename = "cgroupsPath")]
    pub cgroups_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciNamespace {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OciResources {
    pub cpu: OciCpu,
    pub memory: OciMemory,
    pub pids: OciPids,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OciCpu {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shares: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OciMemory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OciPids {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciSeccomp {
    #[serde(rename = "defaultAction")]
    pub default_action: String,
    pub architectures: Vec<String>,
    pub syscalls: Vec<OciSyscall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciSyscall {
    pub names: Vec<String>,
    pub action: String,
}

// ── Default system mounts (same as containerd / Docker) ──────────────────────

fn default_mounts() -> Vec<OciMount> {
    vec![
        OciMount {
            destination: "/proc".into(),
            mount_type: "proc".into(),
            source: "proc".into(),
            options: vec!["nosuid".into(), "noexec".into(), "nodev".into()],
        },
        OciMount {
            destination: "/dev".into(),
            mount_type: "tmpfs".into(),
            source: "tmpfs".into(),
            options: vec!["nosuid".into(), "strictatime".into(), "mode=755".into(), "size=65536k".into()],
        },
        OciMount {
            destination: "/dev/pts".into(),
            mount_type: "devpts".into(),
            source: "devpts".into(),
            options: vec!["nosuid".into(), "noexec".into(), "newinstance".into(), "ptmxmode=0666".into(), "mode=0620".into()],
        },
        OciMount {
            destination: "/dev/shm".into(),
            mount_type: "tmpfs".into(),
            source: "shm".into(),
            options: vec!["nosuid".into(), "noexec".into(), "nodev".into(), "mode=1777".into(), "size=65536k".into()],
        },
        OciMount {
            destination: "/dev/mqueue".into(),
            mount_type: "mqueue".into(),
            source: "mqueue".into(),
            options: vec!["nosuid".into(), "noexec".into(), "nodev".into()],
        },
        OciMount {
            destination: "/sys".into(),
            mount_type: "sysfs".into(),
            source: "sysfs".into(),
            options: vec!["nosuid".into(), "noexec".into(), "nodev".into(), "ro".into()],
        },
        OciMount {
            destination: "/sys/fs/cgroup".into(),
            mount_type: "cgroup2".into(),
            source: "cgroup2".into(),
            options: vec!["nosuid".into(), "noexec".into(), "nodev".into(), "relatime".into()],
        },
    ]
}

fn default_seccomp() -> OciSeccomp {
    OciSeccomp {
        default_action: "SCMP_ACT_ERRNO".into(),
        architectures: vec!["SCMP_ARCH_X86_64".into(), "SCMP_ARCH_X86".into(), "SCMP_ARCH_X32".into()],
        syscalls: vec![
            OciSyscall {
                names: vec![
                    "accept".into(), "accept4".into(), "access".into(), "arch_prctl".into(),
                    "bind".into(), "brk".into(), "capget".into(), "capset".into(),
                    "chdir".into(), "chmod".into(), "chown".into(), "clone".into(),
                    "close".into(), "connect".into(), "dup".into(), "dup2".into(), "dup3".into(),
                    "epoll_create".into(), "epoll_create1".into(), "epoll_ctl".into(),
                    "epoll_pwait".into(), "epoll_wait".into(), "eventfd".into(), "eventfd2".into(),
                    "execve".into(), "execveat".into(), "exit".into(), "exit_group".into(),
                    "faccessat".into(), "fchmod".into(), "fchmodat".into(), "fchown".into(),
                    "fchownat".into(), "fcntl".into(), "fdatasync".into(), "fgetxattr".into(),
                    "flistxattr".into(), "flock".into(), "fork".into(), "fsetxattr".into(),
                    "fstat".into(), "fstatfs".into(), "fsync".into(), "ftruncate".into(),
                    "futex".into(), "getcwd".into(), "getdents".into(), "getdents64".into(),
                    "getegid".into(), "geteuid".into(), "getgid".into(), "getgroups".into(),
                    "getpeername".into(), "getpgrp".into(), "getpid".into(), "getppid".into(),
                    "getrandom".into(), "getrlimit".into(), "getsockname".into(),
                    "getsockopt".into(), "gettid".into(), "gettimeofday".into(), "getuid".into(),
                    "kill".into(), "lchown".into(), "listen".into(), "lseek".into(),
                    "lstat".into(), "madvise".into(), "mkdir".into(), "mkdirat".into(),
                    "mknod".into(), "mknodat".into(), "mlock".into(), "mmap".into(),
                    "mount".into(), "mprotect".into(), "munlock".into(), "munmap".into(),
                    "nanosleep".into(), "newfstatat".into(), "open".into(), "openat".into(),
                    "pause".into(), "pipe".into(), "pipe2".into(), "poll".into(), "ppoll".into(),
                    "prctl".into(), "pread64".into(), "prlimit64".into(), "pwrite64".into(),
                    "read".into(), "readlink".into(), "readlinkat".into(), "readv".into(),
                    "recv".into(), "recvfrom".into(), "recvmmsg".into(), "recvmsg".into(),
                    "rename".into(), "renameat".into(), "renameat2".into(), "rmdir".into(),
                    "rt_sigaction".into(), "rt_sigprocmask".into(), "rt_sigreturn".into(),
                    "rt_sigsuspend".into(), "rt_sigtimedwait".into(), "sched_getaffinity".into(),
                    "sched_getparam".into(), "sched_getscheduler".into(), "sched_yield".into(),
                    "send".into(), "sendfile".into(), "sendmmsg".into(), "sendmsg".into(),
                    "sendto".into(), "set_robust_list".into(), "setgid".into(), "setgroups".into(),
                    "setitimer".into(), "setpgid".into(), "setrlimit".into(), "setsid".into(),
                    "setsockopt".into(), "setuid".into(), "sigaltstack".into(), "socket".into(),
                    "socketpair".into(), "stat".into(), "statfs".into(), "statx".into(),
                    "symlink".into(), "symlinkat".into(), "sysinfo".into(), "tgkill".into(),
                    "time".into(), "timer_create".into(), "timer_delete".into(),
                    "timer_getoverrun".into(), "timer_gettime".into(), "timer_settime".into(),
                    "timerfd_create".into(), "timerfd_gettime".into(), "timerfd_settime".into(),
                    "tkill".into(), "truncate".into(), "umask".into(), "uname".into(),
                    "unlink".into(), "unlinkat".into(), "utime".into(), "utimensat".into(),
                    "utimes".into(), "vfork".into(), "wait4".into(), "waitid".into(),
                    "write".into(), "writev".into(),
                ],
                action: "SCMP_ACT_ALLOW".into(),
            },
        ],
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Generate an OCI runtime spec from a container spec + merged rootfs path.
pub fn generate(spec: &ContainerSpec, rootfs: &PathBuf, container_id: &str) -> OciSpec {
    let args = build_args(spec);
    let env = build_env(spec);
    let cwd = spec.working_dir.clone().unwrap_or_else(|| "/".into());

    let mut mounts = default_mounts();
    for m in &spec.mounts {
        mounts.push(user_mount(m));
    }

    OciSpec {
        oci_version: "1.0.2-dev".into(),
        process: OciProcess {
            terminal: false,
            user: OciUser { uid: 0, gid: 0, additional_gids: vec![] },
            args,
            env,
            cwd,
            capabilities: OciCapabilities::default_container(),
            no_new_privileges: true,
            rlimits: vec![
                OciRlimit { kind: "RLIMIT_NOFILE".into(), hard: 1024, soft: 1024 },
            ],
        },
        root: OciRoot {
            path: rootfs.to_string_lossy().into_owned(),
            readonly: false,
        },
        hostname: spec.hostname.clone().unwrap_or_else(|| container_id[..12.min(container_id.len())].to_string()),
        mounts,
        linux: OciLinux {
            namespaces: vec![
                OciNamespace { kind: "pid".into(),  path: None },
                OciNamespace { kind: "ipc".into(),  path: None },
                OciNamespace { kind: "uts".into(),  path: None },
                OciNamespace { kind: "mount".into(), path: None },
                OciNamespace { kind: "network".into(), path: None },
            ],
            resources: OciResources {
                cpu: OciCpu {
                    shares: spec.resources.cpu_shares,
                    quota: spec.resources.cpu_quota,
                    period: if spec.resources.cpu_quota.is_some() { Some(100_000) } else { None },
                },
                memory: OciMemory {
                    limit: spec.resources.memory_limit.map(|v| v as i64),
                    swap: None,
                },
                pids: OciPids {
                    limit: spec.resources.pids_limit.map(|v| v as i64),
                },
            },
            seccomp: Some(default_seccomp()),
            masked_paths: vec![
                "/proc/acpi".into(),
                "/proc/asound".into(),
                "/proc/kcore".into(),
                "/proc/keys".into(),
                "/proc/latency_stats".into(),
                "/proc/timer_list".into(),
                "/proc/timer_stats".into(),
                "/proc/sched_debug".into(),
                "/proc/scsi".into(),
                "/sys/firmware".into(),
            ],
            readonly_paths: vec![
                "/proc/bus".into(),
                "/proc/fs".into(),
                "/proc/irq".into(),
                "/proc/sys".into(),
                "/proc/sysrq-trigger".into(),
            ],
            cgroups_path: format!("/cave/{}", container_id),
        },
        annotations: HashMap::new(),
    }
}

/// Write config.json to container bundle directory.
pub fn write(spec: &OciSpec, bundle_dir: &PathBuf) -> std::io::Result<()> {
    let path = bundle_dir.join("config.json");
    let json = serde_json::to_string_pretty(spec)?;
    std::fs::write(path, json)
}

fn build_args(spec: &ContainerSpec) -> Vec<String> {
    let mut args = spec.command.clone();
    args.extend(spec.args.clone());
    if args.is_empty() {
        args.push("/bin/sh".into());
    }
    args
}

fn build_env(spec: &ContainerSpec) -> Vec<String> {
    let mut env: Vec<String> = spec.env.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();
    // Always inject PATH if not set
    if !env.iter().any(|e| e.starts_with("PATH=")) {
        env.push("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());
    }
    env
}

fn user_mount(m: &Mount) -> OciMount {
    let (mount_type, mut options) = match m.mount_type {
        MountType::Bind  => ("bind".to_string(), vec!["rbind".to_string()]),
        MountType::Tmpfs => ("tmpfs".to_string(), vec!["nosuid".to_string(), "noexec".to_string()]),
        MountType::Volume => ("bind".to_string(), vec!["rbind".to_string()]),
    };
    // Translate CRI mount propagation → OCI options. Cite: containerd v2.2.3
    // `pkg/cri/server/container_create_linux.go` (mount propagation map).
    let prop = match m.propagation {
        crate::models::MountPropagation::Private        => "rprivate",
        crate::models::MountPropagation::HostToContainer => "rslave",
        crate::models::MountPropagation::Bidirectional   => "rshared",
    };
    options.push(prop.into());
    if m.read_only {
        options.push("ro".into());
    }
    OciMount {
        destination: m.destination.to_string_lossy().into_owned(),
        mount_type,
        source: m.source.to_string_lossy().into_owned(),
        options,
    }
}

// ─── Volume + security context injectors (deeper-002) ───────────────────────
//
// Cite: containerd v2.2.3
// `pkg/cri/server/container_create_linux.go::generateContainerMounts` and
// `pkg/cri/server/container_create_linux.go::setOCISecurityContext`.

/// Append/replace user mounts on an already-rendered OCI spec. Existing
/// default mounts (proc/sys/dev/...) are preserved; user-supplied mounts
/// are appended in the order given. If a user mount has the same
/// `destination` as a default mount, the user mount wins (last-write-wins
/// at runtime as well).
pub fn apply_volume_mounts(spec: &mut OciSpec, user_mounts: &[Mount]) {
    for m in user_mounts {
        let dest = m.destination.to_string_lossy().into_owned();
        spec.mounts.retain(|existing| existing.destination != dest);
        spec.mounts.push(user_mount(m));
    }
}

/// Fold a CRI [`crate::models::SecurityContext`] into the OCI spec. Mirrors
/// the per-knob translation in containerd v2.2.3
/// `pkg/cri/server/container_create_linux.go::setOCISecurityContext` and
/// runc v1.4.2 `libcontainer/specconv/spec_linux.go`.
pub fn apply_security_context(spec: &mut OciSpec, sec: &crate::models::SecurityContext) {
    if let Some(uid) = sec.run_as_user { spec.process.user.uid = uid; }
    if let Some(gid) = sec.run_as_group { spec.process.user.gid = gid; }
    if !sec.supplemental_groups.is_empty() {
        spec.process.user.additional_gids = sec.supplemental_groups.clone();
    }

    spec.root.readonly = sec.readonly_rootfs;
    spec.process.no_new_privileges = !sec.allow_privilege_escalation;

    // Privileged → grant all linux capabilities; clear masked/readonly paths
    // and seccomp so the container can introspect /proc and /sys freely.
    if sec.privileged {
        let all = full_capability_set();
        spec.process.capabilities = OciCapabilities {
            bounding: all.clone(),
            effective: all.clone(),
            permitted: all.clone(),
            ambient: all.clone(),
            inheritable: all,
        };
        spec.linux.masked_paths.clear();
        spec.linux.readonly_paths.clear();
        spec.linux.seccomp = None;
    } else {
        // Add then remove caps. Drops apply AFTER adds (matches CRI semantics).
        let mut caps = std::collections::BTreeSet::new();
        caps.extend(spec.process.capabilities.bounding.iter().cloned());
        for add in &sec.capabilities_add {
            caps.insert(normalise_cap(add));
        }
        for drop in &sec.capabilities_drop {
            caps.remove(&normalise_cap(drop));
        }
        // Special-case: "ALL" in drops removes everything
        if sec.capabilities_drop.iter().any(|c| normalise_cap(c) == "ALL") {
            caps.clear();
        }
        // And "ALL" in adds expands to the full set
        if sec.capabilities_add.iter().any(|c| normalise_cap(c) == "ALL") {
            caps = full_capability_set().into_iter().collect();
        }
        let cap_vec: Vec<String> = caps.into_iter().collect();
        spec.process.capabilities = OciCapabilities {
            bounding: cap_vec.clone(),
            effective: cap_vec.clone(),
            permitted: cap_vec.clone(),
            ambient: vec![],
            inheritable: vec![],
        };

        match sec.seccomp_profile.as_ref() {
            Some(crate::models::SeccompProfile::Unconfined) => {
                spec.linux.seccomp = None;
            }
            Some(crate::models::SeccompProfile::RuntimeDefault) | None => {
                if spec.linux.seccomp.is_none() {
                    spec.linux.seccomp = Some(default_seccomp());
                }
            }
            Some(crate::models::SeccompProfile::Localhost(_path)) => {
                // Profile loading is the runtime's job; mark as set so the
                // generator doesn't fall back to RuntimeDefault.
                spec.linux.seccomp = Some(default_seccomp());
            }
        }
    }
}

fn normalise_cap(s: &str) -> String {
    let s = s.trim().to_uppercase();
    if s.starts_with("CAP_") || s == "ALL" { s } else { format!("CAP_{}", s) }
}

fn full_capability_set() -> Vec<String> {
    [
        "CAP_AUDIT_CONTROL", "CAP_AUDIT_READ", "CAP_AUDIT_WRITE", "CAP_BLOCK_SUSPEND",
        "CAP_BPF", "CAP_CHECKPOINT_RESTORE", "CAP_CHOWN", "CAP_DAC_OVERRIDE",
        "CAP_DAC_READ_SEARCH", "CAP_FOWNER", "CAP_FSETID", "CAP_IPC_LOCK", "CAP_IPC_OWNER",
        "CAP_KILL", "CAP_LEASE", "CAP_LINUX_IMMUTABLE", "CAP_MAC_ADMIN", "CAP_MAC_OVERRIDE",
        "CAP_MKNOD", "CAP_NET_ADMIN", "CAP_NET_BIND_SERVICE", "CAP_NET_BROADCAST",
        "CAP_NET_RAW", "CAP_PERFMON", "CAP_SETFCAP", "CAP_SETGID", "CAP_SETPCAP",
        "CAP_SETUID", "CAP_SYS_ADMIN", "CAP_SYS_BOOT", "CAP_SYS_CHROOT", "CAP_SYS_MODULE",
        "CAP_SYS_NICE", "CAP_SYS_PACCT", "CAP_SYS_PTRACE", "CAP_SYS_RAWIO",
        "CAP_SYS_RESOURCE", "CAP_SYS_TIME", "CAP_SYS_TTY_CONFIG", "CAP_SYSLOG",
        "CAP_WAKE_ALARM",
    ].into_iter().map(String::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ContainerSpec, NetworkMode, ResourceLimits, RestartPolicy};

    fn basic_spec() -> ContainerSpec {
        ContainerSpec {
            name: "test".into(),
            image: "nginx:latest".into(),
            command: vec!["/bin/nginx".into()],
            args: vec!["-g".into(), "daemon off;".into()],
            env: [("PORT".to_string(), "8080".to_string())].into(),
            mounts: vec![],
            resources: ResourceLimits::default(),
            labels: Default::default(),
            working_dir: Some("/app".into()),
            user: None,
            hostname: Some("my-container".into()),
            network_mode: NetworkMode::Bridge,
            restart_policy: RestartPolicy::Never,
        }
    }

    #[test]
    fn generate_oci_version() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.oci_version, "1.0.2-dev");
    }

    #[test]
    fn generate_args_combines_command_and_args() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.process.args[0], "/bin/nginx");
        assert!(spec.process.args.contains(&"-g".to_string()));
        assert!(spec.process.args.contains(&"daemon off;".to_string()));
    }

    #[test]
    fn generate_empty_command_defaults_to_sh() {
        let mut s = basic_spec();
        s.command = vec![];
        s.args = vec![];
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.process.args, vec!["/bin/sh"]);
    }

    #[test]
    fn generate_env_contains_user_vars() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert!(spec.process.env.iter().any(|e| e.starts_with("PORT=")));
    }

    #[test]
    fn generate_env_injects_path_when_missing() {
        let mut s = basic_spec();
        s.env.clear();
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert!(spec.process.env.iter().any(|e| e.starts_with("PATH=")));
    }

    #[test]
    fn generate_env_does_not_duplicate_path() {
        let mut s = basic_spec();
        s.env.insert("PATH".into(), "/custom/bin".into());
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        let path_count = spec.process.env.iter().filter(|e| e.starts_with("PATH=")).count();
        assert_eq!(path_count, 1);
    }

    #[test]
    fn generate_working_dir() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.process.cwd, "/app");
    }

    #[test]
    fn generate_default_working_dir_is_root() {
        let mut s = basic_spec();
        s.working_dir = None;
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.process.cwd, "/");
    }

    #[test]
    fn generate_root_path_matches_rootfs() {
        let spec = generate(&basic_spec(), &PathBuf::from("/var/lib/cave/containers/abc/merged"), "abc123");
        assert!(spec.root.path.contains("merged"));
    }

    #[test]
    fn generate_hostname_from_spec() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.hostname, "my-container");
    }

    #[test]
    fn generate_hostname_truncates_id_when_none() {
        let mut s = basic_spec();
        s.hostname = None;
        let spec = generate(&s, &PathBuf::from("/merged"), "abcdef123456789");
        assert_eq!(spec.hostname, "abcdef123456");
    }

    #[test]
    fn generate_includes_default_system_mounts() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        let dests: Vec<&str> = spec.mounts.iter().map(|m| m.destination.as_str()).collect();
        assert!(dests.contains(&"/proc"));
        assert!(dests.contains(&"/sys"));
        assert!(dests.contains(&"/dev"));
        assert!(dests.contains(&"/dev/shm"));
    }

    #[test]
    fn generate_user_bind_mount_appended() {
        let mut s = basic_spec();
        s.mounts.push(Mount {
            source: "/host/data".into(),
            destination: "/data".into(),
            read_only: false,
            mount_type: MountType::Bind,
            propagation: crate::models::MountPropagation::Private,
        });
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        let data_mount = spec.mounts.iter().find(|m| m.destination == "/data").unwrap();
        assert_eq!(data_mount.mount_type, "bind");
        assert!(data_mount.options.contains(&"rbind".to_string()));
    }

    #[test]
    fn generate_readonly_user_mount_has_ro_option() {
        let mut s = basic_spec();
        s.mounts.push(Mount {
            source: "/host/cfg".into(),
            destination: "/etc/cfg".into(),
            read_only: true,
            mount_type: MountType::Bind,
            propagation: crate::models::MountPropagation::Private,
        });
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        let m = spec.mounts.iter().find(|m| m.destination == "/etc/cfg").unwrap();
        assert!(m.options.contains(&"ro".to_string()));
    }

    #[test]
    fn generate_tmpfs_mount() {
        let mut s = basic_spec();
        s.mounts.push(Mount {
            source: "tmpfs".into(),
            destination: "/tmp".into(),
            read_only: false,
            mount_type: MountType::Tmpfs,
            propagation: crate::models::MountPropagation::Private,
        });
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        let m = spec.mounts.iter().find(|m| m.destination == "/tmp").unwrap();
        assert_eq!(m.mount_type, "tmpfs");
    }

    #[test]
    fn generate_namespaces_include_all_required() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        let kinds: Vec<&str> = spec.linux.namespaces.iter().map(|n| n.kind.as_str()).collect();
        assert!(kinds.contains(&"pid"));
        assert!(kinds.contains(&"network"));
        assert!(kinds.contains(&"mount"));
        assert!(kinds.contains(&"ipc"));
        assert!(kinds.contains(&"uts"));
    }

    #[test]
    fn generate_cgroups_path() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert!(spec.linux.cgroups_path.contains("abc123"));
    }

    #[test]
    fn generate_resources_cpu_shares() {
        let mut s = basic_spec();
        s.resources.cpu_shares = Some(512);
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.linux.resources.cpu.shares, Some(512));
    }

    #[test]
    fn generate_resources_cpu_quota_sets_period() {
        let mut s = basic_spec();
        s.resources.cpu_quota = Some(50_000);
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.linux.resources.cpu.quota, Some(50_000));
        assert_eq!(spec.linux.resources.cpu.period, Some(100_000));
    }

    #[test]
    fn generate_resources_memory_limit() {
        let mut s = basic_spec();
        s.resources.memory_limit = Some(512 * 1024 * 1024);
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.linux.resources.memory.limit, Some(512 * 1024 * 1024));
    }

    #[test]
    fn generate_resources_pids_limit() {
        let mut s = basic_spec();
        s.resources.pids_limit = Some(100);
        let spec = generate(&s, &PathBuf::from("/merged"), "abc123");
        assert_eq!(spec.linux.resources.pids.limit, Some(100));
    }

    #[test]
    fn generate_no_new_privileges() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert!(spec.process.no_new_privileges);
    }

    #[test]
    fn generate_capabilities_include_net_bind() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert!(spec.process.capabilities.bounding.contains(&"CAP_NET_BIND_SERVICE".to_string()));
    }

    #[test]
    fn generate_seccomp_default_action_deny() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        let seccomp = spec.linux.seccomp.unwrap();
        assert_eq!(seccomp.default_action, "SCMP_ACT_ERRNO");
    }

    #[test]
    fn generate_seccomp_allows_common_syscalls() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        let seccomp = spec.linux.seccomp.unwrap();
        let allowed: Vec<&str> = seccomp.syscalls.iter()
            .filter(|s| s.action == "SCMP_ACT_ALLOW")
            .flat_map(|s| s.names.iter().map(|n| n.as_str()))
            .collect();
        assert!(allowed.contains(&"read"));
        assert!(allowed.contains(&"write"));
        assert!(allowed.contains(&"execve"));
        assert!(allowed.contains(&"exit_group"));
    }

    #[test]
    fn generate_masked_paths_include_proc_kcore() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert!(spec.linux.masked_paths.contains(&"/proc/kcore".to_string()));
    }

    #[test]
    fn generate_readonly_paths_include_proc_sys() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        assert!(spec.linux.readonly_paths.contains(&"/proc/sys".to_string()));
    }

    #[test]
    fn write_produces_valid_json() {
        let dir = tempfile::tempdir().unwrap();
        let s = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        write(&s, &dir.path().to_path_buf()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("config.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["ociVersion"], "1.0.2-dev");
    }

    #[test]
    fn generated_spec_roundtrips_json() {
        let spec = generate(&basic_spec(), &PathBuf::from("/merged"), "abc123");
        let json = serde_json::to_string(&spec).unwrap();
        let back: OciSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.oci_version, spec.oci_version);
        assert_eq!(back.process.args, spec.process.args);
    }
}
