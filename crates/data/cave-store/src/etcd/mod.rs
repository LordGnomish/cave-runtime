// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! etcd v3 API implementation.

pub mod auth;
pub mod cluster;
pub mod routes;

pub use routes::etcd_router;
