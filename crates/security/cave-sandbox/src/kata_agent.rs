// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: gRPC surface adapted from kata-containers src/agent/* (Apache-2.0).
//! kata-agent — in-VM agent gRPC service.
//!
//! Models the wire-level gRPC method set defined in
//! `src/libs/protocols/protos/agent.proto`. We do not invoke vsock here; the
//! actual transport runs over `AF_VSOCK` on a live VM (OUT OF SCOPE).

use crate::oci_runtime_spec::Spec;
use serde::{Deserialize, Serialize};

/// Process exec request — `agent.ExecProcessRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ExecProcessRequest {
    pub container_id: String,
    pub exec_id: String,
    pub argv: Vec<String>,
    pub env: Vec<String>,
    pub cwd: String,
    pub uid: u32,
    pub gid: u32,
    pub terminal: bool,
}

/// Signal — `agent.SignalProcessRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SignalProcessRequest {
    pub container_id: String,
    pub exec_id: String,
    pub signal: u32,
}

/// Stream read — `agent.ReadStreamRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ReadStreamRequest {
    pub container_id: String,
    pub exec_id: String,
    pub len: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct WriteStreamRequest {
    pub container_id: String,
    pub exec_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct WriteStreamResponse {
    pub len: u32,
}

/// Container create — `agent.CreateContainerRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CreateContainerRequest {
    pub container_id: String,
    pub exec_id: String,
    pub spec: Spec,
    pub sandbox_pidns: bool,
}

/// `agent.StartContainerRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StartContainerRequest {
    pub container_id: String,
}

/// `agent.CreateSandboxRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CreateSandboxRequest {
    pub sandbox_id: String,
    pub hostname: String,
    pub dns: Vec<String>,
    pub storages: Vec<Storage>,
    pub kernel_modules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Storage {
    pub driver: String,
    pub source: String,
    pub mount_point: String,
    pub fstype: String,
    pub options: Vec<String>,
}

/// `agent.ListInterfacesResponse`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ListInterfacesResponse {
    pub interfaces: Vec<Interface>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Interface {
    pub name: String,
    pub hw_addr: String,
    pub mtu: u32,
    pub ip_addresses: Vec<String>,
}

/// Agent RPC method enum — closes G8 mapped count for the agent surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentRpc {
    CreateContainer(CreateContainerRequest),
    StartContainer(StartContainerRequest),
    ExecProcess(ExecProcessRequest),
    SignalProcess(SignalProcessRequest),
    ReadStdout(ReadStreamRequest),
    ReadStderr(ReadStreamRequest),
    WriteStdin(WriteStreamRequest),
    CloseStdin { container_id: String, exec_id: String },
    CreateSandbox(CreateSandboxRequest),
    DestroySandbox { sandbox_id: String },
    ListInterfaces,
    UpdateInterface(Interface),
    /// `agent.PauseContainer`.
    PauseContainer { container_id: String },
    /// `agent.ResumeContainer`.
    ResumeContainer { container_id: String },
    /// `agent.RemoveContainer`.
    RemoveContainer { container_id: String },
}

impl AgentRpc {
    /// Method name as it appears on the wire (`/grpc.AgentService/<method>`).
    pub fn method_name(&self) -> &'static str {
        match self {
            AgentRpc::CreateContainer(_) => "CreateContainer",
            AgentRpc::StartContainer(_) => "StartContainer",
            AgentRpc::ExecProcess(_) => "ExecProcess",
            AgentRpc::SignalProcess(_) => "SignalProcess",
            AgentRpc::ReadStdout(_) => "ReadStdout",
            AgentRpc::ReadStderr(_) => "ReadStderr",
            AgentRpc::WriteStdin(_) => "WriteStdin",
            AgentRpc::CloseStdin { .. } => "CloseStdin",
            AgentRpc::CreateSandbox(_) => "CreateSandbox",
            AgentRpc::DestroySandbox { .. } => "DestroySandbox",
            AgentRpc::ListInterfaces => "ListInterfaces",
            AgentRpc::UpdateInterface(_) => "UpdateInterface",
            AgentRpc::PauseContainer { .. } => "PauseContainer",
            AgentRpc::ResumeContainer { .. } => "ResumeContainer",
            AgentRpc::RemoveContainer { .. } => "RemoveContainer",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_names_pascalcase() {
        let r = AgentRpc::ListInterfaces;
        assert_eq!(r.method_name(), "ListInterfaces");
        let r = AgentRpc::DestroySandbox { sandbox_id: "x".into() };
        assert_eq!(r.method_name(), "DestroySandbox");
    }

    #[test]
    fn create_container_round_trips() {
        let req = CreateContainerRequest {
            container_id: "c1".into(),
            exec_id: "e1".into(),
            spec: Spec::minimal_shell("/r"),
            sandbox_pidns: true,
        };
        let j = serde_json::to_string(&req).unwrap();
        let back: CreateContainerRequest = serde_json::from_str(&j).unwrap();
        assert_eq!(back.container_id, "c1");
        assert!(back.sandbox_pidns);
    }

    #[test]
    fn exec_default_no_terminal() {
        let e = ExecProcessRequest::default();
        assert!(!e.terminal);
    }

    #[test]
    fn signal_serializes() {
        let s = SignalProcessRequest { container_id: "c".into(), exec_id: "e".into(), signal: 15 };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["signal"], 15);
    }

    #[test]
    fn write_stream_response_len() {
        let r = WriteStreamResponse { len: 42 };
        assert_eq!(r.len, 42);
    }

    #[test]
    fn list_interfaces_method() {
        assert_eq!(AgentRpc::ListInterfaces.method_name(), "ListInterfaces");
    }

    #[test]
    fn storage_serializable() {
        let s = Storage {
            driver: "blk".into(),
            source: "/dev/vda".into(),
            mount_point: "/".into(),
            fstype: "ext4".into(),
            options: vec!["ro".into()],
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"driver\":\"blk\""));
    }

    #[test]
    fn interface_with_ipv6() {
        let i = Interface {
            name: "eth0".into(),
            hw_addr: "00:11:22:33:44:55".into(),
            mtu: 1500,
            ip_addresses: vec!["10.0.0.5/24".into(), "fe80::1/64".into()],
        };
        assert_eq!(i.ip_addresses.len(), 2);
    }

    #[test]
    fn rpc_pause_resume_remove() {
        let r = AgentRpc::PauseContainer { container_id: "c".into() };
        assert_eq!(r.method_name(), "PauseContainer");
        let r = AgentRpc::ResumeContainer { container_id: "c".into() };
        assert_eq!(r.method_name(), "ResumeContainer");
        let r = AgentRpc::RemoveContainer { container_id: "c".into() };
        assert_eq!(r.method_name(), "RemoveContainer");
    }
}
