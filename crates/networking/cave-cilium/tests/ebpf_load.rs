// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Headline acceptance test for the eBPF object loader (RED→GREEN).
//!
//! Ports cilium's load path: a relocatable BPF ELF (`bpf_xdp.o`-style) is
//! parsed into a CollectionSpec analog, the verifier model gates it, the
//! loader assigns map FDs, and a program is attached to an interface and
//! pinned into the bpffs. The ELF here is hand-built byte-for-byte so the
//! parser is exercised against a real ET_REL / EM_BPF object — not a mock
//! struct.

use cave_cilium::ebpf::{
    AttachType, BpfObject, ElfObject, Loader, MapType, ProgramType, VerifierError,
};

/// Little-endian byte sink used by the ELF builder below.
#[derive(Default)]
struct Buf(Vec<u8>);
impl Buf {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn pad_to(&mut self, n: usize) {
        while self.0.len() < n {
            self.0.push(0);
        }
    }
}

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;

/// One legacy `bpf_map_def` (20 bytes): type, key, value, max_entries, flags.
fn map_def(map_type: u32, key: u32, value: u32, max: u32, flags: u32) -> Vec<u8> {
    let mut b = Buf::default();
    b.u32(map_type);
    b.u32(key);
    b.u32(value);
    b.u32(max);
    b.u32(flags);
    b.0
}

/// BPF instruction (8 bytes): opcode, dst|src nibble, off, imm.
fn insn(op: u8, dst: u8, src: u8, off: i16, imm: i32) -> Vec<u8> {
    let mut b = Buf::default();
    b.u8(op);
    b.u8((src << 4) | (dst & 0x0f));
    b.u16(off as u16);
    b.u32(imm as u32);
    b.0
}

struct SectionIn {
    name: &'static str,
    sh_type: u32,
    data: Vec<u8>,
    link: u32,
    info: u32,
    entsize: u64,
}

