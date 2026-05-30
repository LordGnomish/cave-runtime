// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — eBPF program loader (userspace approximation of
// grafana/beyla pkg/internal/ebpf/tracer.go + cilium/ebpf CollectionSpec
// load path). Models the userspace half of loading an eBPF object: spec
// validation, map resolution, and the kernel verifier's GPL-license gate
// for GPL-only helpers. The actual bpf(2) syscall FFI is a documented
// approximation (see loader.rs module docs).

use cave_ebpf_common::loader::{
    CollectionSpec, LoaderError, MapSpec, MapType, ProgramSpec, ProgramType,
};

fn tracepoint_spec() -> ProgramSpec {
    ProgramSpec {
        name: "beyla_kprobe_tcp_sendmsg".into(),
        prog_type: ProgramType::Kprobe,
        section: "kprobe/tcp_sendmsg".into(),
        attach_to: "tcp_sendmsg".into(),
        license: "GPL".into(),
        map_refs: vec!["events".into()],
    }
}

fn events_map() -> MapSpec {
    MapSpec {
        name: "events".into(),
        map_type: MapType::RingBuf,
        key_size: 0,
        value_size: 0,
        max_entries: 1 << 20,
    }
}

#[test]
fn test_load_collection_resolves_program_and_maps() {
    let spec = CollectionSpec {
        programs: vec![tracepoint_spec()],
        maps: vec![events_map()],
    };
    let coll = spec.load().expect("load");
    assert_eq!(coll.program_count(), 1);
    assert_eq!(coll.map_count(), 1);
    assert!(coll.program("beyla_kprobe_tcp_sendmsg").is_some());
    assert!(coll.map("events").is_some());
}

#[test]
fn test_gpl_only_helper_requires_gpl_license() {
    // kprobe programs call bpf_probe_read_kernel (GPL-only). A non-GPL
    // license must be rejected, mirroring the kernel verifier gate.
    let mut p = tracepoint_spec();
    p.license = "Proprietary".into();
    let spec = CollectionSpec {
        programs: vec![p],
        maps: vec![events_map()],
    };
    match spec.load() {
        Err(LoaderError::GplRequired { program }) => {
            assert_eq!(program, "beyla_kprobe_tcp_sendmsg");
        }
        other => panic!("expected GplRequired, got {other:?}"),
    }
}

#[test]
fn test_dual_bsd_gpl_license_accepted() {
    let mut p = tracepoint_spec();
    p.license = "Dual BSD/GPL".into();
    let spec = CollectionSpec {
        programs: vec![p],
        maps: vec![events_map()],
    };
    assert!(spec.load().is_ok());
}

#[test]
fn test_unresolved_map_ref_is_error() {
    let mut p = tracepoint_spec();
    p.map_refs = vec!["nonexistent".into()];
    let spec = CollectionSpec {
        programs: vec![p],
        maps: vec![events_map()],
    };
    match spec.load() {
        Err(LoaderError::UnresolvedMap { program, map }) => {
            assert_eq!(program, "beyla_kprobe_tcp_sendmsg");
            assert_eq!(map, "nonexistent");
        }
        other => panic!("expected UnresolvedMap, got {other:?}"),
    }
}

#[test]
fn test_duplicate_program_name_is_error() {
    let spec = CollectionSpec {
        programs: vec![tracepoint_spec(), tracepoint_spec()],
        maps: vec![events_map()],
    };
    match spec.load() {
        Err(LoaderError::DuplicateProgram { name }) => {
            assert_eq!(name, "beyla_kprobe_tcp_sendmsg");
        }
        other => panic!("expected DuplicateProgram, got {other:?}"),
    }
}

#[test]
fn test_zero_max_entries_rejected() {
    let mut m = events_map();
    m.max_entries = 0;
    let spec = CollectionSpec {
        programs: vec![tracepoint_spec()],
        maps: vec![m],
    };
    match spec.load() {
        Err(LoaderError::InvalidMap { map, .. }) => assert_eq!(map, "events"),
        other => panic!("expected InvalidMap, got {other:?}"),
    }
}

#[test]
fn test_uprobe_does_not_require_gpl() {
    // Uprobes on userspace symbols do not necessarily call GPL-only
    // helpers; Beyla's Go/HTTP uprobes ship under Apache-2.0-compatible
    // licensing, so the gate only applies to kernel-probe types.
    let p = ProgramSpec {
        name: "uprobe_go_runtime_newproc".into(),
        prog_type: ProgramType::Uprobe,
        section: "uprobe/runtime.newproc1".into(),
        attach_to: "runtime.newproc1".into(),
        license: "Apache-2.0".into(),
        map_refs: vec![],
    };
    let spec = CollectionSpec {
        programs: vec![p],
        maps: vec![],
    };
    assert!(spec.load().is_ok());
}
