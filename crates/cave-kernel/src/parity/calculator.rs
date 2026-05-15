//! Parity calculator: resolves a `ParityManifest` against the local filesystem.

use super::manifest::ParityManifest;
use super::types::{GapItem, GapKind, ParityMetric, ParityReport};

// ── FileResolver trait ────────────────────────────────────────────────────────

/// Abstracts filesystem access so the calculator can be unit-tested without real files.
pub trait FileResolver: Send + Sync {
    /// Returns `true` if the given path (relative to the module root) exists on disk.
    fn file_exists(&self, path: &str) -> bool;
    /// Returns `true` if `path` (relative to the module root) contains `pattern`.
    fn file_contains(&self, path: &str, pattern: &str) -> bool;
    /// Returns `true` if any `.rs` file under `source_root` (relative to the module root)
    /// contains `pattern`.
    fn source_contains(&self, source_root: &str, pattern: &str) -> bool;
    /// Returns `true` if any `.rs` file under either `source_root` or the
    /// sibling `tests/` directory contains `pattern`. Used for test parity so
    /// integration tests living outside `src/` are still discoverable.
    fn source_or_tests_contains(&self, source_root: &str, pattern: &str) -> bool {
        self.source_contains(source_root, pattern) || self.source_contains("tests", pattern)
    }
    /// Returns the number of lines containing `todo!` or `unimplemented!` in the source tree.
    fn count_stubs(&self, source_root: &str) -> u32;
}

// ── FsResolver ────────────────────────────────────────────────────────────────

/// Production resolver that reads from the real filesystem.
///
/// Construct with the module's crate root (usually `env!("CARGO_MANIFEST_DIR")`).
pub struct FsResolver {
    base: std::path::PathBuf,
}

impl FsResolver {
    pub fn new(base: impl Into<std::path::PathBuf>) -> Self {
        Self { base: base.into() }
    }
}

impl FileResolver for FsResolver {
    fn file_exists(&self, path: &str) -> bool {
        self.base.join(path).exists()
    }

    fn file_contains(&self, path: &str, pattern: &str) -> bool {
        let full = self.base.join(path);
        std::fs::read_to_string(full)
            .map(|c| c.contains(pattern))
            .unwrap_or(false)
    }

    fn source_contains(&self, source_root: &str, pattern: &str) -> bool {
        walk_contains(&self.base.join(source_root), pattern)
    }

    fn count_stubs(&self, source_root: &str) -> u32 {
        walk_stub_count(&self.base.join(source_root))
    }
}

fn walk_contains(dir: &std::path::Path, pattern: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if walk_contains(&path, pattern) {
                return true;
            }
        } else if path.extension().map_or(false, |e| e == "rs") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.contains(pattern) {
                    return true;
                }
            }
        }
    }
    false
}

