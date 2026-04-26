//! cave-cloud-controller-manager — parity scaffold for `cloud-controller-manager`.
//!
//! Mirrors the `staging/src/k8s.io/cloud-provider` tree of upstream Kubernetes
//! (pinned to [`types::UPSTREAM_VERSION`]), plus minimal scaffolds for two
//! out-of-tree providers:
//!
//! * **Hetzner Cloud** — `hetznercloud/hcloud-cloud-controller-manager`
//!   (pinned to [`providers::hetzner::PROVIDER_VERSION`]).
//! * **Microsoft Azure** — `kubernetes-sigs/cloud-provider-azure`
//!   (pinned to [`providers::azure::PROVIDER_VERSION`]).
//!
//! Multi-tenancy is a first-class concern: every [`provider::CloudConfig`]
//! carries a [`types::TenantId`] and providers MUST refuse to act on a
//! resource belonging to a different tenant.

#![allow(clippy::needless_doctest_main)]

pub mod types;
pub mod provider;

pub mod node_controller;
pub mod node_lifecycle;
pub mod route_controller;
pub mod service_controller;
pub mod service_extras;

pub mod providers;

pub use provider::{
    CloudConfig, CloudProvider, ClustersIface, InstancesIface, LoadBalancerIface, RoutesIface,
    ZonesIface,
};
pub use types::{Cite, CloudError, ProviderName, Reconcile, TenantId, UPSTREAM_VERSION};
