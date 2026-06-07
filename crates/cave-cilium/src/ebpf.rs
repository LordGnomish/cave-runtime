// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! eBPF object loader — a userspace port of Cilium's program-load path.
//!
//! Ports the structure cilium drives through the `cilium/ebpf` library and
//! `pkg/datapath/loader`:
//!   * ELF parse of an ET_REL / EM_BPF object (`bpf_xdp.o`, `bpf_lxc.o`, …)
//!     — header validation, section table, `.shstrtab`/`.strtab`/`.symtab`.
//!   * CollectionSpec extraction: legacy `bpf_map_def` map specs named via
//!     the symbol table, and program specs typed from the ELF section name
//!     (`libbpf_prog_type_by_name` / cilium's section prefixes).
//!   * A verifier *model* (license gate + structural checks the in-kernel
//!     verifier would also require) and a loader that assigns map FDs,
//!     attaches programs to tc/xdp/cgroup hooks, and pins maps into bpffs.
//!
//! No kernel syscalls are made — FDs and bpffs are modelled in userspace,
//! the same way `cave-net::ebpf_sim` models the datapath. This is the
//! control-plane bookkeeping cilium's agent performs before/around the
//! actual `bpf()` syscalls.

use std::cell::RefCell;
use std::collections::HashMap;

use thiserror::Error;

// ---------------------------------------------------------------------------
// ELF parsing
// ---------------------------------------------------------------------------

const EM_BPF: u16 = 247;

#[derive(Debug, Error)]
pub enum ElfError {
    #[error("buffer too small ({0} bytes) for ELF64 header")]
    TooSmall(usize),
    #[error("bad ELF magic")]
    BadMagic,
    #[error("not a 64-bit little-endian object")]
    NotElf64Le,
    #[error("e_machine is {0}, expected EM_BPF ({EM_BPF})")]
    NotBpf(u16),
    #[error("section table out of bounds")]
    SectionOob,
}

fn rd_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn rd_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(a)
}

/// A parsed ELF section.
#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub sh_type: u32,
    pub data: Vec<u8>,
    pub link: u32,
    pub info: u32,
    pub entsize: u64,
}

/// A parsed symbol-table entry (Elf64_Sym).
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub info: u8,
    pub shndx: u16,
    pub value: u64,
}

impl Symbol {
    /// Low nibble of `st_info` is the symbol *type* (STT_*).
    pub fn st_type(&self) -> u8 {
        self.info & 0x0f
    }
}

const STT_FUNC: u8 = 2;

/// A parsed relocatable BPF ELF object.
#[derive(Debug, Clone)]
pub struct ElfObject {
    pub sections: Vec<Section>,
}