fn walk_stub_count(dir: &std::path::Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut count = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += walk_stub_count(&path);
        } else if path.extension().map_or(false, |e| e == "rs") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.starts_with("//") {
                        if trimmed.contains("todo!") || trimmed.contains("unimplemented!") {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    count
}

// ── ParityCalculator ──────────────────────────────────────────────────────────

pub struct ParityCalculator<R: FileResolver> {
    resolver: R,
}

impl<R: FileResolver> ParityCalculator<R> {
    pub fn new(resolver: R) -> Self {
        Self { resolver }
    }

    pub fn calculate(&self, manifest: &ParityManifest) -> ParityReport {
        let source_root = manifest.module.source_root.as_deref().unwrap_or("src");

        let file_parity = self.calc_file_parity(manifest);
        let function_parity = self.calc_function_parity(manifest);
        let test_parity = self.calc_test_parity(manifest, source_root);
        let surface_parity = self.calc_surface_parity(manifest, source_root);
        let stubs_detected = self.resolver.count_stubs(source_root);

        // 2026-05-13: prefer the manifest's measured `[parity] fill_ratio`
        // over the legacy 4-axis heuristic average. The heuristic does
        // substring matches that pass too easily (it counts `fn name` as
        // present even when the function has different generics, body, or
        // wrong impl block); the on-disk `fill_ratio` is the author's
        // own audited number, which the compliance dashboard already
        // uses elsewhere. Burak reported "/upstream shows Cilium 100%"
        // (heuristic) vs the dashboard's 92% (fill_ratio) — both refer
        // to cave-net. Make them consistent.
        //
        // Falls back to the heuristic average when no `[parity]` block
        // exists (legacy manifests) or when fill_ratio is unset.
        let heuristic_overall = (file_parity.score
            + function_parity.score
            + test_parity.score
            + surface_parity.score)
            / 4.0;
        let overall = manifest
            .parity
            .as_ref()
            .and_then(|p| p.measured_ratio())
            .unwrap_or(heuristic_overall);

        let gaps = self.collect_gaps(manifest, source_root);

        let upstream_ref = match manifest.primary_upstream() {
            Some(u) => format!("{}/{} @ {}", u.org, u.repo, u.version),
            None => "(no upstream declared)".to_string(),
        };
        ParityReport {
            module: manifest.module.name.clone(),
            upstream_ref,
            measured_at: chrono::Utc::now(),
            file_parity,
            function_parity,
            test_parity,
            surface_parity,
            overall,
            stubs_detected,
            gaps,
        }
    }

    fn calc_file_parity(&self, manifest: &ParityManifest) -> ParityMetric {
        let total = manifest.files.len() as u32;
        if total == 0 {
            return ParityMetric { score: 0.0, matched: 0, total: 0 };
        }
        let matched = manifest
            .files
            .iter()
            .filter(|f| self.resolver.file_exists(&f.local))
            .count() as u32;
        ParityMetric { score: matched as f32 / total as f32, matched, total }
    }

    fn calc_function_parity(&self, manifest: &ParityManifest) -> ParityMetric {
        let total = manifest.functions.len() as u32;
        if total == 0 {
            return ParityMetric { score: 0.0, matched: 0, total: 0 };
        }
        let matched = manifest
            .functions
            .iter()
            .filter(|f| {
                self.resolver
                    .file_contains(&f.file, &format!("fn {}", f.local_name))
            })
            .count() as u32;
        ParityMetric { score: matched as f32 / total as f32, matched, total }
    }

    fn calc_test_parity(&self, manifest: &ParityManifest, source_root: &str) -> ParityMetric {
        let total = manifest.tests.len() as u32;
        if total == 0 {
            return ParityMetric { score: 0.0, matched: 0, total: 0 };
        }
        let matched = manifest
            .tests
            .iter()
            .filter(|t| {
                self.resolver
                    .source_or_tests_contains(source_root, &format!("fn {}", t.local_test))
            })
            .count() as u32;
        ParityMetric { score: matched as f32 / total as f32, matched, total }
    }

    fn calc_surface_parity(&self, manifest: &ParityManifest, source_root: &str) -> ParityMetric {
        let total = manifest.surfaces.len() as u32;
        if total == 0 {
            return ParityMetric { score: 0.0, matched: 0, total: 0 };
        }
        let matched = manifest
            .surfaces
            .iter()
            .filter(|s| self.resolver.source_contains(source_root, &s.local_path))
            .count() as u32;
        ParityMetric { score: matched as f32 / total as f32, matched, total }
    }

    fn collect_gaps(&self, manifest: &ParityManifest, source_root: &str) -> Vec<GapItem> {
        let mut gaps = Vec::new();

        for f in &manifest.files {
            if !self.resolver.file_exists(&f.local) {
                gaps.push(GapItem {
                    kind: GapKind::File,
                    upstream: f.upstream.clone(),
                    local: Some(f.local.clone()),
                });
            }
        }

        for f in &manifest.functions {
            if !self.resolver
                .file_contains(&f.file, &format!("fn {}", f.local_name))
            {
                gaps.push(GapItem {
                    kind: GapKind::Function,
                    upstream: f.upstream_name.clone(),
                    local: Some(f.local_name.clone()),
                });
            }
        }

        for t in &manifest.tests {
            if !self.resolver
                .source_contains(source_root, &format!("fn {}", t.local_test))
            {
                gaps.push(GapItem {
                    kind: GapKind::Test,
                    upstream: t.upstream_test.clone(),
                    local: Some(t.local_test.clone()),
                });
            }
        }

        for s in &manifest.surfaces {
            if !self.resolver.source_contains(source_root, &s.local_path) {
                gaps.push(GapItem {
                    kind: GapKind::Surface,
                    upstream: s.upstream_path.clone(),
                    local: Some(s.local_path.clone()),
                });
            }
        }

        gaps
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::manifest::{
        FileMapping, FunctionMapping, ModuleInfo, ParityManifest, ParitySection, SurfaceMapping,
        TestMapping, UpstreamInfo,
    };
    use std::collections::{HashMap, HashSet};

    // ── MockResolver ──────────────────────────────────────────────────────────

    struct MockResolver {
        files: HashSet<String>,
        file_patterns: HashMap<String, Vec<String>>,
        source_patterns: HashMap<String, Vec<String>>,
        stubs: HashMap<String, u32>,
    }

    impl MockResolver {
        fn new() -> Self {
            Self {
                files: HashSet::new(),
                file_patterns: HashMap::new(),
                source_patterns: HashMap::new(),
                stubs: HashMap::new(),
            }
        }

        fn with_file(mut self, path: &str) -> Self {
            self.files.insert(path.to_string());
            self
        }

        /// Register that `file` contains the given `pattern`.
        fn with_file_pattern(mut self, file: &str, pattern: &str) -> Self {
            self.files.insert(file.to_string());
            self.file_patterns
                .entry(file.to_string())
                .or_default()
                .push(pattern.to_string());
            self
        }

        /// Register that some file under `root` contains `pattern`.
        fn with_source_pattern(mut self, root: &str, pattern: &str) -> Self {
            self.source_patterns
                .entry(root.to_string())
                .or_default()
                .push(pattern.to_string());
            self
        }

        fn with_stubs(mut self, root: &str, count: u32) -> Self {
            self.stubs.insert(root.to_string(), count);
            self
        }
    }

    impl FileResolver for MockResolver {
        fn file_exists(&self, path: &str) -> bool {
            self.files.contains(path)
        }

        fn file_contains(&self, file: &str, pattern: &str) -> bool {
            self.file_patterns
                .get(file)
                .map(|ps| ps.iter().any(|p| p.contains(pattern)))
                .unwrap_or(false)
        }

        fn source_contains(&self, root: &str, pattern: &str) -> bool {
            self.source_patterns
                .get(root)
                .map(|ps| ps.iter().any(|p| p.contains(pattern)))
                .unwrap_or(false)
        }

        fn count_stubs(&self, root: &str) -> u32 {
            *self.stubs.get(root).unwrap_or(&0)
        }
    }

    // ── Fixtures ──────────────────────────────────────────────────────────────

    fn sample_manifest() -> ParityManifest {
        ParityManifest {
            upstream: Some(UpstreamInfo {
                org: "upstream-org".into(),
                repo: "upstream-repo".into(),
                version: "v1.0.0".into(),
            }),
            upstreams: Vec::new(),
            module: ModuleInfo {
                name: "test-module".into(),
                description: None,
                source_root: Some("src".into()),
            },
            files: vec![
                FileMapping { upstream: "foo.go".into(), local: "src/foo.rs".into() },
                FileMapping { upstream: "bar.go".into(), local: "src/bar.rs".into() },
            ],
            functions: vec![
                FunctionMapping {
                    upstream_name: "Foo".into(),
                    local_name: "foo".into(),
                    file: "src/foo.rs".into(),
                },
                FunctionMapping {
                    upstream_name: "Bar".into(),
                    local_name: "bar".into(),
                    file: "src/foo.rs".into(),
                },
            ],
            tests: vec![
                TestMapping {
                    upstream_test: "TestFoo".into(),
                    local_test: "test_foo".into(),
                },
                TestMapping {
                    upstream_test: "TestBar".into(),
                    local_test: "test_bar".into(),
                },
            ],
            surfaces: vec![
                SurfaceMapping {
                    kind: "http".into(),
                    upstream_path: "/api/foo".into(),
                    local_path: "/api/foo".into(),
                },
                SurfaceMapping {
                    kind: "http".into(),
                    upstream_path: "/api/bar".into(),
                    local_path: "/api/bar".into(),
                },
            ],
            parity: None,
        }
    }

    // ── file_parity ───────────────────────────────────────────────────────────

    #[test]
    fn file_parity_all_present() {
        let r = MockResolver::new().with_file("src/foo.rs").with_file("src/bar.rs");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.file_parity.matched, 2);
        assert_eq!(report.file_parity.total, 2);
        assert!((report.file_parity.score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn file_parity_partial() {
        let r = MockResolver::new().with_file("src/foo.rs");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.file_parity.matched, 1);
        assert_eq!(report.file_parity.total, 2);
        assert!((report.file_parity.score - 0.5).abs() < 1e-5);
    }

    #[test]
    fn file_parity_none_present() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.file_parity.matched, 0);
        assert_eq!(report.file_parity.total, 2);
        assert!((report.file_parity.score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn file_parity_empty_mapping() {
        let r = MockResolver::new();
        let mut m = sample_manifest();
        m.files.clear();
        let report = ParityCalculator::new(r).calculate(&m);
        assert_eq!(report.file_parity.total, 0);
        assert_eq!(report.file_parity.matched, 0);
        assert!((report.file_parity.score - 0.0).abs() < f32::EPSILON);
    }

    // ── function_parity ───────────────────────────────────────────────────────

    #[test]
    fn function_parity_all_present() {
        let r = MockResolver::new()
            .with_file_pattern("src/foo.rs", "fn foo")
            .with_file_pattern("src/foo.rs", "fn bar");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.function_parity.matched, 2);
        assert_eq!(report.function_parity.total, 2);
        assert!((report.function_parity.score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn function_parity_one_missing() {
        let r = MockResolver::new().with_file_pattern("src/foo.rs", "fn foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.function_parity.matched, 1);
        assert_eq!(report.function_parity.total, 2);
        assert!((report.function_parity.score - 0.5).abs() < 1e-5);
    }

    #[test]
    fn function_parity_none_present() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.function_parity.matched, 0);
        assert!((report.function_parity.score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn function_parity_empty_mapping() {
        let r = MockResolver::new();
        let mut m = sample_manifest();
        m.functions.clear();
        let report = ParityCalculator::new(r).calculate(&m);
        assert_eq!(report.function_parity.total, 0);
        assert_eq!(report.function_parity.matched, 0);
    }

    // ── test_parity ───────────────────────────────────────────────────────────

    #[test]
    fn test_parity_all_present() {
        let r = MockResolver::new()
            .with_source_pattern("src", "fn test_foo")
            .with_source_pattern("src", "fn test_bar");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.test_parity.matched, 2);
        assert_eq!(report.test_parity.total, 2);
        assert!((report.test_parity.score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parity_partial() {
        let r = MockResolver::new().with_source_pattern("src", "fn test_foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.test_parity.matched, 1);
        assert_eq!(report.test_parity.total, 2);
        assert!((report.test_parity.score - 0.5).abs() < 1e-5);
    }

    #[test]
    fn test_parity_none_present() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.test_parity.matched, 0);
        assert!((report.test_parity.score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_parity_empty_mapping() {
        let r = MockResolver::new();
        let mut m = sample_manifest();
        m.tests.clear();
        let report = ParityCalculator::new(r).calculate(&m);
        assert_eq!(report.test_parity.total, 0);
        assert_eq!(report.test_parity.matched, 0);
    }

    // ── surface_parity ────────────────────────────────────────────────────────

    #[test]
    fn surface_parity_all_present() {
        let r = MockResolver::new()
            .with_source_pattern("src", "/api/foo")
            .with_source_pattern("src", "/api/bar");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.surface_parity.matched, 2);
        assert_eq!(report.surface_parity.total, 2);
        assert!((report.surface_parity.score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn surface_parity_partial() {
        let r = MockResolver::new().with_source_pattern("src", "/api/foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.surface_parity.matched, 1);
        assert_eq!(report.surface_parity.total, 2);
        assert!((report.surface_parity.score - 0.5).abs() < 1e-5);
    }

    #[test]
    fn surface_parity_none() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.surface_parity.matched, 0);
        assert!((report.surface_parity.score - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn surface_parity_empty_mapping() {
        let r = MockResolver::new();
        let mut m = sample_manifest();
        m.surfaces.clear();
        let report = ParityCalculator::new(r).calculate(&m);
        assert_eq!(report.surface_parity.total, 0);
    }

    // ── stubs_detected ────────────────────────────────────────────────────────

    #[test]
    fn stubs_detected_nonzero() {
        let r = MockResolver::new().with_stubs("src", 7);
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.stubs_detected, 7);
    }

    #[test]
    fn stubs_detected_zero_by_default() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.stubs_detected, 0);
    }

    // ── overall ───────────────────────────────────────────────────────────────

    #[test]
    fn overall_is_average_of_four_metrics() {
        // 1/2 files, 1/2 functions, 1/2 tests, 1/2 surfaces → overall = 0.5
        let r = MockResolver::new()
            .with_file("src/foo.rs")
            .with_file_pattern("src/foo.rs", "fn foo")
            .with_source_pattern("src", "fn test_foo")
            .with_source_pattern("src", "/api/foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert!((report.overall - 0.5).abs() < 1e-5,
            "expected 0.5, got {}", report.overall);
    }

    #[test]
    fn overall_zero_when_all_missing() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert!((report.overall - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn overall_one_when_all_present() {
        let r = MockResolver::new()
            .with_file("src/foo.rs")
            .with_file("src/bar.rs")
            .with_file_pattern("src/foo.rs", "fn foo")
            .with_file_pattern("src/foo.rs", "fn bar")
            .with_source_pattern("src", "fn test_foo")
            .with_source_pattern("src", "fn test_bar")
            .with_source_pattern("src", "/api/foo")
            .with_source_pattern("src", "/api/bar");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert!((report.overall - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn overall_prefers_manifest_fill_ratio_over_heuristic() {
        // Regression for the Cilium-100% display bug (2026-05-13).
        // Before the fix, /upstream tracker over-reported cave-net as
        // 100% because all 4 heuristic axes' substring matches passed,
        // while the on-disk fill_ratio was 0.9179. Both surfaces
        // (compliance dashboard + /upstream tracker) must show the
        // same number now — the manifest's measured fill_ratio wins.
        let r = MockResolver::new()
            .with_file("src/foo.rs")
            .with_file("src/bar.rs")
            .with_file_pattern("src/foo.rs", "fn foo")
            .with_file_pattern("src/foo.rs", "fn bar")
            .with_source_pattern("src", "fn test_foo")
            .with_source_pattern("src", "fn test_bar")
            .with_source_pattern("src", "/api/foo")
            .with_source_pattern("src", "/api/bar");
        let mut manifest = sample_manifest();
        manifest.parity = Some(ParitySection {
            fill_ratio: Some(0.9179),
            honest_ratio: Some(0.9179),
            ratio: None,
            infra_only: Some(false),
            mapped_count: Some(42),
            partial_count: Some(0),
            skipped_count: Some(81),
            unmapped_count: Some(11),
            total: Some(134),
            last_audit: Some("2026-05-13".into()),
        });
        let report = ParityCalculator::new(r).calculate(&manifest);
        // Heuristic alone would give 1.0; manifest fill_ratio wins.
        assert!(
            (report.overall - 0.9179).abs() < 1e-4,
            "expected overall = manifest fill_ratio 0.9179, got {}",
            report.overall
        );
    }

    #[test]
    fn overall_legacy_ratio_field_recognised() {
        // Pre-2026-05-12 manifests used `ratio` instead of `fill_ratio`.
        // The reader treats them as synonymous (measured_ratio falls
        // back to ratio when fill_ratio is None).
        let r = MockResolver::new(); // empty resolver → heuristic = 0
        let mut manifest = sample_manifest();
        manifest.parity = Some(ParitySection {
            fill_ratio: None,
            ratio: Some(0.42),
            ..Default::default()
        });
        let report = ParityCalculator::new(r).calculate(&manifest);
        assert!((report.overall - 0.42).abs() < 1e-4);
    }

    #[test]
    fn overall_falls_back_to_heuristic_when_no_parity_block() {
        // Back-compat: manifests without a [parity] block keep the
        // legacy 4-axis heuristic. Test re-asserts the same
        // expectation as overall_one_when_all_present.
        let r = MockResolver::new()
            .with_file("src/foo.rs")
            .with_file("src/bar.rs")
            .with_file_pattern("src/foo.rs", "fn foo")
            .with_file_pattern("src/foo.rs", "fn bar")
            .with_source_pattern("src", "fn test_foo")
            .with_source_pattern("src", "fn test_bar")
            .with_source_pattern("src", "/api/foo")
            .with_source_pattern("src", "/api/bar");
        let mut manifest = sample_manifest();
        manifest.parity = None;
        let report = ParityCalculator::new(r).calculate(&manifest);
        assert!((report.overall - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn overall_parity_section_without_ratio_falls_back_to_heuristic() {
        // [parity] block present but measured_ratio is None — fall back
        // to heuristic.
        let r = MockResolver::new()
            .with_file("src/foo.rs")
            .with_file("src/bar.rs");
        // 2 of 2 files match, 0 functions/tests/surfaces → heuristic = 0.25.
        let mut manifest = sample_manifest();
        manifest.parity = Some(ParitySection {
            fill_ratio: None,
            ratio: None,
            ..Default::default()
        });
        let report = ParityCalculator::new(r).calculate(&manifest);
        assert!((report.overall - 0.25).abs() < f32::EPSILON);
    }

    // ── gaps ──────────────────────────────────────────────────────────────────

    #[test]
    fn gaps_report_missing_file() {
        let r = MockResolver::new().with_file("src/foo.rs"); // bar.rs missing
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        let file_gaps: Vec<_> = report.gaps.iter().filter(|g| g.kind == GapKind::File).collect();
        assert_eq!(file_gaps.len(), 1);
        assert_eq!(file_gaps[0].upstream, "bar.go");
        assert_eq!(file_gaps[0].local.as_deref(), Some("src/bar.rs"));
    }

    #[test]
    fn gaps_report_missing_function() {
        let r = MockResolver::new().with_file_pattern("src/foo.rs", "fn foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        let fn_gaps: Vec<_> =
            report.gaps.iter().filter(|g| g.kind == GapKind::Function).collect();
        assert_eq!(fn_gaps.len(), 1);
        assert_eq!(fn_gaps[0].upstream, "Bar");
        assert_eq!(fn_gaps[0].local.as_deref(), Some("bar"));
    }

    #[test]
    fn gaps_report_missing_test() {
        let r = MockResolver::new().with_source_pattern("src", "fn test_foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        let test_gaps: Vec<_> =
            report.gaps.iter().filter(|g| g.kind == GapKind::Test).collect();
        assert_eq!(test_gaps.len(), 1);
        assert_eq!(test_gaps[0].upstream, "TestBar");
    }

    #[test]
    fn gaps_report_missing_surface() {
        let r = MockResolver::new().with_source_pattern("src", "/api/foo");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        let surf_gaps: Vec<_> =
            report.gaps.iter().filter(|g| g.kind == GapKind::Surface).collect();
        assert_eq!(surf_gaps.len(), 1);
        assert_eq!(surf_gaps[0].upstream, "/api/bar");
    }

    #[test]
    fn gaps_empty_when_all_present() {
        let r = MockResolver::new()
            .with_file("src/foo.rs")
            .with_file("src/bar.rs")
            .with_file_pattern("src/foo.rs", "fn foo")
            .with_file_pattern("src/foo.rs", "fn bar")
            .with_source_pattern("src", "fn test_foo")
            .with_source_pattern("src", "fn test_bar")
            .with_source_pattern("src", "/api/foo")
            .with_source_pattern("src", "/api/bar");
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn gaps_all_missing_are_reported() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        // 2 files + 2 functions + 2 tests + 2 surfaces = 8 gaps
        assert_eq!(report.gaps.len(), 8);
    }

    // ── manifest parsing ──────────────────────────────────────────────────────

    #[test]
    fn manifest_parses_from_toml() {
        let toml = r#"
[upstream]
org = "test-org"
repo = "test-repo"
version = "v1.0"

[module]
name = "test-mod"
source_root = "src"

[[files]]
upstream = "foo.go"
local = "src/foo.rs"

[[functions]]
upstream_name = "Foo"
local_name = "foo"
file = "src/foo.rs"

[[tests]]
upstream_test = "TestFoo"
local_test = "test_foo"

[[surfaces]]
kind = "http"
upstream_path = "/api/foo"
local_path = "/api/foo"
"#;
        let manifest: crate::parity::manifest::ParityManifest =
            toml::from_str(toml).unwrap();
        let primary = manifest.primary_upstream().unwrap();
        assert_eq!(primary.org, "test-org");
        assert_eq!(primary.repo, "test-repo");
        assert_eq!(primary.version, "v1.0");
        assert_eq!(manifest.module.name, "test-mod");
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.functions.len(), 1);
        assert_eq!(manifest.tests.len(), 1);
        assert_eq!(manifest.surfaces.len(), 1);
    }

    #[test]
    fn manifest_allows_empty_arrays() {
        let toml = r#"
[upstream]
org = "o"
repo = "r"
version = "v0"

[module]
name = "m"
"#;
        let manifest: crate::parity::manifest::ParityManifest =
            toml::from_str(toml).unwrap();
        assert!(manifest.files.is_empty());
        assert!(manifest.functions.is_empty());
        assert!(manifest.tests.is_empty());
        assert!(manifest.surfaces.is_empty());
    }

    #[test]
    fn upstream_ref_format_is_correct() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.upstream_ref, "upstream-org/upstream-repo @ v1.0.0");
    }

    #[test]
    fn module_name_in_report_matches_manifest() {
        let r = MockResolver::new();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        assert_eq!(report.module, "test-module");
    }

    #[test]
    fn source_root_defaults_to_src_when_absent() {
        let r = MockResolver::new().with_source_pattern("src", "fn test_foo");
        let mut m = sample_manifest();
        m.module.source_root = None;
        let report = ParityCalculator::new(r).calculate(&m);
        // If the default "src" is used correctly, test_foo will be found
        assert_eq!(report.test_parity.matched, 1);
    }

    #[test]
    fn measured_at_is_set() {
        let r = MockResolver::new();
        let before = chrono::Utc::now();
        let report = ParityCalculator::new(r).calculate(&sample_manifest());
        let after = chrono::Utc::now();
        assert!(report.measured_at >= before);
        assert!(report.measured_at <= after);
    }
}
