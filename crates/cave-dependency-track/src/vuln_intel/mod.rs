// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Vulnerability intelligence aggregation.
//!
//! Mirrors `org.dependencytrack.parser.{nvd,osv,github,snyk,ossindex,vulndb,epss}`.

pub mod epss;
pub mod ghsa;
pub mod nvd;
pub mod osv;
pub mod ossindex;
pub mod snyk;
pub mod store;
pub mod vulndb;

pub use epss::{EpssEntry, join_epss, parse_epss_csv};
pub use ghsa::{GhsaAdvisory, parse_ghsa_json};
pub use nvd::{NvdCve, parse_nvd_2_0};
pub use osv::{OsvAdvisory, parse_osv_json};
pub use ossindex::{OssIndexReport, parse_ossindex_response};
pub use snyk::{SnykAdvisory, parse_snyk_json};
pub use store::VulnStore;
pub use vulndb::{VulnDbEntry, parse_vulndb_response};