impl ElfObject {
    /// Parse an ET_REL / EM_BPF ELF64 little-endian object.
    pub fn parse(b: &[u8]) -> Result<Self, ElfError> {
        if b.len() < 64 {
            return Err(ElfError::TooSmall(b.len()));
        }
        if b[0..4] != [0x7f, b'E', b'L', b'F'] {
            return Err(ElfError::BadMagic);
        }
        // EI_CLASS == 2 (64-bit), EI_DATA == 1 (little-endian).
        if b[4] != 2 || b[5] != 1 {
            return Err(ElfError::NotElf64Le);
        }
        let e_machine = rd_u16(b, 18);
        if e_machine != EM_BPF {
            return Err(ElfError::NotBpf(e_machine));
        }
        let e_shoff = rd_u64(b, 40) as usize;
        let e_shentsize = rd_u16(b, 58) as usize;
        let e_shnum = rd_u16(b, 60) as usize;
        let e_shstrndx = rd_u16(b, 62) as usize;

        if e_shoff == 0 || e_shoff + e_shnum * e_shentsize > b.len() {
            return Err(ElfError::SectionOob);
        }

        // First pass: raw section descriptors.
        struct Raw {
            name_off: u32,
            sh_type: u32,
            offset: usize,
            size: usize,
            link: u32,
            info: u32,
            entsize: u64,
        }
        let mut raw = Vec::with_capacity(e_shnum);
        for i in 0..e_shnum {
            let base = e_shoff + i * e_shentsize;
            raw.push(Raw {
                name_off: rd_u32(b, base),
                sh_type: rd_u32(b, base + 4),
                offset: rd_u64(b, base + 24) as usize,
                size: rd_u64(b, base + 32) as usize,
                link: rd_u32(b, base + 40),
                info: rd_u32(b, base + 44),
                entsize: rd_u64(b, base + 56),
            });
        }

        // The section-header string table holds section names.
        let shstr = raw.get(e_shstrndx).ok_or(ElfError::SectionOob)?;
        if shstr.offset + shstr.size > b.len() {
            return Err(ElfError::SectionOob);
        }
        let shstr_data = &b[shstr.offset..shstr.offset + shstr.size];
        let resolve = |off: u32| -> String { read_cstr(shstr_data, off as usize) };

        let mut sections = Vec::with_capacity(e_shnum);
        for r in &raw {
            // SHT_NOBITS (8) has no file data; everything else copies its bytes.
            let data = if r.sh_type == 8 || r.size == 0 {
                Vec::new()
            } else {
                if r.offset + r.size > b.len() {
                    return Err(ElfError::SectionOob);
                }
                b[r.offset..r.offset + r.size].to_vec()
            };
            sections.push(Section {
                name: resolve(r.name_off),
                sh_type: r.sh_type,
                data,
                link: r.link,
                info: r.info,
                entsize: r.entsize,
            });
        }

        Ok(ElfObject { sections })
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn section_index(&self, name: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.name == name)
    }

    /// The license string from the `license` section (NUL-terminated).
    pub fn license(&self) -> String {
        self.section("license")
            .map(|s| read_cstr(&s.data, 0))
            .unwrap_or_default()
    }

    /// Decode `.symtab` against its linked `.strtab`.
    pub fn symbols(&self) -> Vec<Symbol> {
        let Some(symtab) = self
            .sections
            .iter()
            .find(|s| s.sh_type == 2 /*SHT_SYMTAB*/)
        else {
            return Vec::new();
        };
        let strtab = self
            .sections
            .get(symtab.link as usize)
            .map(|s| s.data.as_slice())
            .unwrap_or(&[]);
        let mut out = Vec::new();
        let entsize = if symtab.entsize == 0 {
            24
        } else {
            symtab.entsize as usize
        };
        let mut off = 0;
        while off + 24 <= symtab.data.len() {
            let name_off = rd_u32(&symtab.data, off);
            let info = symtab.data[off + 4];
            let shndx = rd_u16(&symtab.data, off + 6);
            let value = rd_u64(&symtab.data, off + 8);
            out.push(Symbol {
                name: read_cstr(strtab, name_off as usize),
                info,
                shndx,
                value,
            });
            off += entsize;
        }
        out
    }
}

fn read_cstr(buf: &[u8], off: usize) -> String {
    if off >= buf.len() {
        return String::new();
    }
    let end = buf[off..]
        .iter()
        .position(|&c| c == 0)
        .map(|p| off + p)
        .unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[off..end]).into_owned()
}

// ---------------------------------------------------------------------------
// Map / program specs (CollectionSpec analog)
// ---------------------------------------------------------------------------

/// BPF map types (subset of `enum bpf_map_type`) relevant to cilium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapType {
    Hash,
    Array,
    ProgArray,
    PerfEventArray,
    PerCpuHash,
    PerCpuArray,
    LruHash,
    LpmTrie,
    Lru,
    Unknown(u32),
}

impl MapType {
    pub fn from_u32(v: u32) -> Self {
        // Matches uapi `enum bpf_map_type` ordinals.
        match v {
            1 => MapType::Hash,
            2 => MapType::Array,
            3 => MapType::ProgArray,
            4 => MapType::PerfEventArray,
            5 => MapType::PerCpuHash,
            6 => MapType::PerCpuArray,
            9 => MapType::LruHash,
            11 => MapType::LpmTrie,
            other => MapType::Unknown(other),
        }
    }
}

