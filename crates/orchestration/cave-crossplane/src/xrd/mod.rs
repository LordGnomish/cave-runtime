// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! XRD (Composite Resource Definition) module — spec, schema validation,
//! defaulting, conversion + store.
//!
//! Upstream: apis/apiextensions/v1/xrd_types.go + internal/xcrd/

pub mod conversion;
pub mod crd_gen;
pub mod defaulting;
pub mod schema_validate;
pub mod spec;
pub mod store;

pub use crd_gen::{for_composite_resource, for_composite_resource_claim};
pub use store::XrdStore;
