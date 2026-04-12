//! Event field definitions and EventContext for Falco condition evaluation.
//!
//! Covers all syscall, process, file descriptor, container, and k8s fields
//! that Falco supports.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Field name constants
// ---------------------------------------------------------------------------

/// Event fields
pub mod field {
    // Event
    pub const EVT_TYPE: &str = "evt.type";
    pub const EVT_DIR: &str = "evt.dir";
    pub const EVT_ARGS: &str = "evt.args";
    pub const EVT_ARG: &str = "evt.arg";   // evt.arg.0, evt.arg.1 ...
    pub const EVT_NUM: &str = "evt.num";
    pub const EVT_TIME: &str = "evt.time";
    pub const EVT_RAWTIME: &str = "evt.rawtime";
    pub const EVT_CPU: &str = "evt.cpu";
    pub const EVT_LATENCY: &str = "evt.latency";
    pub const EVT_CATEGORY: &str = "evt.category";

    // Process
    pub const PROC_NAME: &str = "proc.name";
    pub const PROC_PNAME: &str = "proc.pname";
    pub const PROC_CMDLINE: &str = "proc.cmdline";
    pub const PROC_EXEPATH: &str = "proc.exepath";
    pub const PROC_CWD: &str = "proc.cwd";
    pub const PROC_PID: &str = "proc.pid";
    pub const PROC_PPID: &str = "proc.ppid";
    pub const PROC_SID: &str = "proc.sid";
    pub const PROC_TTYNAME: &str = "proc.ttyname";
    pub const PROC_ARGS: &str = "proc.args";
    pub const PROC_ENV: &str = "proc.env";

    // User / Group
    pub const USER_NAME: &str = "user.name";
    pub const USER_UID: &str = "user.uid";
    pub const USER_HOMEDIR: &str = "user.homedir";
    pub const USER_SHELL: &str = "user.shell";
    pub const GROUP_NAME: &str = "group.name";
    pub const GROUP_GID: &str = "group.gid";

    // File descriptor
    pub const FD_NAME: &str = "fd.name";
    pub const FD_DIRECTORY: &str = "fd.directory";
    pub const FD_FILENAME: &str = "fd.filename";
    pub const FD_TYPE: &str = "fd.type";
    pub const FD_TYPECHAR: &str = "fd.typechar";
    pub const FD_SIP: &str = "fd.sip";
    pub const FD_SPORT: &str = "fd.sport";
    pub const FD_DIP: &str = "fd.dip";
    pub const FD_DPORT: &str = "fd.dport";
    pub const FD_PROTO: &str = "fd.proto";
    pub const FD_SIP_NAME: &str = "fd.sip.name";
    pub const FD_DIP_NAME: &str = "fd.dip.name";
    pub const FD_L4PROTO: &str = "fd.l4proto";

    // Container
    pub const CONTAINER_ID: &str = "container.id";
    pub const CONTAINER_NAME: &str = "container.name";
    pub const CONTAINER_IMAGE: &str = "container.image";
    pub const CONTAINER_IMAGE_REPOSITORY: &str = "container.image.repository";
    pub const CONTAINER_IMAGE_TAG: &str = "container.image.tag";
    pub const CONTAINER_IMAGE_DIGEST: &str = "container.image.digest";
    pub const CONTAINER_TYPE: &str = "container.type";
    pub const CONTAINER_PRIVILEGED: &str = "container.privileged";
    pub const CONTAINER_MOUNTS: &str = "container.mounts";
    pub const CONTAINER_ENV: &str = "container.env";

    // Kubernetes
    pub const K8S_NS_NAME: &str = "k8s.ns.name";
    pub const K8S_POD_NAME: &str = "k8s.pod.name";
    pub const K8S_POD_ID: &str = "k8s.pod.id";
    pub const K8S_POD_LABEL: &str = "k8s.pod.label";
    pub const K8S_DEPLOYMENT_NAME: &str = "k8s.deployment.name";
    pub const K8S_REPLICASET_NAME: &str = "k8s.replicaset.name";
    pub const K8S_SERVICE_NAME: &str = "k8s.service.name";
    pub const K8S_NODE_NAME: &str = "k8s.node.name";
}

// ---------------------------------------------------------------------------
// Event direction
// ---------------------------------------------------------------------------

/// Syscall entry/exit direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventDir {
    /// Syscall entry  `<`
    Entry,
    /// Syscall exit   `>`
    Exit,
}

// ---------------------------------------------------------------------------
// Event source
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    Syscall,
    K8sAudit,
    AwsCloudtrail,
    GcpAudit,
}

// ---------------------------------------------------------------------------
// Generic event context
// ---------------------------------------------------------------------------

/// All fields for one event, as plain strings.
///
/// Fields are resolved from whatever event representation arrives (syscall
/// eBPF event, k8s audit log, cloud trail record, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventContext {
    pub source: Option<EventSource>,
    pub fields: HashMap<String, String>,
}

impl EventContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.fields.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }

    /// Convenience: create a syscall execve event context.
    pub fn syscall_execve(
        proc_name: &str,
        cmdline: &str,
        user: &str,
        uid: u32,
        container_id: &str,
    ) -> Self {
        let mut ctx = Self::new();
        ctx.source = Some(EventSource::Syscall);
        ctx.set(field::EVT_TYPE, "execve");
        ctx.set(field::EVT_DIR, ">");
        ctx.set(field::PROC_NAME, proc_name);
        ctx.set(field::PROC_CMDLINE, cmdline);
        ctx.set(field::USER_NAME, user);
        ctx.set(field::USER_UID, uid.to_string());
        if !container_id.is_empty() {
            ctx.set(field::CONTAINER_ID, container_id);
        } else {
            ctx.set(field::CONTAINER_ID, "host");
        }
        ctx
    }

    /// Convenience: create a k8s audit event context.
    pub fn k8s_audit(
        verb: &str,
        resource: &str,
        namespace: &str,
        user: &str,
    ) -> Self {
        let mut ctx = Self::new();
        ctx.source = Some(EventSource::K8sAudit);
        ctx.set(field::EVT_TYPE, verb);
        ctx.set(field::EVT_DIR, ">");
        ctx.set("ka.verb", verb);
        ctx.set("ka.target.resource", resource);
        ctx.set(field::K8S_NS_NAME, namespace);
        ctx.set(field::USER_NAME, user);
        ctx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_context_get_set() {
        let mut ctx = EventContext::new();
        ctx.set(field::EVT_TYPE, "open");
        assert_eq!(ctx.get(field::EVT_TYPE), "open");
        assert_eq!(ctx.get("nonexistent"), "");
    }

    #[test]
    fn syscall_execve_context() {
        let ctx = EventContext::syscall_execve("bash", "bash -i", "root", 0, "abc123");
        assert_eq!(ctx.get(field::EVT_TYPE), "execve");
        assert_eq!(ctx.get(field::PROC_NAME), "bash");
        assert_eq!(ctx.get(field::CONTAINER_ID), "abc123");
    }
}
