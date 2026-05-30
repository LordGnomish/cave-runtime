// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! eBPF program loader — userspace approximation of grafana/beyla
//! `pkg/internal/ebpf/tracer.go` and the cilium/ebpf `CollectionSpec`
//! load path.
//!
//! ## Userspace approximation boundary
//!
//! Beyla loads a bpf2go-generated ELF object, resolves its maps, runs the
//! in-kernel verifier (via the `bpf(2)` syscall), and pins the programs.
//! This module ports the *userspace* half of that flow — the part that is
//! deterministic and testable without a kernel: spec validation, map-ref
//! resolution, duplicate detection, and the verifier's GPL-license gate
//! that rejects non-GPL programs which call GPL-only helpers
//! (`bpf_probe_read_kernel`, `bpf_get_current_task`, …).
//!
//! The actual `bpf(2)` FFI (`BPF_PROG_LOAD`, `BPF_MAP_CREATE`) is **not**
//! crossed here — it would require `libbpf-rs`/`aya` and a privileged
//! kernel. That boundary is tracked honestly as a `partial` subsystem in
//! `parity.manifest.toml` (`bpf-syscall-ffi`).

use serde::{Deserialize, Serialize};

/// eBPF program attach type (subset Beyla uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgramType {
    Kprobe,
    Kretprobe,
    Uprobe,
    Uretprobe,
    Tracepoint,
    SocketFilter,
    SchedCls,
}

impl ProgramType {
    /// Kernel-probe types that may invoke GPL-only helpers and so are
    /// subject to the verifier's GPL-license gate.
    pub fn requires_gpl(self) -> bool {
        matches!(
            self,
            ProgramType::Kprobe
                | ProgramType::Kretprobe
                | ProgramType::Tracepoint
                | ProgramType::SchedCls
        )
    }
}

/// eBPF map kind (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapType {
    Hash,
    LruHash,
    Array,
    PerCpuArray,
    RingBuf,
    PerfEventArray,
}

/// Declared program in an object's `CollectionSpec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramSpec {
    pub name: String,
    pub prog_type: ProgramType,
    pub section: String,
    pub attach_to: String,
    pub license: String,
    /// Names of maps this program references (resolved against the spec).
    pub map_refs: Vec<String>,
}

/// Declared map in a `CollectionSpec`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapSpec {
    pub name: String,
    pub map_type: MapType,
    pub key_size: u32,
    pub value_size: u32,
    pub max_entries: u32,
}

/// A whole object's declared programs + maps, prior to load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSpec {
    pub programs: Vec<ProgramSpec>,
    pub maps: Vec<MapSpec>,
}

/// A "loaded" collection — userspace model of resolved programs+maps.
#[derive(Debug, Clone)]
pub struct Collection {
    programs: Vec<ProgramSpec>,
    maps: Vec<MapSpec>,
}

/// Errors surfaced by the userspace load path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderError {
    /// A GPL-only program type carries a non-GPL license.
    GplRequired { program: String },
    /// A program references a map not present in the spec.
    UnresolvedMap { program: String, map: String },
    /// Two programs share a name.
    DuplicateProgram { name: String },
    /// A map declaration is structurally invalid.
    InvalidMap { map: String, reason: String },
}

impl std::fmt::Display for LoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoaderError::GplRequired { program } => {
                write!(f, "program '{program}' uses GPL-only helpers but license is not GPL")
            }
            LoaderError::UnresolvedMap { program, map } => {
                write!(f, "program '{program}' references unknown map '{map}'")
            }
            LoaderError::DuplicateProgram { name } => {
                write!(f, "duplicate program name '{name}'")
            }
            LoaderError::InvalidMap { map, reason } => {
                write!(f, "map '{map}' invalid: {reason}")
            }
        }
    }
}

impl std::error::Error for LoaderError {}

/// True when a license string satisfies the kernel's GPL gate.
///
/// The kernel accepts "GPL", "GPL v2", "GPL and additional rights", and
/// any "Dual …/GPL" combination (e.g. "Dual BSD/GPL", "Dual MIT/GPL").
fn is_gpl_compatible(license: &str) -> bool {
    let l = license.trim();
    l == "GPL"
        || l.starts_with("GPL ")
        || l.starts_with("GPL v2")
        || l == "GPL and additional rights"
        || (l.starts_with("Dual ") && l.ends_with("/GPL"))
}

impl CollectionSpec {
    /// Run the userspace load path: validate and resolve, returning a
    /// [`Collection`] or the first [`LoaderError`].
    pub fn load(&self) -> Result<Collection, LoaderError> {
        // Duplicate program names.
        for (i, p) in self.programs.iter().enumerate() {
            if self.programs[..i].iter().any(|q| q.name == p.name) {
                return Err(LoaderError::DuplicateProgram {
                    name: p.name.clone(),
                });
            }
        }

        // Map validity.
        for m in &self.maps {
            if m.max_entries == 0 {
                return Err(LoaderError::InvalidMap {
                    map: m.name.clone(),
                    reason: "max_entries must be > 0".into(),
                });
            }
        }

        // Per-program: GPL gate + map resolution.
        for p in &self.programs {
            if p.prog_type.requires_gpl() && !is_gpl_compatible(&p.license) {
                return Err(LoaderError::GplRequired {
                    program: p.name.clone(),
                });
            }
            for r in &p.map_refs {
                if !self.maps.iter().any(|m| &m.name == r) {
                    return Err(LoaderError::UnresolvedMap {
                        program: p.name.clone(),
                        map: r.clone(),
                    });
                }
            }
        }

        Ok(Collection {
            programs: self.programs.clone(),
            maps: self.maps.clone(),
        })
    }
}

impl Collection {
    pub fn program_count(&self) -> usize {
        self.programs.len()
    }
    pub fn map_count(&self) -> usize {
        self.maps.len()
    }
    pub fn program(&self, name: &str) -> Option<&ProgramSpec> {
        self.programs.iter().find(|p| p.name == name)
    }
    pub fn map(&self, name: &str) -> Option<&MapSpec> {
        self.maps.iter().find(|m| m.name == name)
    }
}
