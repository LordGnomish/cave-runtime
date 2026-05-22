// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts shared abstraction layer (no single upstream)
//! Shared abstraction layer for cave-artifacts.
//!
//! Pulp and Harbor each have their own per-side primitives (RepositoryVersion
//! vs Project, ContentArtifact vs Manifest, etc.) that we keep upstream-
//! faithful inside their own sub-modules. This `core` module sits *above*
//! both and defines the small set of structurally-equivalent types that
//! both sides agree on — and that cross-cutting integrations (Trivy scan,
//! Cosign signature verify, retention policy evaluator, cave-portal admin
//! UI) bind against without leaking either side's specifics.
//!
//! The shape is deliberately minimal:
//! - [`RepositoryKind`] — what the repo holds (Container / Rpm / File / …)
//! - [`Repository`] trait — common surface for lookup/list/count
//! - [`Distribution`] trait — how the repo is *served* to clients
//! - [`Artifact`] — content-addressable unit (digest, size, media type, tags)
//! - [`Tag`] — human-readable pointer at an [`Artifact`] digest
//! - [`Signature`] — cosign-shape signature attached to a target digest
//! - [`Vulnerability`] — scanner finding mapped from Trivy / Grype / etc.
//! - [`RetentionPolicy`] — rule set + `evaluate(artifact) -> Keep | Delete`
//!
//! No single upstream maps to this file — it is a Cave-side aggregator.
//! Both `pulp/` and `harbor/` import it via `crate::core::*`.

pub mod artifact;
pub mod gc;
pub mod repository;
pub mod retention;
pub mod signature;
pub mod vulnerability;

pub use artifact::{Artifact, Tag};
pub use repository::{Distribution, Repository, RepositoryKind};
pub use retention::{RetentionAction, RetentionPolicy, RetentionRule};
pub use signature::{Signature, SignatureAlg};
pub use vulnerability::{AffectedComponent, Severity, Vulnerability, VulnerabilitySource};
