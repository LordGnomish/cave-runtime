// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Import / Export — serialise repository content for air-gapped environments.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{Export, Exporter};
use crate::pulp::store::ArtifactsState;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

/// Run an export job: collect content from the specified repositories and
/// write a manifest to the exporter's path.
///
/// In production this writes tarballs + checksums to `exporter.path`.
/// Currently returns a stub export record.
pub async fn run_export(
    state: Arc<ArtifactsState>,
    exporter: &Exporter,
) -> Result<Export, ArtifactsError> {
    let mut exported: Vec<String> = vec![];
    let mut file_info: HashMap<String, String> = HashMap::new();

    for repo_href in &exporter.repositories {
        let repo = state
            .get_repository(repo_href)
            .await
            .ok_or_else(|| ArtifactsError::NotFound(format!("repository {repo_href}")))?;

        let versions = state.list_repo_versions(repo_href).await;
        let latest = versions.iter().max_by_key(|v| v.number);

        if let Some(ver) = latest {
            let content_count = ver.content_hrefs.len();
            info!(
                repo = %repo.name,
                version = ver.number,
                content_units = content_count,
                "exporting repository"
            );
            exported.push(ver.pulp_href.clone());
            file_info.insert(
                format!("{}.json", repo.name),
                format!("sha256:stub-{}", repo.pulp_id),
            );
        }
    }

    let id = Uuid::new_v4();
    let task_href = format!("/pulp/api/v3/tasks/{id}/");

    Ok(Export {
        pulp_href: format!("{}{}/", exporter.pulp_href, id),
        pulp_id: id,
        task: task_href,
        exported_resources: exported,
        output_file_info: file_info,
        created_at: chrono::Utc::now(),
    })
}

/// Validate that an export at `path` can be imported (checks manifest integrity).
pub fn validate_import(path: &str) -> Result<(), ArtifactsError> {
    if path.is_empty() {
        return Err(ArtifactsError::ExportError(
            "import path must not be empty".into(),
        ));
    }
    // In production: read manifest, verify SHA256 checksums.
    info!(path, "import validation (stub)");
    Ok(())
}
