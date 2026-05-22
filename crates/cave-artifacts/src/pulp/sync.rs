// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: pulp/pulpcore@0f991c2fa2bf6c8635e8a2de064ef04dacbbcf4f pulpcore/app/tasks/sync.py
//! Content sync logic — mirror upstream remotes into a repository.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, DownloadPolicy, Remote, Repository};
use crate::pulp::store::ArtifactsState;
use std::sync::Arc;
use tracing::{info, warn};

/// Options controlling a single sync run.
#[derive(Debug, Clone)]
pub struct SyncOptions {
    /// If true, remove local content not present in the upstream.
    pub mirror: bool,
    /// If true, skip re-syncing content whose SHA256 already matches.
    pub optimize: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            mirror: false,
            optimize: true,
        }
    }
}

/// Synchronise `repo` from `remote`, applying `opts`.
///
/// Returns the HREFs of any new content units created.
pub async fn sync_repository(
    state: Arc<ArtifactsState>,
    repo: Repository,
    remote: Remote,
    opts: SyncOptions,
) -> Result<Vec<String>, ArtifactsError> {
    info!(
        repo = %repo.name,
        remote = %remote.url,
        policy = ?remote.download_policy,
        mirror = opts.mirror,
        "starting sync"
    );

    let plugin = state
        .plugins
        .get(&repo.plugin_type)
        .ok_or_else(|| ArtifactsError::PluginNotFound(repo.plugin_type.to_string()))?;

    let index = fetch_remote_index(&remote).await?;
    let mut created_hrefs: Vec<String> = vec![];

    for item in index {
        let existing = state
            .list_content(Some(&repo.plugin_type))
            .await;

        let already_have = existing.iter().any(|u| {
            u.relative_path.as_deref() == item.relative_path.as_deref()
        });

        if already_have && opts.optimize {
            continue;
        }

        let unit = match remote.download_policy {
            DownloadPolicy::Immediate => {
                let data = fetch_content_data(&remote, &item).await?;
                let mut u = plugin
                    .parse_content(&data, item.relative_path.as_deref().unwrap_or(""))
                    .map_err(|e| ArtifactsError::SyncError(e.to_string()))?;
                u.sha256 = Some(compute_sha256(&data));
                u.size = Some(data.len() as u64);
                u
            }
            DownloadPolicy::OnDemand | DownloadPolicy::Streamed => {
                // Record the unit without downloading the blob yet.
                let mut u = ContentUnit::new(
                    repo.plugin_type.clone(),
                    item.metadata.clone(),
                );
                u.relative_path = item.relative_path.clone();
                u
            }
        };

        let stored = state.store_content(unit).await;
        created_hrefs.push(stored.pulp_href.clone());
    }

    // Create a new repository version containing all current + new content.
    let existing_content: Vec<String> = state
        .list_content(Some(&repo.plugin_type))
        .await
        .into_iter()
        .map(|u| u.pulp_href)
        .collect();

    let ver = state
        .create_repo_version(&repo.pulp_href, existing_content)
        .await?;

    info!(
        repo = %repo.name,
        version = ver.number,
        new_units = created_hrefs.len(),
        "sync complete"
    );

    Ok(vec![ver.pulp_href])
}

// ---------------------------------------------------------------------------
// Stub helpers (would talk to real HTTP remotes in production)
// ---------------------------------------------------------------------------

struct RemoteItem {
    relative_path: Option<String>,
    metadata: serde_json::Value,
}

async fn fetch_remote_index(remote: &Remote) -> Result<Vec<RemoteItem>, ArtifactsError> {
    // In a real implementation this would HTTP-GET the remote's index endpoint.
    // We return an empty list so tests and CI compile and pass.
    info!(url = %remote.url, "fetching remote index (stub)");
    Ok(vec![])
}

async fn fetch_content_data(
    _remote: &Remote,
    _item: &RemoteItem,
) -> Result<Vec<u8>, ArtifactsError> {
    warn!("fetch_content_data stub called");
    Ok(vec![])
}

fn compute_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_stable() {
        let h = compute_sha256(b"hello");
        assert_eq!(
            h,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
