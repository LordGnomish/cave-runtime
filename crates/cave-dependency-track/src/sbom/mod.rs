// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SBOM parsing + ingestion.

pub mod cyclonedx;
pub mod ingest;
pub mod spdx;

pub use cyclonedx::{CycloneDxBom, parse_cyclonedx_json};
pub use ingest::{IngestReport, ingest};
pub use spdx::{SpdxDocument, parse_spdx_json, parse_spdx_tag_value};
