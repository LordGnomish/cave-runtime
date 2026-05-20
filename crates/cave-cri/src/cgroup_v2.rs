// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cgroup v2 unified-hierarchy extensions.
//!
//! cgroup v1 had per-controller filesystem hierarchies with `devices.allow`
//! / `devices.deny` ASCII rules; cgroup v2's "unified hierarchy" replaces
//! the devices controller with eBPF cgroup_device programs and adds soft
//! pressure knobs (`memory.high`, `cpu.weight.nice`). This module covers
//! the parts of that surface that `crate::cgroup` doesn't already touch:
//!
//! - `memory.high` (soft pressure) and `memory.swap.max`
//! - `cpu.weight` (1..10000) and `cpu.weight.nice` (-20..19)
//! - `io.weight` (1..10000) and per-device `io.max` rbps/wbps caps
//! - BPF cgroup_device program assembly — the textual representation of
//!   the BPF instructions that the runtime would JIT and attach to the
//!   cgroup file descriptor via `BPF_PROG_ATTACH(BPF_CGROUP_DEVICE)`.
//! - `check_unified_hierarchy(root)` — confirms `cgroup.controllers`
//!   exists at the root, distinguishing v2 from v1 hybrid mounts.
//!
//! Upstream:
//! - kernel docs: <https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html>
//! - runc:        `libcontainer/cgroups/fs2/devices.go`,
//!                `libcontainer/cgroups/devices/ebpf.go`
//! - containerd:  `pkg/cri/server/container_create_linux.go` (resources to
//!                cgroup-v2 translation)

use crate::error::{CriError, CriResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Extended resource limits ─────────────────────────────────────────────────

/// Cgroup v2 specific knobs not covered by the legacy ResourceLimits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CgroupV2Limits {
    /// `memory.high` — soft pressure threshold; processes get throttled
    /// before the hard limit is hit. None → "max".
    pub memory_high: Option<u64>,
    /// `memory.swap.max` — hard cap on swap. None → "max".
    pub memory_swap_max: Option<u64>,
    /// `cpu.weight` — proportional CPU share, 1..10000 (default 100).
    pub cpu_weight: Option<u32>,
    /// `cpu.weight.nice` — nice-style alternate encoding, -20..19.
    pub cpu_weight_nice: Option<i32>,
    /// `io.weight` — block I/O weight, 1..10000.
    pub io_weight: Option<u32>,
    /// Per-device `io.max` rbps/wbps caps. Each entry becomes one line:
    /// `<major>:<minor> rbps=<n> wbps=<n>`.
    #[serde(default)]
    pub io_max: Vec<IoMaxEntry>,
    /// Device access rules; emitted as a BPF cgroup_device program.
    #[serde(default)]
    pub devices: Vec<DeviceRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoMaxEntry {
    pub major: u32,
    pub minor: u32,
    pub rbps: Option<u64>,
    pub wbps: Option<u64>,
    pub riops: Option<u64>,
    pub wiops: Option<u64>,
}

impl IoMaxEntry {
    pub fn render(&self) -> String {
        let mut parts = vec![format!("{}:{}", self.major, self.minor)];
        if let Some(v) = self.rbps {
            parts.push(format!("rbps={}", v));
        }
        if let Some(v) = self.wbps {
            parts.push(format!("wbps={}", v));
        }
        if let Some(v) = self.riops {
            parts.push(format!("riops={}", v));
        }
        if let Some(v) = self.wiops {
            parts.push(format!("wiops={}", v));
        }
        parts.join(" ")
    }
}

// ── Device BPF rules ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    /// `'a'` — applies to both block and char devices.
    All,
    /// `'b'` — block devices.
    Block,
    /// `'c'` — character devices.
    Char,
}

impl DeviceType {
    pub fn as_char(self) -> char {
        match self {
            DeviceType::All => 'a',
            DeviceType::Block => 'b',
            DeviceType::Char => 'c',
        }
    }
}

/// One device-access rule. Mirrors runc's `configs.DeviceRule`.
///
/// `access` is a free-form subset of `r` (read), `w` (write), `m` (mknod).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRule {
    pub kind: DeviceType,
    /// `None` → wildcard.
    pub major: Option<i64>,
    /// `None` → wildcard.
    pub minor: Option<i64>,
    /// Subset of `rwm` granted (allow=true) or denied (allow=false).
    pub access: String,
    pub allow: bool,
}

