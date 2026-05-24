// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Provider module — package install + ProviderConfig + ProviderRevision +
//! DeploymentRuntimeConfig + store.
//!
//! Upstream: apis/pkg/v1/ + internal/controller/pkg/

pub mod config;
pub mod revision;
pub mod runtime;
pub mod store;

pub use store::ProviderStore;