/// A map specification extracted from the `maps` section.
#[derive(Debug, Clone)]
pub struct MapSpec {
    pub name: String,
    pub map_type: MapType,
    pub key_size: u32,
    pub value_size: u32,
    pub max_entries: u32,
    pub flags: u32,
}

/// BPF program types (subset of `enum bpf_prog_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramType {
    Xdp,
    SchedCls,
    SchedAct,
    CGroupSkb,
    CGroupSock,
    CGroupSockAddr,
    SockOps,
    SkMsg,
    SkSkb,
    Kprobe,
    Tracepoint,
}

impl ProgramType {
    /// Derive the program type from an ELF section name, following the
    /// section-prefix table libbpf/cilium use (`libbpf_prog_type_by_name`).
    pub fn from_section(name: &str) -> Option<ProgramType> {
        let n = name;
        let starts = |p: &str| n == p || n.starts_with(&format!("{p}/")) || n.starts_with(p);
        // Order: most specific cgroup variants first.
        if n.starts_with("cgroup/connect")
            || n.starts_with("cgroup/sendmsg")
            || n.starts_with("cgroup/recvmsg")
            || n.starts_with("cgroup/bind")
            || n.starts_with("cgroup/getpeername")
            || n.starts_with("cgroup/getsockname")
        {
            return Some(ProgramType::CGroupSockAddr);
        }
        if n.starts_with("cgroup_skb") || n.starts_with("cgroup/skb") {
            return Some(ProgramType::CGroupSkb);
        }
        if n.starts_with("cgroup/sock") || n.starts_with("cgroup/post_bind") {
            return Some(ProgramType::CGroupSock);
        }
        if starts("xdp") {
            return Some(ProgramType::Xdp);
        }
        if starts("tc") || starts("classifier") || starts("sched_cls") {
            return Some(ProgramType::SchedCls);
        }
        if starts("sched_act") || starts("action") {
            return Some(ProgramType::SchedAct);
        }
        if starts("sockops") || starts("sock_ops") {
            return Some(ProgramType::SockOps);
        }
        if starts("sk_msg") {
            return Some(ProgramType::SkMsg);
        }
        if starts("sk_skb") {
            return Some(ProgramType::SkSkb);
        }
        if starts("kprobe") || starts("kretprobe") {
            return Some(ProgramType::Kprobe);
        }
        if starts("tracepoint") || starts("tp") {
            return Some(ProgramType::Tracepoint);
        }
        None
    }
}

/// A program specification: typed bytecode awaiting load.
#[derive(Debug, Clone)]
pub struct ProgramSpec {
    pub name: String,
    pub section: String,
    pub prog_type: ProgramType,
    pub instructions: Vec<u8>,
}

/// CollectionSpec analog — everything the loader needs from the ELF.
#[derive(Debug, Clone)]
pub struct BpfObject {
    pub license: String,
    pub maps: Vec<MapSpec>,
    pub programs: Vec<ProgramSpec>,
}

