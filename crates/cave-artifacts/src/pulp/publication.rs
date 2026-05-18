// SPDX-License-Identifier: AGPL-3.0-or-later
//! Publication workflow: create_publication triggers metadata generation.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{CreatePublicationRequest, Publication};
use crate::pulp::store::ArtifactsState;
use std::sync::Arc;
use tracing::info;

/// Create a publication for the latest (or specified) repository version.
///
/// The plugin is asked to generate repository-level metadata (index files,
/// Packages.gz, etc.) which gets attached to the publication.
pub async fn create_publication(
    state: Arc<ArtifactsState>,
    req: CreatePublicationRequest,
) -> Result<Publication, ArtifactsError> {
    let pub_ = state.create_publication(req).await?;

    // Retrieve the version's content units and ask the plugin to generate
    // metadata for them.
    let ver = state
        .get_repo_version(&pub_.repository_version)
        .await
        .ok_or_else(|| ArtifactsError::NotFound("repository version".into()))?;

    let units = {
        let mut out = vec![];
        for href in &ver.content_hrefs {
            if let Some(u) = state.get_content(href).await {
                out.push(u);
            }
        }
        out
    };

    let repo = state
        .get_repository(ver.repository.as_str())
        .await
        .ok_or_else(|| ArtifactsError::NotFound("repository".into()))?;

    if let Some(plugin) = state.plugins.get(&repo.plugin_type) {
        let meta = plugin.generate_metadata(&ver, &units);
        info!(
            pub_href = %pub_.pulp_href,
            content_units = units.len(),
            metadata_keys = meta.as_object().map(|o| o.len()).unwrap_or(0),
            "publication metadata generated"
        );
    }

    Ok(pub_)
}
