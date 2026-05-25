// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DNSSEC primitives — NSEC / NSEC3 denial-of-existence, DNSKEY trust
//! anchor, RRSIG validation, top-level validator orchestrator.
//!
//! The protocol layer (`src/protocol/dnssec.rs`) provides the hickory-proto
//! bindings; this module breaks the upstream `plugin/dnssec/` directory
//! into focused sub-modules so each sub-system can be audited + tested
//! independently of the rest.

pub mod dnskey;
pub mod nsec;
pub mod nsec3;
pub mod rrsig;
pub mod validator;

pub use dnskey::{Dnskey, DnskeyFlags, TrustAnchor};
pub use nsec::Nsec;
pub use nsec3::Nsec3;
pub use rrsig::{Rrsig, RrsigAlgorithm};
pub use validator::{ValidationOutcome, Validator};