impl BpfObject {
    /// Build the collection spec from a parsed ELF, resolving map and
    /// program names through the symbol table exactly as libbpf does.
    pub fn from_elf(elf: &ElfObject) -> Result<Self, ElfError> {
        let license = elf.license();
        let symbols = elf.symbols();

        // Maps: every symbol pointing into the `maps` section names a
        // legacy bpf_map_def at `st_value`.
        let mut maps = Vec::new();
        if let (Some(maps_idx), Some(maps_sec)) = (elf.section_index("maps"), elf.section("maps")) {
            for s in symbols.iter().filter(|s| s.shndx as usize == maps_idx) {
                let off = s.value as usize;
                if off + 20 <= maps_sec.data.len() {
                    let d = &maps_sec.data;
                    maps.push(MapSpec {
                        name: s.name.clone(),
                        map_type: MapType::from_u32(rd_u32(d, off)),
                        key_size: rd_u32(d, off + 4),
                        value_size: rd_u32(d, off + 8),
                        max_entries: rd_u32(d, off + 12),
                        flags: rd_u32(d, off + 16),
                    });
                }
            }
            maps.sort_by(|a, b| a.name.cmp(&b.name));
        }

        // Programs: each section whose name carries a program-type prefix.
        let mut programs = Vec::new();
        for (idx, sec) in elf.sections.iter().enumerate() {
            let Some(prog_type) = ProgramType::from_section(&sec.name) else {
                continue;
            };
            if sec.data.is_empty() {
                continue;
            }
            // Prefer the FUNC symbol that lives in this section for the name.
            let name = symbols
                .iter()
                .find(|s| s.shndx as usize == idx && s.st_type() == STT_FUNC && !s.name.is_empty())
                .map(|s| s.name.clone())
                .unwrap_or_else(|| sec.name.clone());
            programs.push(ProgramSpec {
                name,
                section: sec.name.clone(),
                prog_type,
                instructions: sec.data.clone(),
            });
        }
        programs.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(BpfObject {
            license,
            maps,
            programs,
        })
    }
}

// ---------------------------------------------------------------------------
// Verifier model + loader
// ---------------------------------------------------------------------------