/// Assemble a minimal but valid ET_REL / EM_BPF ELF64 (little-endian).
fn build_elf(sections: &[SectionIn]) -> Vec<u8> {
    // Build .shstrtab: section name strings. Index 0 = empty name.
    let mut shstr = vec![0u8];
    let mut name_off = Vec::new();
    for s in sections {
        name_off.push(shstr.len() as u32);
        shstr.extend_from_slice(s.name.as_bytes());
        shstr.push(0);
    }
    let shstr_name_off = shstr.len() as u32;
    shstr.extend_from_slice(b".shstrtab\0");

    // Section header table: NULL + each input section + .shstrtab.
    let total = sections.len() + 2; // null + inputs + shstrtab
    let ehsize = 64usize;
    let shentsize = 64usize;

    // Lay out section data after the ELF header.
    let mut data_blob = Buf::default();
    let mut offsets = Vec::new();
    let mut sizes = Vec::new();
    for s in sections {
        offsets.push(ehsize as u64 + data_blob.0.len() as u64);
        sizes.push(s.data.len() as u64);
        data_blob.bytes(&s.data);
    }
    // .shstrtab data
    let shstr_off = ehsize as u64 + data_blob.0.len() as u64;
    let shstr_size = shstr.len() as u64;
    data_blob.bytes(&shstr);

    let shoff = ehsize as u64 + data_blob.0.len() as u64;

    let mut out = Buf::default();
    // e_ident
    out.bytes(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    out.bytes(&[0, 0, 0, 0, 0, 0, 0, 0]);
    out.u16(1); // e_type ET_REL
    out.u16(247); // e_machine EM_BPF
    out.u32(1); // e_version
    out.u64(0); // e_entry
    out.u64(0); // e_phoff
    out.u64(shoff); // e_shoff
    out.u32(0); // e_flags
    out.u16(ehsize as u16);
    out.u16(0); // e_phentsize
    out.u16(0); // e_phnum
    out.u16(shentsize as u16);
    out.u16(total as u16); // e_shnum
    out.u16((total - 1) as u16); // e_shstrndx (last = .shstrtab)
    out.pad_to(ehsize);

    // section data
    out.bytes(&data_blob.0);

    // section headers
    // NULL
    for _ in 0..64 {
        out.u8(0);
    }
    for (i, s) in sections.iter().enumerate() {
        out.u32(name_off[i]); // sh_name
        out.u32(s.sh_type);
        out.u64(0); // flags
        out.u64(0); // addr
        out.u64(offsets[i]);
        out.u64(sizes[i]);
        out.u32(s.link);
        out.u32(s.info);
        out.u64(8); // addralign
        out.u64(s.entsize);
    }
    // .shstrtab header
    out.u32(shstr_name_off);
    out.u32(SHT_STRTAB);
    out.u64(0);
    out.u64(0);
    out.u64(shstr_off);
    out.u64(shstr_size);
    out.u32(0);
    out.u32(0);
    out.u64(1);
    out.u64(0);

    out.0
}

/// Build an Elf64_Sym (24 bytes).
fn sym(name_off: u32, info: u8, shndx: u16, value: u64) -> Vec<u8> {
    let mut b = Buf::default();
    b.u32(name_off);
    b.u8(info);
    b.u8(0);
    b.u16(shndx);
    b.u64(value);
    b.u64(0);
    b.0
}

/// A complete xdp object: license=GPL, one HASH map "cilium_lxc",
/// one xdp program "handle_xdp" that is `r0 = 0; exit`.
fn xdp_object() -> Vec<u8> {
    // .strtab for symbol names. idx0 empty.
    let mut strtab = vec![0u8];
    let lxc_off = strtab.len() as u32;
    strtab.extend_from_slice(b"cilium_lxc\0");
    let prog_off = strtab.len() as u32;
    strtab.extend_from_slice(b"handle_xdp\0");

    // section order: [license=1, maps=2, xdp=3, .symtab=4, .strtab=5]
    let maps_shndx = 2u16;
    let xdp_shndx = 3u16;
    let strtab_shndx = 5u32;

    // STT_OBJECT|GLOBAL = (1<<4)|1 = 0x11 ; STT_FUNC|GLOBAL = (1<<4)|2 = 0x12
    let mut symtab = Vec::new();
    symtab.extend_from_slice(&sym(0, 0, 0, 0)); // null sym
    symtab.extend_from_slice(&sym(lxc_off, 0x11, maps_shndx, 0));
    symtab.extend_from_slice(&sym(prog_off, 0x12, xdp_shndx, 0));

    let mut prog = Vec::new();
    prog.extend_from_slice(&insn(0xb7, 0, 0, 0, 0)); // r0 = 0
    prog.extend_from_slice(&insn(0x95, 0, 0, 0, 0)); // exit

    build_elf(&[
        SectionIn {
            name: "license",
            sh_type: SHT_PROGBITS,
            data: b"GPL\0".to_vec(),
            link: 0,
            info: 0,
            entsize: 0,
        },
        SectionIn {
            name: "maps",
            sh_type: SHT_PROGBITS,
            data: map_def(1 /*HASH*/, 4, 8, 1024, 0),
            link: 0,
            info: 0,
            entsize: 0,
        },
        SectionIn {
            name: "xdp/handle",
            sh_type: SHT_PROGBITS,
            data: prog,
            link: 0,
            info: 0,
            entsize: 0,
        },
        SectionIn {
            name: ".symtab",
            sh_type: SHT_SYMTAB,
            data: symtab,
            link: strtab_shndx,
            info: 1,
            entsize: 24,
        },
        SectionIn {
            name: ".strtab",
            sh_type: SHT_STRTAB,
            data: strtab,
            link: 0,
            info: 0,
            entsize: 0,
        },
    ])
}

#[test]
fn parses_elf_header_sections_and_license() {
    let bytes = xdp_object();
    let elf = ElfObject::parse(&bytes).expect("valid ET_REL/EM_BPF ELF");
    assert_eq!(elf.license(), "GPL", "license section content");
    assert!(
        elf.section("maps").is_some(),
        "maps section must be discoverable by name"
    );
    assert!(elf.section("xdp/handle").is_some());
}

#[test]
fn rejects_non_bpf_machine() {
    let mut bytes = xdp_object();
    // e_machine is at offset 18 (after 16 e_ident + 2 e_type) — set to EM_X86_64 (62).
    bytes[18] = 62;
    bytes[19] = 0;
    assert!(
        ElfObject::parse(&bytes).is_err(),
        "non EM_BPF object must be rejected"
    );
}

#[test]
fn extracts_maps_and_programs_from_symtab() {
    let elf = ElfObject::parse(&xdp_object()).unwrap();
    let obj = BpfObject::from_elf(&elf).expect("collection spec");

    assert_eq!(obj.maps.len(), 1);
    let m = &obj.maps[0];
    assert_eq!(m.name, "cilium_lxc");
    assert_eq!(m.map_type, MapType::Hash);
    assert_eq!(m.key_size, 4);
    assert_eq!(m.value_size, 8);
    assert_eq!(m.max_entries, 1024);

    assert_eq!(obj.programs.len(), 1);
    let p = &obj.programs[0];
    assert_eq!(p.name, "handle_xdp");
    assert_eq!(p.prog_type, ProgramType::Xdp);
    // r0=0; exit  => 16 bytes
    assert_eq!(p.instructions.len(), 16);
}

#[test]
fn program_type_is_derived_from_section_prefix() {
    assert_eq!(ProgramType::from_section("xdp"), Some(ProgramType::Xdp));
    assert_eq!(ProgramType::from_section("xdp/devmap"), Some(ProgramType::Xdp));
    assert_eq!(
        ProgramType::from_section("tc"),
        Some(ProgramType::SchedCls)
    );
    assert_eq!(
        ProgramType::from_section("classifier"),
        Some(ProgramType::SchedCls)
    );
    assert_eq!(
        ProgramType::from_section("cgroup/connect4"),
        Some(ProgramType::CGroupSockAddr)
    );
    assert_eq!(
        ProgramType::from_section("sockops"),
        Some(ProgramType::SockOps)
    );
    assert_eq!(ProgramType::from_section("not_a_prog"), None);
}

#[test]
fn loader_verifies_assigns_fds_and_attaches() {
    let elf = ElfObject::parse(&xdp_object()).unwrap();
    let obj = BpfObject::from_elf(&elf).unwrap();

    let mut loader = Loader::new();
    let coll = loader.load(&obj).expect("verifier passes for GPL r0=0;exit");

    // Map FDs are assigned monotonically starting after stdio (>=3).
    let fd = coll.map_fd("cilium_lxc").expect("map fd assigned");
    assert!(fd >= 3, "fd must avoid stdio range, got {}", fd);

    // Attach the xdp program to an interface.
    let link = coll
        .attach("handle_xdp", AttachType::Xdp, "eth0")
        .expect("xdp attaches to a netdev");
    assert_eq!(link.attach_type, AttachType::Xdp);
    assert_eq!(link.target, "eth0");

    // Attaching xdp as a tc hook is a type mismatch.
    assert!(
        coll.attach("handle_xdp", AttachType::TcIngress, "eth0")
            .is_err(),
        "xdp program cannot attach to a tc hook"
    );

    // Pinning persists the map under bpffs.
    coll.pin_map("cilium_lxc", "/sys/fs/bpf/tc/globals/cilium_lxc")
        .unwrap();
    assert_eq!(
        coll.pinned_path("cilium_lxc").as_deref(),
        Some("/sys/fs/bpf/tc/globals/cilium_lxc")
    );
}

#[test]
fn verifier_rejects_missing_license_and_no_exit() {
    // No-license program: rebuild with empty license section.
    let elf = ElfObject::parse(&xdp_object()).unwrap();
    let mut obj = BpfObject::from_elf(&elf).unwrap();
    obj.license.clear();
    let mut loader = Loader::new();
    assert!(matches!(
        loader.load(&obj),
        Err(VerifierError::MissingLicense)
    ));

    // Program whose last instruction is not BPF_EXIT.
    let elf = ElfObject::parse(&xdp_object()).unwrap();
    let mut obj = BpfObject::from_elf(&elf).unwrap();
    // Truncate the trailing exit instruction.
    obj.programs[0].instructions.truncate(8);
    let mut loader = Loader::new();
    assert!(matches!(
        loader.load(&obj),
        Err(VerifierError::NoExit { .. })
    ));
}
