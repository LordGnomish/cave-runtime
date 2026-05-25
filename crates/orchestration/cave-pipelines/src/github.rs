// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! GitHub integration: commit status updates and PR check runs.

use crate::models::RunStatus;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    pub token: String,
    pub owner: String,
    pub repo: String,
    /// Override for GitHub Enterprise (e.g. `"https://github.example.com/api/v3"`).
    pub base_url: Option<String>,
}

impl GitHubConfig {
    pub fn api_base(&self) -> &str {
        self.base_url.as_deref().unwrap_or("https://api.github.com")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommitState {
    Pending,
    Success,
    Failure,
    Error,
}

impl From<&RunStatus> for CommitState {
    fn from(s: &RunStatus) -> Self {
        match s {
            RunStatus::Pending | RunStatus::Running | RunStatus::WaitingApproval => {
                CommitState::Pending
            }
            RunStatus::Succeeded | RunStatus::Skipped => CommitState::Success,
            RunStatus::Failed | RunStatus::Cancelled => CommitState::Failure,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitStatusPayload {
    pub state: CommitState,
    pub target_url: Option<String>,
    pub description: Option<String>,
    pub context: String,
}

#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("GitHub API request failed: {0}")]
    Api(String),
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

pub struct GitHubClient {
    config: GitHubConfig,
    client: reqwest::Client,
}

impl GitHubClient {
    pub fn new(config: GitHubConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    /// POST /repos/{owner}/{repo}/statuses/{sha}
    pub async fn set_commit_status(
        &self,
        sha: &str,
        payload: &CommitStatusPayload,
    ) -> Result<(), GitHubError> {
        let url = format!(
            "{}/repos/{}/{}/statuses/{}",
            self.config.api_base(),
            self.config.owner,
            self.config.repo,
            sha,
        );
        info!(sha = %sha, state = ?payload.state, "setting GitHub commit status");
        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(payload)
            .send()
            .await
            .map_err(|e| GitHubError::Api(e.to_string()))?;
        Ok(())
    }

    /// POST /repos/{owner}/{repo}/check-runs
    pub async fn create_check_run(
        &self,
        name: &str,
        sha: &str,
        status: &RunStatus,
        run_id: Uuid,
    ) -> Result<(), GitHubError> {
        let url = format!(
            "{}/repos/{}/{}/check-runs",
            self.config.api_base(),
            self.config.owner,
            self.config.repo,
        );
        let conclusion: Option<&str> = match status {
            RunStatus::Succeeded => Some("success"),
            RunStatus::Failed => Some("failure"),
            RunStatus::Cancelled => Some("cancelled"),
            RunStatus::Skipped => Some("skipped"),
            _ => None,
        };
        let mut payload = serde_json::json!({
            "name": name,
            "head_sha": sha,
            "status": if conclusion.is_some() { "completed" } else { "in_progress" },
            "external_id": run_id.to_string(),
        });
        if let Some(c) = conclusion {
            payload["conclusion"] = serde_json::Value::String(c.to_string());
        }
        info!(sha = %sha, name = %name, "creating GitHub check run");
        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&payload)
            .send()
            .await
            .map_err(|e| GitHubError::Api(e.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(base_url: Option<&str>) -> GitHubConfig {
        GitHubConfig {
            token: "tok".to_string(),
            owner: "acme".to_string(),
            repo: "app".to_string(),
            base_url: base_url.map(String::from),
        }
    }

    #[test]
    fn test_api_base_defaults_to_github_com() {
        assert_eq!(cfg(None).api_base(), "https://api.github.com");
    }

    #[test]
    fn test_api_base_enterprise_override() {
        let c = cfg(Some("https://ghe.example.com/api/v3"));
        assert_eq!(c.api_base(), "https://ghe.example.com/api/v3");
    }

    #[test]
    fn test_commit_state_pending_statuses() {
        for s in [RunStatus::Pending, RunStatus::Running, RunStatus::WaitingApproval] {
            assert_eq!(CommitState::from(&s), CommitState::Pending);
        }
    }

    #[test]
    fn test_commit_state_success_statuses() {
        assert_eq!(CommitState::from(&RunStatus::Succeeded), CommitState::Success);
        assert_eq!(CommitState::from(&RunStatus::Skipped), CommitState::Success);
    }

    #[test]
    fn test_commit_state_failure_statuses() {
        assert_eq!(CommitState::from(&RunStatus::Failed), CommitState::Failure);
        assert_eq!(CommitState::from(&RunStatus::Cancelled), CommitState::Failure);
    }

    #[test]
    fn test_commit_status_payload_serializes() {
        let payload = CommitStatusPayload {
            state: CommitState::Pending,
            target_url: Some("https://cave.example.com/runs/123".to_string()),
            description: Some("Pipeline running".to_string()),
            context: "cave-pipelines/ci".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("pending"));
        assert!(json.contains("cave-pipelines/ci"));
    }
}
