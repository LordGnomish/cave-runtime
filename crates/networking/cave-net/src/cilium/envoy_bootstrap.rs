// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Envoy bootstrap config generator.
//!
//! Mirrors `pkg/envoy/cell.go` and `pkg/envoy/embedded_envoy.go`. The
//! cilium-agent supervises an embedded envoy process; this module
//! emits the bootstrap JSON the agent writes to the envoy state dir.
//!
//! The shape mirrors the upstream `envoyBootstrap` template literally:
//! `node.cluster`, `node.id`, the admin listener path, the dynamic
//! resources discovery config (`xds_socket`), and the static cluster
//! pointing back at the agent's xDS unix socket.

use crate::cilium::types::Cite;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub node_id: String,
    pub cluster_name: String,
    /// Path to the admin unix socket inside the envoy state dir.
    pub admin_socket_path: String,
    /// Path to the xDS unix socket the agent listens on.
    pub xds_socket_path: String,
    /// Optional access-log socket.
    pub accesslog_socket_path: Option<String>,
    /// Number of envoy worker threads. 0 = single-threaded.
    pub concurrency: u32,
}

impl BootstrapConfig {
    /// Produce the bootstrap JSON document. The shape follows the
    /// upstream `envoy.yaml` / `envoy_bootstrap.yaml` template.
    pub fn to_json(&self) -> String {
        let access_log = self.accesslog_socket_path.as_deref().unwrap_or("");
        format!(
            r#"{{
  "node": {{ "id": "{node}", "cluster": "{cluster}" }},
  "admin": {{ "address": {{ "pipe": {{ "path": "{admin}" }} }} }},
  "static_resources": {{
    "clusters": [
      {{
        "name": "xds-grpc-cilium",
        "connect_timeout": "1s",
        "type": "STATIC",
        "http2_protocol_options": {{}},
        "load_assignment": {{
          "cluster_name": "xds-grpc-cilium",
          "endpoints": [
            {{
              "lb_endpoints": [
                {{
                  "endpoint": {{
                    "address": {{ "pipe": {{ "path": "{xds}" }} }}
                  }}
                }}
              ]
            }}
          ]
        }}
      }}
    ]
  }},
  "dynamic_resources": {{
    "lds_config": {{ "ads": {{}}, "resource_api_version": "V3" }},
    "cds_config": {{ "ads": {{}}, "resource_api_version": "V3" }},
    "ads_config": {{
      "api_type": "GRPC",
      "transport_api_version": "V3",
      "grpc_services": [{{ "envoy_grpc": {{ "cluster_name": "xds-grpc-cilium" }} }}]
    }}
  }},
  "concurrency": {conc},
  "access_log_path": "{accesslog}"
}}"#,
            node = self.node_id,
            cluster = self.cluster_name,
            admin = self.admin_socket_path,
            xds = self.xds_socket_path,
            accesslog = access_log,
            conc = self.concurrency,
        )
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/envoy/embedded_envoy.go", "Bootstrap");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn cfg() -> BootstrapConfig {
        BootstrapConfig {
            node_id: "cilium-host-1".into(),
            cluster_name: "cilium-default".into(),
            admin_socket_path: "/var/run/cilium/envoy/admin.sock".into(),
            xds_socket_path: "/var/run/cilium/xds.sock".into(),
            accesslog_socket_path: Some("/var/run/cilium/access_log.sock".into()),
            concurrency: 2,
        }
    }

    #[test]
    fn json_includes_node_id_and_cluster() {
        let (_c, _t) = cilium_test_ctx!("pkg/envoy/embedded_envoy.go", "JSON.Node", "tenant-eb-n");
        let s = cfg().to_json();
        assert!(s.contains("\"id\": \"cilium-host-1\""));
        assert!(s.contains("\"cluster\": \"cilium-default\""));
    }

    #[test]
    fn json_includes_xds_pipe_address() {
        let (_c, _t) = cilium_test_ctx!("pkg/envoy/embedded_envoy.go", "JSON.XDS", "tenant-eb-x");
        let s = cfg().to_json();
        assert!(s.contains("\"path\": \"/var/run/cilium/xds.sock\""));
    }

    #[test]
    fn json_includes_admin_pipe_address() {
        let (_c, _t) = cilium_test_ctx!("pkg/envoy/embedded_envoy.go", "JSON.Admin", "tenant-eb-a");
        let s = cfg().to_json();
        assert!(s.contains("/var/run/cilium/envoy/admin.sock"));
    }

    #[test]
    fn json_uses_v3_resource_api_version() {
        let (_c, _t) = cilium_test_ctx!("pkg/envoy/embedded_envoy.go", "JSON.V3", "tenant-eb-v3");
        let s = cfg().to_json();
        assert!(s.contains("\"V3\""));
    }

    #[test]
    fn json_static_cluster_is_xds_grpc_cilium() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/envoy/embedded_envoy.go",
            "JSON.ClusterName",
            "tenant-eb-cn"
        );
        let s = cfg().to_json();
        assert!(s.contains("\"name\": \"xds-grpc-cilium\""));
    }

    #[test]
    fn json_concurrency_field_present() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/envoy/embedded_envoy.go",
            "JSON.Concurrency",
            "tenant-eb-cc"
        );
        let s = cfg().to_json();
        assert!(s.contains("\"concurrency\": 2"));
    }

    #[test]
    fn json_optional_accesslog_path_when_set() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/envoy/embedded_envoy.go",
            "JSON.AccessLog",
            "tenant-eb-al"
        );
        let s = cfg().to_json();
        assert!(s.contains("/var/run/cilium/access_log.sock"));
    }

    #[test]
    fn json_accesslog_empty_when_none() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/envoy/embedded_envoy.go",
            "JSON.AccessLog.None",
            "tenant-eb-ale"
        );
        let mut c = cfg();
        c.accesslog_socket_path = None;
        let s = c.to_json();
        assert!(s.contains("\"access_log_path\": \"\""));
    }

    #[test]
    fn json_uses_grpc_api_type() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/envoy/embedded_envoy.go", "JSON.GRPC", "tenant-eb-grpc");
        let s = cfg().to_json();
        assert!(s.contains("\"api_type\": \"GRPC\""));
    }

    #[test]
    fn config_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/envoy/embedded_envoy.go", "Serde", "tenant-eb-srd");
        let c = cfg();
        let s = serde_json::to_string(&c).unwrap();
        let back: BootstrapConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
