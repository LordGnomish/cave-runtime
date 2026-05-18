// SPDX-License-Identifier: AGPL-3.0-or-later
//! GitHub release tracker — fetches latest releases and generates triage items.

use crate::projects::TrackedProject;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub html_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpstreamChange {
    pub project: String,
    pub cave_module: String,
    pub version: String,
    pub changelog: String,
    pub release_url: String,
    pub detected_at: DateTime<Utc>,
    pub triage: Option<String>, // ADOPT / WATCH / SKIP — set by AI triage
    pub triage_reason: Option<String>,
}

pub struct UpstreamTracker {
    client: Client,
    github_token: Option<String>,
}

impl UpstreamTracker {
    pub fn new(github_token: Option<String>) -> Self {
        Self {
            client: Client::new(),
            github_token,
        }
    }

    /// Fetch latest releases for a tracked project.
    pub async fn check_releases(
        &self,
        project: &TrackedProject,
    ) -> Result<Vec<GitHubRelease>, String> {
        let url = format!(
            "https://api.github.com/repos/{}/releases?per_page=5",
            project.github_repo
        );

        let mut request = self
            .client
            .get(&url)
            .header("User-Agent", "cave-upstream-tracker/0.1")
            .header("Accept", "application/vnd.github+json");

        if let Some(ref token) = self.github_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request.send().await.map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "GitHub API error {}: {}",
                response.status(),
                project.github_repo
            ));
        }

        let releases: Vec<GitHubRelease> = response.json().await.map_err(|e| e.to_string())?;
        info!(
            project = project.name,
            releases = releases.len(),
            "Fetched releases"
        );

        Ok(releases)
    }

    /// Check all tracked projects for new releases.
    pub async fn check_all(
        &self,
        projects: &[TrackedProject],
        since: DateTime<Utc>,
    ) -> Vec<UpstreamChange> {
        let mut changes = Vec::new();

        for project in projects {
            match self.check_releases(project).await {
                Ok(releases) => {
                    for release in releases {
                        if let Some(published) = release.published_at {
                            if published > since {
                                changes.push(UpstreamChange {
                                    project: project.name.to_string(),
                                    cave_module: project.cave_module.to_string(),
                                    version: release.tag_name,
                                    changelog: release.body.unwrap_or_default(),
                                    release_url: release.html_url,
                                    detected_at: Utc::now(),
                                    triage: None,
                                    triage_reason: None,
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(project = project.name, error = %e, "Failed to check releases");
                }
            }
        }

        info!(
            total_changes = changes.len(),
            "Upstream check complete"
        );
        changes
    }
}
