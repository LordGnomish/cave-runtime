// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: aquasecurity/trivy@8a3177a pkg/fanal/analyzer/pkg/{apk,dpkg}

//! OS-level package-manager analyzers.

use super::{Analyzer, AnalyzerType, PackageInfo};

// ── Alpine APK ─────────────────────────────────────────────────────────────

pub struct AlpineApkAnalyzer;

impl AlpineApkAnalyzer {
    /// Parses an APK `/lib/apk/db/installed` file.
    ///
    /// Format: records separated by blank lines, each record a sequence of
    /// `<Key>:<value>` lines where `Key` is a single ASCII letter.
    pub fn parse_installed_db(&self, input: &str) -> Vec<PackageInfo> {
        let mut out = Vec::new();
        let mut cur = PackageInfo::default();
        let mut has_data = false;

        for line in input.lines() {
            if line.is_empty() {
                if has_data && !cur.name.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur = PackageInfo::default();
                has_data = false;
                continue;
            }
            let (key, value) = match line.split_once(':') {
                Some(kv) => kv,
                None => continue,
            };
            has_data = true;
            match key {
                "P" => cur.name = value.to_string(),
                "V" => cur.version = value.to_string(),
                "A" => cur.arch = Some(value.to_string()),
                "L" => cur.license = Some(value.to_string()),
                "o" => cur.origin = Some(value.to_string()),
                "p" => {
                    cur.provides = value.split_whitespace().map(String::from).collect();
                }
                "D" => {
                    cur.depends = value.split_whitespace().map(String::from).collect();
                }
                _ => {}
            }
        }
        if has_data && !cur.name.is_empty() {
            out.push(cur);
        }
        out
    }
}

impl Analyzer for AlpineApkAnalyzer {
    fn kind(&self) -> AnalyzerType {
        AnalyzerType::AlpineApk
    }
    fn required(&self, path: &str) -> bool {
        let trimmed = path.trim_start_matches('/');
        trimmed == "lib/apk/db/installed" || trimmed == "usr/lib/apk/db/installed"
    }
}

// ── dpkg status ────────────────────────────────────────────────────────────

pub struct DpkgStatusAnalyzer;

impl DpkgStatusAnalyzer {
    /// Parses `/var/lib/dpkg/status`. Skips entries whose `Status:` is not
    /// `install ok installed`.
    pub fn parse_status(&self, input: &str) -> Vec<PackageInfo> {
        let mut out = Vec::new();
        let mut cur = PackageInfo::default();
        let mut installed = false;
        let mut has_data = false;

        let flush = |out: &mut Vec<PackageInfo>,
                     cur: &mut PackageInfo,
                     installed: &mut bool,
                     has_data: &mut bool| {
            if *has_data && *installed && !cur.name.is_empty() {
                out.push(std::mem::take(cur));
            } else {
                *cur = PackageInfo::default();
            }
            *installed = false;
            *has_data = false;
        };

        for line in input.lines() {
            if line.is_empty() {
                flush(&mut out, &mut cur, &mut installed, &mut has_data);
                continue;
            }
            let (key, value) = match line.split_once(':') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => continue,
            };
            has_data = true;
            match key {
                "Package" => cur.name = value.to_string(),
                "Status" => installed = value == "install ok installed",
                "Version" => cur.version = value.to_string(),
                "Architecture" => cur.arch = Some(value.to_string()),
                "Source" => {
                    if let Some((src, ver)) = value.split_once('(') {
                        cur.source = Some(src.trim().to_string());
                        cur.source_version =
                            Some(ver.trim_end_matches(')').trim().to_string());
                    } else {
                        cur.source = Some(value.to_string());
                    }
                }
                _ => {}
            }
        }
        flush(&mut out, &mut cur, &mut installed, &mut has_data);
        out
    }
}

impl Analyzer for DpkgStatusAnalyzer {
    fn kind(&self) -> AnalyzerType {
        AnalyzerType::DpkgStatus
    }
    fn required(&self, path: &str) -> bool {
        path.trim_start_matches('/') == "var/lib/dpkg/status"
    }
}
