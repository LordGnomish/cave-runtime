// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition module — pipeline + step + patch_transform + legacy mode + store.
//!
//! Upstream: crossplane/crossplane v2.3.1
//!   - apis/apiextensions/v1/composition_types.go
//!   - internal/controller/apiextensions/composite/composition_pipeline.go
//!   - internal/controller/apiextensions/composite/composition_resources.go
//!   - function-patch-and-transform/input/v1beta1/resources.go

pub mod legacy;
pub mod patch_transform;
pub mod pipeline;
pub mod revision_gc;
pub mod step;
pub mod store;

pub use revision_gc::RevisionGarbageCollector;
pub use store::CompositionStore;
