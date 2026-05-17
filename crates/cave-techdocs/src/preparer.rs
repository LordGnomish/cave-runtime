// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TechDocsPreparer trait and NoopPreparer — port of TechDocsPreparer from Backstage.
//!
//! Upstream: TechDocsPreparer in @backstage/plugin-techdocs-node/src/techdocs/TechDocsPreparer.ts

use async_trait::async_trait;
use std::path::Path;

use crate::publisher::TechDocsError;

/// Prepares a source checkout for documentation generation (e.g. clones the repo).
///
/// Upstream: TechDocsPreparer interface in @backstage/plugin-techdocs-node
#[async_trait]
pub trait TechDocsPreparer: Send + Sync {
    /// Prepare the source, placing it under `output_dir`.
    ///
    /// Upstream: prepare(entity, output) in TechDocsPreparer.ts
    async fn prepare(
        &self,
        output_dir: &Path,
    ) -> Result<(), TechDocsError>;
}

/// No-op preparer — assumes source is already available; no cloning required.
///
/// Upstream: used in local/dev setups where the source directory already exists.
pub struct NoopPreparer;

#[async_trait]
impl TechDocsPreparer for NoopPreparer {
    async fn prepare(
        &self,
        _output_dir: &Path,
    ) -> Result<(), TechDocsError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// NoopPreparer.prepare() returns Ok(()).
    ///
    /// Upstream: preparer.test.ts — "NoopPreparer succeeds"
    #[tokio::test]
    async fn noop_preparer_succeeds() {
        let preparer = NoopPreparer;
        let out = TempDir::new().unwrap();
        let result = preparer.prepare(out.path()).await;
        assert!(result.is_ok(), "NoopPreparer must return Ok(())");
    }
}