impl DeviceRule {
    /// runc's default: deny-all to start, then allowlist the standard
    /// container devices (null, zero, full, random, urandom, tty, ptmx, …).
    pub fn default_deny_all() -> Self {
        Self {
            kind: DeviceType::All,
            major: None,
            minor: None,
            access: "rwm".into(),
            allow: false,
        }
    }

    /// Standard set of devices runc allows by default — everything in
    /// `/dev/{null,zero,full,random,urandom,tty,zero,ptmx}` plus
    /// `/dev/console`. Used as a baseline for unprivileged containers.
    pub fn default_allowlist() -> Vec<DeviceRule> {
        vec![
            // /dev/null
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(1),
                minor: Some(3),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/zero
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(1),
                minor: Some(5),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/full
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(1),
                minor: Some(7),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/random
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(1),
                minor: Some(8),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/urandom
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(1),
                minor: Some(9),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/tty
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(5),
                minor: Some(0),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/console
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(5),
                minor: Some(1),
                access: "rwm".into(),
                allow: true,
            },
            // /dev/ptmx
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(5),
                minor: Some(2),
                access: "rwm".into(),
                allow: true,
            },
            // PTY slaves (major 136, any minor)
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(136),
                minor: None,
                access: "rwm".into(),
                allow: true,
            },
        ]
    }
}

/// Pseudo-instruction emitted by the BPF assembler. We keep this textual
/// rather than dropping into raw `bpf_insn` so tests can read it without
/// libbpf and so the snapshot is stable across kernel versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BpfInstruction {
    pub op: String,
    pub comment: String,
}

/// Assemble the BPF cgroup_device program for a list of rules. The
/// assembled bytecode would normally be loaded with
/// `bpf(BPF_PROG_LOAD, BPF_PROG_TYPE_CGROUP_DEVICE, …)`.
///
/// Algorithm mirrors runc's `libcontainer/cgroups/devices/ebpf.go`:
///
/// 1. Load the device type, major, minor, access mask from the
///    `bpf_cgroup_dev_ctx` struct.
/// 2. For each allow rule (in order), emit a comparison block that jumps
///    to "return 1" on match.
/// 3. For each deny rule, emit a block that jumps to "return 0".
/// 4. Default: return 0 (deny).
pub fn assemble_device_program(rules: &[DeviceRule]) -> Vec<BpfInstruction> {
    let mut prog = Vec::new();
    prog.push(BpfInstruction {
        op: "BPF_LDX_MEM(BPF_W, R2, R1, type)".into(),
        comment: "load device type into R2".into(),
    });
    prog.push(BpfInstruction {
        op: "BPF_LDX_MEM(BPF_W, R3, R1, major)".into(),
        comment: "load major into R3".into(),
    });
    prog.push(BpfInstruction {
        op: "BPF_LDX_MEM(BPF_W, R4, R1, minor)".into(),
        comment: "load minor into R4".into(),
    });
    prog.push(BpfInstruction {
        op: "BPF_LDX_MEM(BPF_W, R5, R1, access)".into(),
        comment: "load access mask into R5".into(),
    });
    for rule in rules {
        let action = if rule.allow {
            "return 1 (allow)"
        } else {
            "return 0 (deny)"
        };
        prog.push(BpfInstruction {
            op: format!(
                "CMP type={}, major={:?}, minor={:?}, access={:?} JMP_TO {}",
                rule.kind.as_char(),
                rule.major,
                rule.minor,
                rule.access,
                action,
            ),
            comment: format!(
                "{} {}{}{}",
                if rule.allow { "allow" } else { "deny" },
                rule.kind.as_char(),
                match rule.major {
                    Some(m) => format!(" {}", m),
                    None => " *".into(),
                },
                match rule.minor {
                    Some(m) => format!(":{}", m),
                    None => ":*".into(),
                },
            ),
        });
    }
    prog.push(BpfInstruction {
        op: "BPF_MOV64_IMM(R0, 0)".into(),
        comment: "default deny".into(),
    });
    prog.push(BpfInstruction {
        op: "BPF_EXIT_INSN()".into(),
        comment: "return".into(),
    });
    prog
}

// ── File writers (tempdir-driven for tests) ──────────────────────────────────

