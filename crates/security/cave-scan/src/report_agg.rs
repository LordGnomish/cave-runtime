// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/report/report.go (multi-scanner merge)

//! Cross-scanner report aggregation.

use crate::analyzer::PackageInfo;

#[derive(Debug, Default, Clone)]
pub struct ScannerReport {
    pub scanner_name: String,
    pub target: String,
    pub packages: Vec<PackageInfo>,
}

#[derive(Debug, Default, Clone)]
pub struct AggregatedReport {
    pub reports: Vec<ScannerReport>,
    pub total_packages: usize,
}

pub fn aggregate(reports: Vec<ScannerReport>) -> AggregatedReport {
    let total_packages = reports.iter().map(|r| r.packages.len()).sum();
    AggregatedReport {
        reports,
        total_packages,
    }
}
