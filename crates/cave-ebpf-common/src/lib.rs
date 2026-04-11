//! Shared eBPF types and BPF map definitions.
//! Used by both kernel-space eBPF programs (via Aya) and user-space consumers.

use serde::{Deserialize, Serialize};

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