/// Apply `CgroupV2Limits` underneath `cgroup_dir`. Pure file-IO so the
/// behaviour can be exercised inside a tempdir on macOS.
pub fn apply_v2(cgroup_dir: &Path, limits: &CgroupV2Limits) -> CriResult<()> {
    std::fs::create_dir_all(cgroup_dir).map_err(CriError::Io)?;

    if let Some(high) = limits.memory_high {
        write_file(&cgroup_dir.join("memory.high"), &high.to_string())?;
    }
    if let Some(swap_max) = limits.memory_swap_max {
        write_file(&cgroup_dir.join("memory.swap.max"), &swap_max.to_string())?;
    }
    if let Some(weight) = limits.cpu_weight {
        if !(1..=10_000).contains(&weight) {
            return Err(CriError::Cgroup(format!(
                "cpu.weight {} out of range 1..=10000",
                weight
            )));
        }
        write_file(&cgroup_dir.join("cpu.weight"), &weight.to_string())?;
    }
    if let Some(nice) = limits.cpu_weight_nice {
        if !(-20..=19).contains(&nice) {
            return Err(CriError::Cgroup(format!(
                "cpu.weight.nice {} out of range -20..=19",
                nice
            )));
        }
        write_file(&cgroup_dir.join("cpu.weight.nice"), &nice.to_string())?;
    }
    if let Some(weight) = limits.io_weight {
        if !(1..=10_000).contains(&weight) {
            return Err(CriError::Cgroup(format!(
                "io.weight {} out of range 1..=10000",
                weight
            )));
        }
        write_file(&cgroup_dir.join("io.weight"), &weight.to_string())?;
    }
    for entry in &limits.io_max {
        write_file(&cgroup_dir.join("io.max"), &entry.render())?;
    }
    if !limits.devices.is_empty() {
        let prog = assemble_device_program(&limits.devices);
        let dump: Vec<String> = prog
            .iter()
            .map(|i| format!("{}\t# {}", i.op, i.comment))
            .collect();
        write_file(&cgroup_dir.join("devices.bpf.prog"), &dump.join("\n"))?;
    }
    Ok(())
}

/// Detect the unified hierarchy by checking for `cgroup.controllers` at
/// `root`. v1 hybrid mounts have `cgroup.subtree_control` only inside
/// individual controller subdirs.
pub fn check_unified_hierarchy(root: &Path) -> CriResult<Vec<String>> {
    let controllers_path = root.join("cgroup.controllers");
    if !controllers_path.exists() {
        return Err(CriError::Cgroup(format!(
            "{} missing — not a cgroup v2 unified hierarchy",
            controllers_path.display()
        )));
    }
    let content = std::fs::read_to_string(&controllers_path).map_err(CriError::Io)?;
    Ok(content.split_whitespace().map(|s| s.to_string()).collect())
}

/// Enable a controller in `cgroup.subtree_control` (e.g. "+cpu", "+memory").
pub fn enable_controller(cgroup_dir: &Path, controller: &str) -> CriResult<()> {
    let path: PathBuf = cgroup_dir.join("cgroup.subtree_control");
    write_file(&path, &format!("+{}", controller))
}

