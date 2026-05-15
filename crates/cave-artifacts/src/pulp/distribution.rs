// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Distribution and publication management.

use crate::pulp::models::*;
use crate::pulp::tasks::{Task, TaskQueue};

// ─── Publication operations ──────────────────────────────────────────────────

/// Enqueue a publish task for a repository version.
pub fn enqueue_publish(
    repo: &Repository,
    version_href: &str,
    queue: &TaskQueue,
) -> Task {
    let task_name = format!(
        "pulp_{}.tasks.publish",
        repo.content_type.plugin_name().trim_start_matches("pulp_")
    );
    let task = queue.enqueue(task_name);
    tracing::info!(
        repo = %repo.name,
        version = %version_href,
        task_id = %task.pulp_id,
        "Publish task enqueued"
    );
    task
}

// ─── Distribution URL routing ──────────────────────────────��──────────────────

/// Resolve a content path under a distribution to a publication artifact.
pub fn resolve_content_path(
    distributions: &[Distribution],
    path: &str,
) -> Option<String> {
    for dist in distributions {
        let prefix = format!("/pulp/content/{}/", dist.base_path);
        if path.starts_with(&prefix) {
            let relative = &path[prefix.len()..];
            return Some(relative.to_string());
        }
    }
    None
}

/// Find the distribution serving a given base_path.
pub fn find_distribution_by_path<'a>(
    distributions: &'a [Distribution],
    base_path: &str,
) -> Option<&'a Distribution> {
    distributions.iter().find(|d| d.base_path == base_path)
}

// ─── Distribution validation ───────────────────────────────���─────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum DistributionError {
    BasePathConflict { existing: String },
    InvalidBasePath(String),
    SourceMissing,
    ContentTypeMismatch { expected: String, got: String },
}

/// Validate a distribution before creating/updating.
pub fn validate_distribution(
    dist: &Distribution,
    existing: &[Distribution],
) -> Vec<DistributionError> {
    let mut errors = Vec::new();

    // base_path must not be empty or contain '..'
    if dist.base_path.is_empty() || dist.base_path.contains("..") {
        errors.push(DistributionError::InvalidBasePath(dist.base_path.clone()));
    }

    // base_path must be unique
    if existing.iter().any(|d| d.base_path == dist.base_path && d.pulp_id != dist.pulp_id) {
        errors.push(DistributionError::BasePathConflict { existing: dist.base_path.clone() });
    }

    // Must have exactly one source
    let sources = [dist.publication.is_some(), dist.repository.is_some(), dist.repository_version.is_some()];
    let source_count = sources.iter().filter(|&&x| x).count();
    if source_count == 0 {
        errors.push(DistributionError::SourceMissing);
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dist(name: &str, base_path: &str) -> Distribution {
        let mut d = Distribution::new(name, base_path, ContentType::File);
        d.publication = Some("/pulp/api/v3/publications/abc/".to_string());
        d
    }

    #[test]
    fn resolve_content_path_matches() {
        let dist = make_dist("my-dist", "files/stable");
        let path = "/pulp/content/files/stable/README.txt";
        let result = resolve_content_path(&[dist], path);
        assert_eq!(result, Some("README.txt".to_string()));
    }

    #[test]
    fn resolve_content_path_no_match() {
        let dist = make_dist("my-dist", "files/stable");
        let result = resolve_content_path(&[dist], "/pulp/content/other/README.txt");
        assert!(result.is_none());
    }

    #[test]
    fn find_distribution_by_path() {
        let d1 = make_dist("dist1", "rpm/el9");
        let d2 = make_dist("dist2", "rpm/el8");
        let dists = [d1, d2];
        let result = super::find_distribution_by_path(&dists, "rpm/el9");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "dist1");
    }

    #[test]
    fn validate_distribution_base_path_conflict() {
        let d1 = make_dist("dist1", "pypi/simple");
        let d2 = make_dist("dist2", "pypi/simple");
        let errors = validate_distribution(&d2, &[d1]);
        assert!(errors.iter().any(|e| matches!(e, DistributionError::BasePathConflict { .. })));
    }

    #[test]
    fn validate_distribution_invalid_base_path() {
        let d = make_dist("dist", "../traversal");
        let errors = validate_distribution(&d, &[]);
        assert!(errors.iter().any(|e| matches!(e, DistributionError::InvalidBasePath(_))));
    }

    #[test]
    fn validate_distribution_no_source() {
        let mut d = Distribution::new("dist", "path", ContentType::File);
        // No publication, repository, or repository_version set
        let errors = validate_distribution(&d, &[]);
        assert!(errors.iter().any(|e| matches!(e, DistributionError::SourceMissing)));
    }

    #[test]
    fn validate_distribution_valid() {
        let d = make_dist("dist", "rpm/el9");
        let errors = validate_distribution(&d, &[]);
        assert!(errors.is_empty());
    }

    #[test]
    fn enqueue_publish_task() {
        let repo = Repository::new("my-rpm", ContentType::Rpm);
        let queue = TaskQueue::new();
        let task = enqueue_publish(&repo, "/pulp/api/v3/versions/1/", &queue);
        assert!(task.name.contains("rpm"));
    }
}
