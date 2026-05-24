// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XPKG package manager — pull (offline OCI), install (CRD + composition +
//! function manifest extraction), revision rollout, dependency resolution.
//!
//! Upstream: internal/xpkg/ + internal/controller/pkg/

pub mod dependency;
pub mod install;
pub mod pull;
pub mod revision;