fn write_file(path: &Path, content: &str) -> CriResult<()> {
    std::fs::write(path, content)
        .map_err(|e| CriError::Cgroup(format!("write {} failed: {}", path.display(), e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── IoMaxEntry ───────────────────────────────────────────────────────────

    #[test]
    fn io_max_renders_with_present_caps_only() {
        let e = IoMaxEntry {
            major: 8,
            minor: 0,
            rbps: Some(1_000_000),
            wbps: None,
            riops: None,
            wiops: Some(500),
        };
        let s = e.render();
        assert!(s.starts_with("8:0"));
        assert!(s.contains("rbps=1000000"));
        assert!(!s.contains("wbps="));
        assert!(s.contains("wiops=500"));
    }

    #[test]
    fn io_max_renders_only_device_when_no_caps() {
        let e = IoMaxEntry {
            major: 1,
            minor: 2,
            rbps: None,
            wbps: None,
            riops: None,
            wiops: None,
        };
        assert_eq!(e.render(), "1:2");
    }

    // ── DeviceType ───────────────────────────────────────────────────────────

    #[test]
    fn device_type_chars() {
        assert_eq!(DeviceType::All.as_char(), 'a');
        assert_eq!(DeviceType::Block.as_char(), 'b');
        assert_eq!(DeviceType::Char.as_char(), 'c');
    }

    // ── DeviceRule ───────────────────────────────────────────────────────────

    #[test]
    fn default_deny_all_blocks_everything() {
        let r = DeviceRule::default_deny_all();
        assert!(!r.allow);
        assert_eq!(r.access, "rwm");
        assert_eq!(r.kind, DeviceType::All);
        assert!(r.major.is_none() && r.minor.is_none());
    }

    #[test]
    fn default_allowlist_includes_null_zero_random() {
        let list = DeviceRule::default_allowlist();
        let pairs: Vec<(i64, i64)> = list
            .iter()
            .filter_map(|r| match (r.major, r.minor) {
                (Some(m), Some(n)) => Some((m, n)),
                _ => None,
            })
            .collect();
        assert!(pairs.contains(&(1, 3)), "/dev/null missing");
        assert!(pairs.contains(&(1, 5)), "/dev/zero missing");
        assert!(pairs.contains(&(1, 8)), "/dev/random missing");
        assert!(pairs.contains(&(1, 9)), "/dev/urandom missing");
    }

    #[test]
    fn default_allowlist_includes_pty_slaves_with_wildcard_minor() {
        let list = DeviceRule::default_allowlist();
        assert!(list
            .iter()
            .any(|r| r.major == Some(136) && r.minor.is_none() && r.allow));
    }

    // ── BPF program assembly ─────────────────────────────────────────────────

    #[test]
    fn empty_rules_program_starts_with_loads_and_ends_with_exit() {
        let prog = assemble_device_program(&[]);
        assert!(prog.first().unwrap().op.contains("BPF_LDX_MEM"));
        assert_eq!(prog.last().unwrap().op, "BPF_EXIT_INSN()");
    }

    #[test]
    fn program_loads_all_four_context_fields() {
        let prog = assemble_device_program(&[]);
        let dump: String = prog.iter().map(|i| i.op.clone() + "\n").collect();
        assert!(dump.contains("type"));
        assert!(dump.contains("major"));
        assert!(dump.contains("minor"));
        assert!(dump.contains("access"));
    }

    #[test]
    fn program_includes_one_compare_per_rule() {
        let rules = DeviceRule::default_allowlist();
        let prog = assemble_device_program(&rules);
        let cmps = prog.iter().filter(|i| i.op.starts_with("CMP")).count();
        assert_eq!(cmps, rules.len());
    }

    #[test]
    fn program_default_action_is_deny() {
        let prog = assemble_device_program(&[]);
        let dump: String = prog.iter().map(|i| i.op.clone() + "\n").collect();
        assert!(dump.contains("BPF_MOV64_IMM(R0, 0)"));
    }

    #[test]
    fn program_compare_includes_allow_or_deny_action() {
        let rules = vec![
            DeviceRule {
                kind: DeviceType::Char,
                major: Some(1),
                minor: Some(3),
                access: "rwm".into(),
                allow: true,
            },
            DeviceRule {
                kind: DeviceType::Block,
                major: None,
                minor: None,
                access: "rwm".into(),
                allow: false,
            },
        ];
        let prog = assemble_device_program(&rules);
        let actions: Vec<&str> = prog
            .iter()
            .filter(|i| i.op.starts_with("CMP"))
            .map(|i| i.op.as_str())
            .collect();
        assert!(actions[0].contains("allow"));
        assert!(actions[1].contains("deny"));
    }

    // ── apply_v2 ─────────────────────────────────────────────────────────────

    #[test]
    fn apply_v2_writes_memory_high() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let limits = CgroupV2Limits {
            memory_high: Some(512 * 1024 * 1024),
            ..Default::default()
        };
        apply_v2(&cg, &limits).unwrap();
        let content = std::fs::read_to_string(cg.join("memory.high")).unwrap();
        assert_eq!(content.trim(), "536870912");
    }

    #[test]
    fn apply_v2_writes_memory_swap_max() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let limits = CgroupV2Limits {
            memory_swap_max: Some(0),
            ..Default::default()
        };
        apply_v2(&cg, &limits).unwrap();
        assert_eq!(
            std::fs::read_to_string(cg.join("memory.swap.max"))
                .unwrap()
                .trim(),
            "0"
        );
    }

    #[test]
    fn apply_v2_writes_cpu_weight_in_range() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        apply_v2(
            &cg,
            &CgroupV2Limits {
                cpu_weight: Some(2500),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(cg.join("cpu.weight"))
                .unwrap()
                .trim(),
            "2500"
        );
    }

    #[test]
    fn apply_v2_rejects_cpu_weight_out_of_range() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let err = apply_v2(
            &cg,
            &CgroupV2Limits {
                cpu_weight: Some(20_000),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn apply_v2_writes_cpu_weight_nice() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        apply_v2(
            &cg,
            &CgroupV2Limits {
                cpu_weight_nice: Some(-5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(cg.join("cpu.weight.nice"))
                .unwrap()
                .trim(),
            "-5"
        );
    }

    #[test]
    fn apply_v2_rejects_cpu_weight_nice_out_of_range() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let err = apply_v2(
            &cg,
            &CgroupV2Limits {
                cpu_weight_nice: Some(50),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn apply_v2_writes_io_weight() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        apply_v2(
            &cg,
            &CgroupV2Limits {
                io_weight: Some(500),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(cg.join("io.weight"))
                .unwrap()
                .trim(),
            "500"
        );
    }

    #[test]
    fn apply_v2_rejects_io_weight_out_of_range() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let err = apply_v2(
            &cg,
            &CgroupV2Limits {
                io_weight: Some(0),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn apply_v2_writes_io_max_entry() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let limits = CgroupV2Limits {
            io_max: vec![IoMaxEntry {
                major: 8,
                minor: 0,
                rbps: Some(1_000_000),
                wbps: Some(500_000),
                riops: None,
                wiops: None,
            }],
            ..Default::default()
        };
        apply_v2(&cg, &limits).unwrap();
        let content = std::fs::read_to_string(cg.join("io.max")).unwrap();
        assert!(content.contains("8:0"));
        assert!(content.contains("rbps=1000000"));
        assert!(content.contains("wbps=500000"));
    }

    #[test]
    fn apply_v2_writes_devices_bpf_program() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        let limits = CgroupV2Limits {
            devices: DeviceRule::default_allowlist(),
            ..Default::default()
        };
        apply_v2(&cg, &limits).unwrap();
        let dump = std::fs::read_to_string(cg.join("devices.bpf.prog")).unwrap();
        assert!(dump.contains("BPF_LDX_MEM"));
        assert!(dump.contains("BPF_EXIT_INSN"));
        // Each rule emits a CMP line.
        let cmps = dump.lines().filter(|l| l.starts_with("CMP")).count();
        assert_eq!(cmps, DeviceRule::default_allowlist().len());
    }

    #[test]
    fn apply_v2_with_default_limits_is_noop() {
        let dir = tempdir().unwrap();
        let cg = dir.path().join("cg");
        apply_v2(&cg, &CgroupV2Limits::default()).unwrap();
        // Directory is created, but no knob files appear.
        assert!(cg.exists());
        assert!(!cg.join("memory.high").exists());
        assert!(!cg.join("cpu.weight").exists());
        assert!(!cg.join("devices.bpf.prog").exists());
    }

    // ── check_unified_hierarchy ──────────────────────────────────────────────

    #[test]
    fn check_unified_hierarchy_reads_controller_list() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.controllers"), "cpu memory pids io").unwrap();
        let controllers = check_unified_hierarchy(dir.path()).unwrap();
        assert_eq!(controllers, vec!["cpu", "memory", "pids", "io"]);
    }

    #[test]
    fn check_unified_hierarchy_missing_file_errors() {
        let dir = tempdir().unwrap();
        let err = check_unified_hierarchy(dir.path()).unwrap_err();
        assert!(err.to_string().contains("not a cgroup v2"));
    }

    #[test]
    fn check_unified_hierarchy_handles_extra_whitespace() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("cgroup.controllers"),
            "  cpu \t memory \n pids ",
        )
        .unwrap();
        let controllers = check_unified_hierarchy(dir.path()).unwrap();
        assert_eq!(controllers, vec!["cpu", "memory", "pids"]);
    }

    // ── enable_controller ────────────────────────────────────────────────────

    #[test]
    fn enable_controller_writes_plus_prefix() {
        let dir = tempdir().unwrap();
        enable_controller(dir.path(), "memory").unwrap();
        let content = std::fs::read_to_string(dir.path().join("cgroup.subtree_control")).unwrap();
        assert_eq!(content, "+memory");
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn cgroup_v2_limits_roundtrip_through_json() {
        let limits = CgroupV2Limits {
            memory_high: Some(100),
            memory_swap_max: Some(0),
            cpu_weight: Some(500),
            cpu_weight_nice: Some(0),
            io_weight: Some(200),
            io_max: vec![IoMaxEntry {
                major: 8,
                minor: 0,
                rbps: Some(1),
                wbps: None,
                riops: None,
                wiops: None,
            }],
            devices: vec![DeviceRule::default_deny_all()],
        };
        let json = serde_json::to_string(&limits).unwrap();
        let back: CgroupV2Limits = serde_json::from_str(&json).unwrap();
        assert_eq!(limits, back);
    }

    #[test]
    fn device_rule_roundtrip_through_json() {
        let r = DeviceRule {
            kind: DeviceType::Char,
            major: Some(1),
            minor: Some(3),
            access: "rwm".into(),
            allow: true,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: DeviceRule = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
