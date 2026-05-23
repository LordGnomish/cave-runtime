// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Outbound integrations.
//!
//! Mirrors `org.dependencytrack.integrations.{defectdojo,fortifyssc,kenna}`.
//! ThreadFix is added as a fourth target (CSV-formatted finding upload).

pub mod defectdojo;
pub mod fortify;
pub mod kenna;
pub mod threadfix;

pub use defectdojo::{DefectDojoConfig, build_defectdojo_payload};
pub use fortify::{FortifyConfig, build_fortify_payload};
pub use kenna::{KennaConfig, build_kenna_payload};
pub use threadfix::{ThreadFixConfig, build_threadfix_csv};