const BPF_EXIT: u8 = 0x95;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifierError {
    #[error("program license is missing")]
    MissingLicense,
    #[error("program license {0:?} is not GPL-compatible")]
    NonGplLicense(String),
    #[error("program {name} has no instructions")]
    EmptyProgram { name: String },
    #[error("program {name} instruction stream is not 8-byte aligned ({len} bytes)")]
    BadLength { name: String, len: usize },
    #[error("program {name} does not end with BPF_EXIT")]
    NoExit { name: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoaderError {
    #[error("no program named {0}")]
    UnknownProgram(String),
    #[error("no map named {0}")]
    UnknownMap(String),
    #[error("program type {prog:?} cannot attach to {attach:?}")]
    IncompatibleAttach {
        prog: ProgramType,
        attach: AttachType,
    },
}

/// Attach points (BPF hook types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachType {
    Xdp,
    TcIngress,
    TcEgress,
    CGroupInetIngress,
    CGroupSockAddr,
    SockOps,
}

/// A live attachment of a program to a hook.
#[derive(Debug, Clone)]
pub struct Link {
    pub program: String,
    pub attach_type: AttachType,
    pub target: String,
}

fn attach_compatible(prog: ProgramType, attach: AttachType) -> bool {
    matches!(
        (prog, attach),
        (ProgramType::Xdp, AttachType::Xdp)
            | (ProgramType::SchedCls, AttachType::TcIngress)
            | (ProgramType::SchedCls, AttachType::TcEgress)
            | (ProgramType::SchedAct, AttachType::TcIngress)
            | (ProgramType::SchedAct, AttachType::TcEgress)
            | (ProgramType::CGroupSkb, AttachType::CGroupInetIngress)
            | (ProgramType::CGroupSockAddr, AttachType::CGroupSockAddr)
            | (ProgramType::SockOps, AttachType::SockOps)
    )
}

/// A loaded collection: assigned map FDs, loaded programs, bpffs pins.
pub struct Collection {
    map_fds: HashMap<String, u32>,
    programs: HashMap<String, ProgramType>,
    pinned: RefCell<HashMap<String, String>>,
}

impl Collection {
    pub fn map_fd(&self, name: &str) -> Option<u32> {
        self.map_fds.get(name).copied()
    }

    pub fn has_program(&self, name: &str) -> bool {
        self.programs.contains_key(name)
    }

    /// Attach a loaded program to a hook, enforcing type compatibility.
    pub fn attach(
        &self,
        program: &str,
        attach_type: AttachType,
        target: &str,
    ) -> Result<Link, LoaderError> {
        let prog = *self
            .programs
            .get(program)
            .ok_or_else(|| LoaderError::UnknownProgram(program.to_string()))?;
        if !attach_compatible(prog, attach_type) {
            return Err(LoaderError::IncompatibleAttach {
                prog,
                attach: attach_type,
            });
        }
        Ok(Link {
            program: program.to_string(),
            attach_type,
            target: target.to_string(),
        })
    }

    /// Pin a map into bpffs at `path` (modelled in userspace).
    pub fn pin_map(&self, name: &str, path: &str) -> Result<(), LoaderError> {
        if !self.map_fds.contains_key(name) {
            return Err(LoaderError::UnknownMap(name.to_string()));
        }
        self.pinned
            .borrow_mut()
            .insert(name.to_string(), path.to_string());
        Ok(())
    }

    pub fn pinned_path(&self, name: &str) -> Option<String> {
        self.pinned.borrow().get(name).cloned()
    }
}

/// The loader: assigns FDs and runs the verifier model.
pub struct Loader {
    next_fd: u32,
}

impl Default for Loader {
    fn default() -> Self {
        // 0/1/2 are stdio; real bpf() FDs start above them.
        Loader { next_fd: 3 }
    }
}

impl Loader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a collection: verify every program, then assign map FDs.
    pub fn load(&mut self, obj: &BpfObject) -> Result<Collection, VerifierError> {
        Self::verify(obj)?;

        let mut map_fds = HashMap::new();
        for m in &obj.maps {
            map_fds.insert(m.name.clone(), self.next_fd);
            self.next_fd += 1;
        }
        let mut programs = HashMap::new();
        for p in &obj.programs {
            programs.insert(p.name.clone(), p.prog_type);
        }
        Ok(Collection {
            map_fds,
            programs,
            pinned: RefCell::new(HashMap::new()),
        })
    }

    /// The verifier model: gates the same structural invariants the
    /// in-kernel verifier and `bpf()` LOAD path require.
    pub fn verify(obj: &BpfObject) -> Result<(), VerifierError> {
        let lic = obj.license.trim();
        if lic.is_empty() {
            return Err(VerifierError::MissingLicense);
        }
        // GPL-compatible licenses per `license_is_gpl_compatible()`.
        let gpl_ok = matches!(
            lic,
            "GPL" | "GPL v2" | "GPL and additional rights" | "Dual BSD/GPL" | "Dual MIT/GPL"
                | "Dual MPL/GPL"
        );
        if !gpl_ok {
            return Err(VerifierError::NonGplLicense(lic.to_string()));
        }
        for p in &obj.programs {
            if p.instructions.is_empty() {
                return Err(VerifierError::EmptyProgram {
                    name: p.name.clone(),
                });
            }
            if p.instructions.len() % 8 != 0 {
                return Err(VerifierError::BadLength {
                    name: p.name.clone(),
                    len: p.instructions.len(),
                });
            }
            // The last instruction's opcode must be BPF_EXIT.
            let last = p.instructions.len() - 8;
            if p.instructions[last] != BPF_EXIT {
                return Err(VerifierError::NoExit {
                    name: p.name.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maptype_ordinals_match_uapi() {
        assert_eq!(MapType::from_u32(1), MapType::Hash);
        assert_eq!(MapType::from_u32(11), MapType::LpmTrie);
        assert_eq!(MapType::from_u32(99), MapType::Unknown(99));
    }

    #[test]
    fn dual_license_is_gpl_compatible() {
        let obj = BpfObject {
            license: "Dual BSD/GPL".into(),
            maps: vec![],
            programs: vec![],
        };
        assert!(Loader::verify(&obj).is_ok());
    }

    #[test]
    fn proprietary_license_rejected() {
        let obj = BpfObject {
            license: "Proprietary".into(),
            maps: vec![],
            programs: vec![],
        };
        assert!(matches!(
            Loader::verify(&obj),
            Err(VerifierError::NonGplLicense(_))
        ));
    }
}
