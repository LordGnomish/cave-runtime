// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/scanner/scanner.go

//! Scanner trait + dispatch.

use crate::analyzer::PackageInfo;
use std::path::PathBuf;
use thiserror::Error;

pub mod fs;

#[derive(Debug, Clone)]
pub enum ScanTarget {
    Filesystem(PathBuf),
    ImageTar(PathBuf),
    ImageReference(String),
    Sbom(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ScanRequest {
    pub target: ScanTarget,
}

#[derive(Debug, Default, Clone)]
pub struct ScanReport {
    pub target: String,
    pub packages: Vec<PackageInfo>,
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid target for scanner: {0}")]
    InvalidTarget(String),
    #[error("parse error: {0}")]
    Parse(String),
}

pub trait Scanner: Send + Sync {
    fn name(&self) -> &'static str;
    fn scan(&self, req: &ScanRequest) -> Result<ScanReport, ScanError>;
}
