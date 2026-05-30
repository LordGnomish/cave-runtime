// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD cycle — kprobe / uprobe / tracepoint attach registry
// (userspace model of grafana/beyla's probe attachment in
// pkg/internal/ebpf). Models kernel-symbol resolution (kallsyms),
// uprobe symbol→offset resolution against a binary's symbol table, and
// the link lifecycle (attach → LinkId → detach).

use cave_ebpf_common::probe::{ProbeError, ProbeRegistry};

fn registry() -> ProbeRegistry {
    let mut r = ProbeRegistry::new();
    r.add_kernel_symbol("tcp_sendmsg");
    r.add_kernel_symbol("tcp_recvmsg");
    // libssl.so exports SSL_read at offset 0x3a100.
    r.add_binary_symbol("/lib/libssl.so", "SSL_read", 0x3a100);
    r.add_binary_symbol("/lib/libssl.so", "SSL_write", 0x3a200);
    r
}

#[test]
fn test_attach_and_detach_kprobe() {
    let mut r = registry();
    let id = r.attach_kprobe("beyla_tcp_sendmsg", "tcp_sendmsg", false).unwrap();
    assert_eq!(r.active(), 1);
    r.detach(id).unwrap();
    assert_eq!(r.active(), 0);
}

#[test]
fn test_kprobe_unknown_symbol_rejected() {
    let mut r = registry();
    assert_eq!(
        r.attach_kprobe("p", "no_such_fn", false).err(),
        Some(ProbeError::UnknownSymbol("no_such_fn".into()))
    );
}

#[test]
fn test_kretprobe_flag_recorded() {
    let mut r = registry();
    let id = r.attach_kprobe("p", "tcp_recvmsg", true).unwrap();
    let att = r.link(id).unwrap();
    assert!(att.is_return);
    assert_eq!(att.target, "tcp_recvmsg");
}

#[test]
fn test_tracepoint_attach_and_format() {
    let mut r = registry();
    let id = r
        .attach_tracepoint("p", "syscalls", "sys_enter_openat")
        .unwrap();
    assert_eq!(r.link(id).unwrap().target, "syscalls:sys_enter_openat");
}

#[test]
fn test_tracepoint_empty_name_rejected() {
    let mut r = registry();
    assert_eq!(
        r.attach_tracepoint("p", "syscalls", "").err(),
        Some(ProbeError::BadTracepoint)
    );
}

#[test]
fn test_uprobe_resolves_symbol_to_offset() {
    let mut r = registry();
    let id = r
        .attach_uprobe("p", "/lib/libssl.so", Some("SSL_read"), None, false)
        .unwrap();
    let att = r.link(id).unwrap();
    assert_eq!(att.offset, Some(0x3a100));
}

#[test]
fn test_uprobe_unknown_symbol_rejected() {
    let mut r = registry();
    assert_eq!(
        r.attach_uprobe("p", "/lib/libssl.so", Some("SSL_nope"), None, false)
            .err(),
        Some(ProbeError::UnknownSymbol("SSL_nope".into()))
    );
}

#[test]
fn test_uprobe_explicit_offset_without_symbol() {
    let mut r = registry();
    let id = r
        .attach_uprobe("p", "/bin/app", None, Some(0x1234), true)
        .unwrap();
    let att = r.link(id).unwrap();
    assert_eq!(att.offset, Some(0x1234));
    assert!(att.is_return);
}

#[test]
fn test_uprobe_requires_symbol_or_offset() {
    let mut r = registry();
    assert_eq!(
        r.attach_uprobe("p", "/bin/app", None, None, false).err(),
        Some(ProbeError::MissingLocation)
    );
}

#[test]
fn test_detach_unknown_link_is_error() {
    let mut r = registry();
    assert_eq!(r.detach(999), Err(ProbeError::NotFound));
}

#[test]
fn test_links_for_program_filters() {
    let mut r = registry();
    r.attach_kprobe("prog_a", "tcp_sendmsg", false).unwrap();
    r.attach_kprobe("prog_a", "tcp_recvmsg", false).unwrap();
    r.attach_tracepoint("prog_b", "syscalls", "sys_exit_read")
        .unwrap();
    assert_eq!(r.links_for_program("prog_a").len(), 2);
    assert_eq!(r.links_for_program("prog_b").len(), 1);
}
