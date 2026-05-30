// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared eBPF types and BPF map definitions, plus a userspace-approximation
//! port of the grafana/beyla eBPF auto-instrumentation pipeline.
//!
//! ## Layout
//!
//! The original crate held the Cave-internal eBPF event structs
//! ([`SyscallEvent`], [`NetEvent`], [`ResourceEvent`]). It now also hosts a
//! userspace port of [Beyla](https://github.com/grafana/beyla) (Apache-2.0)
//! — the userspace half of Beyla's eBPF auto-instrumentation:
//!
//! * [`loader`]   — eBPF object load path (spec validation, GPL gate).
//! * [`map`]      — BPF map abstraction (hash / array / LRU / ringbuf).
//! * [`ringbuf`]  — ring-buffer record reader (reserve / commit / consume).
//! * [`probe`]    — kprobe / uprobe / tracepoint attach registry.
//! * [`discover`] — HTTP / gRPC / SQL protocol detection from raw buffers.
//! * [`process`]  — process discovery + exec/exit watcher.
//! * [`otlp`]     — request span → OTLP trace export.
//!
//! Kernel-side eBPF C programs and the `bpf(2)` syscall FFI are documented
//! userspace approximations, tracked honestly in `parity.manifest.toml`.
//! The Cilium parity port lives in `cave-net`, not here.

use serde::{Deserialize, Serialize};

pub mod loader;

/// Syscall audit event from eBPF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyscallEvent {
    pub pid: u32,
    pub tid: u32,
    pub syscall_nr: u32,
    pub comm: [u8; 16],
    pub timestamp_ns: u64,
}

/// Network event from eBPF net-tap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetEvent {
    pub src_addr: u32,
    pub dst_addr: u32,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8,
    pub bytes: u64,
    pub timestamp_ns: u64,
}

/// Resource usage event from eBPF resource-meter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEvent {
    pub cgroup_id: u64,
    pub cpu_ns: u64,
    pub mem_bytes: u64,
    pub io_bytes: u64,
    pub timestamp_ns: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_syscall_event() -> SyscallEvent {
        let mut comm = [0u8; 16];
        let name = b"nginx";
        comm[..name.len()].copy_from_slice(name);
        SyscallEvent {
            pid: 1234,
            tid: 1235,
            syscall_nr: 59,
            comm,
            timestamp_ns: 1_000_000_000,
        }
    }

    #[test]
    fn test_syscall_event_serde() {
        let event = make_syscall_event();
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: SyscallEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.pid, event.pid);
        assert_eq!(decoded.tid, event.tid);
        assert_eq!(decoded.syscall_nr, event.syscall_nr);
        assert_eq!(decoded.comm, event.comm);
        assert_eq!(decoded.timestamp_ns, event.timestamp_ns);
    }

    #[test]
    fn test_net_event_serde() {
        let event = NetEvent {
            src_addr: 0x7F000001, // 127.0.0.1
            dst_addr: 0x08080808, // 8.8.8.8
            src_port: 54321,
            dst_port: 443,
            protocol: 6, // TCP
            bytes: 4096,
            timestamp_ns: 2_000_000_000,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: NetEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.src_addr, event.src_addr);
        assert_eq!(decoded.dst_port, event.dst_port);
        assert_eq!(decoded.bytes, event.bytes);
    }

    #[test]
    fn test_resource_event_serde() {
        let event = ResourceEvent {
            cgroup_id: 42,
            cpu_ns: 100_000,
            mem_bytes: 1024 * 1024,
            io_bytes: 512,
            timestamp_ns: 3_000_000_000,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: ResourceEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.cgroup_id, event.cgroup_id);
        assert_eq!(decoded.cpu_ns, event.cpu_ns);
        assert_eq!(decoded.mem_bytes, event.mem_bytes);
        assert_eq!(decoded.io_bytes, event.io_bytes);
        assert_eq!(decoded.timestamp_ns, event.timestamp_ns);
    }

    #[test]
    fn test_syscall_event_debug() {
        let event = make_syscall_event();
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("SyscallEvent"));
    }

    #[test]
    fn test_net_event_fields() {
        let event = NetEvent {
            src_addr: 0xC0A80001, // 192.168.0.1
            dst_addr: 0xC0A80002, // 192.168.0.2
            src_port: 8080,
            dst_port: 80,
            protocol: 6,
            bytes: 256,
            timestamp_ns: 999,
        };
        assert_eq!(event.src_port, 8080);
        assert_eq!(event.dst_port, 80);
        assert_eq!(event.protocol, 6);
        assert_eq!(event.bytes, 256);
    }
}
