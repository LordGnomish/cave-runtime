// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-bench — K8s security benchmarks.
//!
//! Dual deep-port:
//! - aquasecurity/kube-bench v0.15.5 (CIS K8s Benchmark — master/node/etcd/control-plane)
//! - kubescape/kubescape    v4.0.8 (NSA hardening guide + MITRE ATT&CK mapping)
//!
//! Both upstreams Apache-2.0.

pub mod error;
pub mod models;

pub use error::BenchError;
pub use models::*;
