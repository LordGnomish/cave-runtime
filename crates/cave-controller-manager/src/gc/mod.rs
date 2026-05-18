// SPDX-License-Identifier: AGPL-3.0-or-later
//! GarbageCollector controller — `pkg/controller/garbagecollector`.
//!
//! Mirrors the Kubernetes GC controller. Three propagation modes:
//!
//! * **Foreground**: orphan-finalizer set on the object; controller waits
//!   until every dependent with `blockOwnerDeletion = true` is gone, then
//!   removes the finalizer.
//! * **Background**: object is deleted immediately; dependents are queued
//!   for asynchronous deletion via the dependent graph.
//! * **Orphan**: object is deleted; direct dependents have their owner
//!   references rewritten to drop the deleted object's UID.
//!
//! Pinned to k8s v1.36.0 ([`crate::types::UPSTREAM_VERSION`]).

pub mod cascade;
pub mod finalizer;
pub mod graph;
pub mod orphan;
pub mod owner_ref;
pub mod resync;

pub use cascade::{CascadePlan, DeletionPropagation, compute_cascade_plan};
pub use graph::{DependencyGraph, ObjectId};
pub use owner_ref::{OwnerReference, validate_owner_refs};
