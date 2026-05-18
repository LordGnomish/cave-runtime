// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::harbor::models::{GCPolicy, GCResult, ImageMetadata};
use chrono::Utc;

/// Determine which images should be garbage collected
/// Rules: images not in keep_tags, older than keep_days, beyond keep_last_n
pub fn find_gc_candidates(images: &[ImageMetadata], policy: &GCPolicy) -> GCResult {
    let now = Utc::now();
    let cutoff = now - chrono::Duration::days(policy.keep_days as i64);

    // Sort by pushed_at descending to identify recency
    let mut sorted: Vec<&ImageMetadata> = images.iter().collect();
    sorted.sort_by(|a, b| b.pushed_at.cmp(&a.pushed_at));

    let keep_tag_set: std::collections::HashSet<&str> =
        policy.keep_tags.iter().map(|s| s.as_str()).collect();

    let mut candidates: Vec<uuid::Uuid> = vec![];

    for (i, img) in sorted.iter().enumerate() {
        if keep_tag_set.contains(img.tag.as_str()) {
            continue;
        }
        if i < policy.keep_last_n {
            continue;
        }
        if img.pushed_at > cutoff {
            continue;
        }
        candidates.push(img.id);
    }

    let bytes_to_free: u64 = candidates
        .iter()
        .filter_map(|id| images.iter().find(|img| &img.id == id))
        .map(|img| img.size_bytes)
        .sum();

    GCResult {
        images_to_remove: candidates.len(),
        bytes_to_free,
        candidates,
    }
}

/// Check if an image is eligible for deletion under a policy
pub fn should_gc(image: &ImageMetadata, policy: &GCPolicy, rank: usize) -> bool {
    let now = Utc::now();
    let cutoff = now - chrono::Duration::days(policy.keep_days as i64);
    if policy.keep_tags.iter().any(|t| t == &image.tag) {
        return false;
    }
    if rank < policy.keep_last_n {
        return false;
    }
    if image.pushed_at > cutoff {
        return false;
    }
    true
}

/// Parse a Docker image reference into (repository, tag)
pub fn parse_image_ref(reference: &str) -> Option<(String, String)> {
    if let Some((repo, tag)) = reference.rsplit_once(':') {
        Some((repo.to_string(), tag.to_string()))
    } else {
        None
    }
}

/// Compute the total size of a set of images
pub fn total_size_bytes(images: &[ImageMetadata]) -> u64 {
    images.iter().map(|i| i.size_bytes).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_image(tag: &str, size: u64, days_old: i64) -> ImageMetadata {
        ImageMetadata {
            id: Uuid::new_v4(),
            repository: "myrepo".to_string(),
            tag: tag.to_string(),
            digest: format!("sha256:{:016x}", size),
            size_bytes: size,
            pushed_at: Utc::now() - chrono::Duration::days(days_old),
            pulled_at: None,
            labels: HashMap::new(),
        }
    }

    fn default_policy() -> GCPolicy {
        GCPolicy {
            keep_last_n: 3,
            keep_days: 30,
            keep_tags: vec!["latest".to_string()],
        }
    }

    #[test]
    fn test_find_gc_candidates_empty() {
        let policy = default_policy();
        let result = find_gc_candidates(&[], &policy);
        assert_eq!(result.candidates.len(), 0);
        assert_eq!(result.bytes_to_free, 0);
        assert_eq!(result.images_to_remove, 0);
    }

    #[test]
    fn test_find_gc_candidates_keep_last_n() {
        // 5 images all older than keep_days, no protected tags
        // keep_last_n=3, so oldest 2 should be candidates
        let policy = GCPolicy {
            keep_last_n: 3,
            keep_days: 1,
            keep_tags: vec![],
        };
        let images = vec![
            make_image("v1", 100, 200),
            make_image("v2", 100, 180),
            make_image("v3", 100, 150),
            make_image("v4", 100, 120),
            make_image("v5", 100, 100),
        ];
        let result = find_gc_candidates(&images, &policy);
        // v1 and v2 are oldest (beyond rank 3 and older than 1 day)
        assert_eq!(result.images_to_remove, 2);
        assert_eq!(result.bytes_to_free, 200);
    }

    #[test]
    fn test_find_gc_candidates_keep_protected_tag() {
        let policy = GCPolicy {
            keep_last_n: 0,
            keep_days: 1,
            keep_tags: vec!["latest".to_string()],
        };
        let images = vec![
            make_image("latest", 500, 200),
            make_image("v1", 100, 200),
        ];
        let result = find_gc_candidates(&images, &policy);
        // "latest" should never be a candidate
        let candidate_ids: Vec<_> = result.candidates.iter().collect();
        let latest_id = images.iter().find(|i| i.tag == "latest").unwrap().id;
        assert!(!candidate_ids.contains(&&latest_id));
        assert_eq!(result.images_to_remove, 1);
    }

    #[test]
    fn test_find_gc_candidates_keep_recent() {
        let policy = GCPolicy {
            keep_last_n: 0,
            keep_days: 30,
            keep_tags: vec![],
        };
        // Image pushed 1 hour ago - should not be GC'd
        let mut images = vec![make_image("v1", 100, 200)]; // old
        let recent = ImageMetadata {
            id: Uuid::new_v4(),
            repository: "myrepo".to_string(),
            tag: "v2".to_string(),
            digest: "sha256:recent".to_string(),
            size_bytes: 200,
            pushed_at: Utc::now() - chrono::Duration::hours(1),
            pulled_at: None,
            labels: HashMap::new(),
        };
        images.push(recent.clone());
        let result = find_gc_candidates(&images, &policy);
        assert!(!result.candidates.contains(&recent.id));
    }

    #[test]
    fn test_should_gc_old_unprotected() {
        let policy = default_policy();
        let img = make_image("v1", 100, 100);
        assert!(should_gc(&img, &policy, 5));
    }

    #[test]
    fn test_should_gc_protected_tag() {
        let policy = default_policy();
        let img = make_image("latest", 100, 100);
        assert!(!should_gc(&img, &policy, 5));
    }

    #[test]
    fn test_should_gc_within_keep_n() {
        let policy = default_policy();
        let img = make_image("v1", 100, 100);
        assert!(!should_gc(&img, &policy, 0));
    }

    #[test]
    fn test_parse_image_ref_with_tag() {
        let result = parse_image_ref("nginx:1.21");
        assert_eq!(result, Some(("nginx".to_string(), "1.21".to_string())));
    }

    #[test]
    fn test_parse_image_ref_no_tag() {
        let result = parse_image_ref("nginx");
        assert_eq!(result, None);
    }

    #[test]
    fn test_total_size_bytes() {
        let images = vec![
            make_image("v1", 100, 10),
            make_image("v2", 200, 10),
            make_image("v3", 300, 10),
        ];
        assert_eq!(total_size_bytes(&images), 600);
    }

    #[test]
    fn test_total_size_bytes_empty() {
        assert_eq!(total_size_bytes(&[]), 0);
    }

    #[test]
    fn test_parse_image_ref_with_registry() {
        let result = parse_image_ref("registry.example.com/myimage:v2.0");
        assert_eq!(
            result,
            Some(("registry.example.com/myimage".to_string(), "v2.0".to_string()))
        );
    }
}
