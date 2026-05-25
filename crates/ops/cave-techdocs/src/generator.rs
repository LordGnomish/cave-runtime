// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TechDocsGenerator trait and NoopGenerator — port of TechDocsGenerator from Backstage.
//!
//! Upstream: TechDocsGenerator in @backstage/plugin-techdocs-node/src/techdocs/TechDocsGenerator.ts

use async_trait::async_trait;
use std::path::Path;

use crate::publisher::TechDocsError;

/// Generates TechDocs from a source directory (e.g. runs mkdocs).
///
/// Upstream: TechDocsGenerator interface in @backstage/plugin-techdocs-node
#[async_trait]
pub trait TechDocsGenerator: Send + Sync {
    /// Generate documentation from `source_dir` into `output_dir`.
    ///
    /// Upstream: generateDocs(entity, source, output) in TechDocsGenerator.ts
    async fn generate(&self, source_dir: &Path, output_dir: &Path) -> Result<(), TechDocsError>;
}

/// No-op generator — passes source through without running mkdocs.
///
/// Upstream: used in test/local setups where docs are pre-built.
pub struct NoopGenerator;

#[async_trait]
impl TechDocsGenerator for NoopGenerator {
    async fn generate(&self, _source_dir: &Path, _output_dir: &Path) -> Result<(), TechDocsError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// NoopGenerator.generate() returns Ok(()).
    ///
    /// Upstream: generator.test.ts — "NoopGenerator succeeds"
    #[tokio::test]
    async fn noop_generator_succeeds() {
        let generator = NoopGenerator;
        let src = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let result = generator.generate(src.path(), out.path()).await;
        assert!(result.is_ok(), "NoopGenerator must return Ok(())");
    }
}
