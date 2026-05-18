// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cleanup policy evaluation: scans a repository's assets and returns the
//! set of asset IDs that match every configured criterion. The caller is
//! responsible for actually deleting them so dry-run / preview flows can
//! reuse the same selection logic.

use super::error::NexusError;
use super::models::{Asset, CleanupPolicy};
use super::store::NexusStore;
use chrono::{Duration, Utc};
use regex::Regex;
use uuid::Uuid;

/// Evaluate a cleanup policy against `repo_name`, returning the asset IDs
/// that match every configured criterion. With no criteria set, nothing
/// matches — Nexus' behaviour, to avoid accidental wipe-everything runs.
pub fn evaluate(
    store: &NexusStore,
    policy: &CleanupPolicy,
    repo_name: &str,
) -> Result<Vec<Uuid>, NexusError> {
    if policy.criteria.older_than_days.is_none()
        && policy.criteria.last_downloaded_days.is_none()
        && policy.criteria.regex.is_none()
    {
        return Ok(vec![]);
    }

    // If the policy is format-scoped, ensure the repository matches.
    if let Some(fmt) = policy.format {
        let repo = store.get_repository(repo_name)?;
        if repo.format != fmt {
            return Ok(vec![]);
        }
    }

    let regex = policy
        .criteria
        .regex
        .as_deref()
        .map(|r| Regex::new(r).map_err(|e| NexusError::InvalidRegex(e.to_string())))
        .transpose()?;

    let now = Utc::now();
    let assets = store.assets_in_repo(repo_name);
    let matched = assets
        .into_iter()
        .filter(|a| matches_criteria(a, policy, &regex, now))
        .map(|a| a.id)
        .collect();
    Ok(matched)
}

/// Run [`evaluate`] and delete every matched asset, returning the count.
pub fn apply(
    store: &NexusStore,
    policy: &CleanupPolicy,
    repo_name: &str,
) -> Result<usize, NexusError> {
    let ids = evaluate(store, policy, repo_name)?;
    let count = ids.len();
    for id in ids {
        store.delete_asset(id)?;
    }
    Ok(count)
}

fn matches_criteria(
    asset: &Asset,
    policy: &CleanupPolicy,
    regex: &Option<Regex>,
    now: chrono::DateTime<Utc>,
) -> bool {
    if let Some(days) = policy.criteria.older_than_days {
        let threshold = now - Duration::days(days as i64);
        if asset.created_at >= threshold {
            return false;
        }
    }
    if let Some(days) = policy.criteria.last_downloaded_days {
        let threshold = now - Duration::days(days as i64);
        let last = asset.last_downloaded.unwrap_or(asset.created_at);
        if last >= threshold {
            return false;
        }
    }
    if let Some(re) = regex {
        if !re.is_match(&asset.path) {
            return false;
        }
    }
    true
}
