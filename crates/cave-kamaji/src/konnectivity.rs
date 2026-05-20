// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Konnectivity controller — tenant→host networking tunnel.
//!
//! Upstream reference (Kamaji v1.0.0):
//!   internal/resources/konnectivity/*.go
//!
//! Konnectivity sits between the tenant control plane's api-server and
//! the management cluster's networking fabric: an agent runs alongside
//! the kubelet on each tenant node and a server runs on the management
//! cluster, multiplexing every node's connection over a single secure
//! tunnel. The Cave port models the server-side configuration; the agent
//! pod manifest is produced via [`Konnectivity::agent_manifest_args`].

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KonnectivityMode {
    /// gRPC streams over TCP — the upstream default.
    Grpc,
    /// HTTP CONNECT — used in environments where gRPC is blocked.
    HttpConnect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Konnectivity {
    pub mode: KonnectivityMode,
    /// Server port — defaults to 8132 (gRPC) or 8133 (HTTP CONNECT).
    pub server_port: u16,
    /// Agent identifier — usually the tenant control plane name.
    pub agent_id: String,
    /// Shared secret the agent presents during handshake. None until
    /// the controller mints a token.
    pub agent_token: Option<String>,
    /// Server-side advertise hostname (FQDN or IP).
    pub server_host: String,
}

impl Konnectivity {
    pub fn new(mode: KonnectivityMode) -> Self {
        let server_port = match mode {
            KonnectivityMode::Grpc => 8132,
            KonnectivityMode::HttpConnect => 8133,
        };
        Self {
            mode,
            server_port,
            agent_id: String::new(),
            agent_token: None,
            server_host: "konnectivity.svc".to_string(),
        }
    }

    pub fn with_agent_token(&mut self, token: &str) -> &mut Self {
        self.agent_token = Some(token.to_string());
        self
    }

    pub fn with_server_host(&mut self, host: &str) -> &mut Self {
        self.server_host = host.to_string();
        self
    }

    /// Render the agent-side command-line args. Used by the pod-mgmt
    /// module when scheduling the agent daemonset.
    pub fn agent_manifest_args(&self) -> Vec<String> {
        let mode = match self.mode {
            KonnectivityMode::Grpc => "grpc",
            KonnectivityMode::HttpConnect => "http-connect",
        };
        let mut args = vec![
            format!("--proxy-server-host={}", self.server_host),
            format!("--proxy-server-port={}", self.server_port),
            format!("--mode={mode}"),
        ];
        if !self.agent_id.is_empty() {
            args.push(format!("--agent-identifiers=ipv4={}", self.agent_id));
        }
        if let Some(token) = &self.agent_token {
            args.push(format!("--service-account-token-path=/var/run/secrets/{token}"));
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_manifest_args_carry_mode() {
        let k = Konnectivity::new(KonnectivityMode::Grpc);
        let args = k.agent_manifest_args();
        assert!(args.iter().any(|a| a == "--mode=grpc"));
    }

    #[test]
    fn agent_manifest_args_include_agent_identifier_when_set() {
        let mut k = Konnectivity::new(KonnectivityMode::Grpc);
        k.agent_id = "10.0.0.5".into();
        let args = k.agent_manifest_args();
        assert!(args
            .iter()
            .any(|a| a.contains("--agent-identifiers=ipv4=10.0.0.5")));
    }
}
