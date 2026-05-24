// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-bench — K8s security benchmarks.
//!
//! Dual deep-port:
//! - aquasecurity/kube-bench v0.15.5 (CIS K8s Benchmark — master/node/etcd/control-plane)
//! - kubescape/kubescape    v4.0.8 (NSA hardening guide + MITRE ATT&CK mapping)
//!
//! Both upstreams Apache-2.0.

pub mod cis_control_plane;
pub mod cis_engine;
pub mod cis_etcd;
pub mod cis_master;
pub mod cis_node;
pub mod error;
pub mod kubescape_mitre;
pub mod kubescape_nsa;
pub mod models;

pub use error::BenchError;
pub use models::*;

pub const MODULE_NAME: &str = "bench";
