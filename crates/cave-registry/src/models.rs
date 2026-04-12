use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageMetadata {
    pub id: Uuid,
    pub repository: String,
    pub tag: String,
    pub digest: String,
    pub size_bytes: u64,
    pub pushed_at: DateTime<Utc>,
    pub pulled_at: Option<DateTime<Utc>>,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GCPolicy {
    pub keep_last_n: usize,
    pub keep_days: u32,
    pub keep_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GCResult {
    pub candidates: Vec<Uuid>,
    pub bytes_to_free: u64,
    pub images_to_remove: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_image(tag: &str, size: u64) -> ImageMetadata {
        ImageMetadata {
            id: Uuid::new_v4(),
            repository: "myrepo".to_string(),
            tag: tag.to_string(),
            digest: format!("sha256:{:064x}", size),
            size_bytes: size,
            pushed_at: Utc::now(),
            pulled_at: None,
            labels: HashMap::new(),
        }
    }

    #[test]
    fn test_image_metadata_serde_roundtrip() {
        let img = make_image("latest", 1024);
        let json = serde_json::to_string(&img).unwrap();
        let back: ImageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(img, back);
    }

    #[test]
    fn test_image_metadata_with_pulled_at() {
        let mut img = make_image("v1.0", 2048);
        img.pulled_at = Some(Utc::now());
        let json = serde_json::to_string(&img).unwrap();
        let back: ImageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(img, back);
    }

    #[test]
    fn test_image_metadata_with_labels() {
        let mut img = make_image("v2.0", 512);
        img.labels.insert("env".to_string(), "prod".to_string());
        let json = serde_json::to_string(&img).unwrap();
        let back: ImageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(img, back);
        assert_eq!(back.labels.get("env").unwrap(), "prod");
    }

    #[test]
    fn test_gc_policy_serde_roundtrip() {
        let policy = GCPolicy {
            keep_last_n: 5,
            keep_days: 30,
            keep_tags: vec!["latest".to_string(), "stable".to_string()],
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: GCPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, back);
    }

    #[test]
    fn test_gc_result_serde_roundtrip() {
        let result = GCResult {
            candidates: vec![Uuid::new_v4(), Uuid::new_v4()],
            bytes_to_free: 4096,
            images_to_remove: 2,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: GCResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, back);
    }

    #[test]
    fn test_gc_result_empty() {
        let result = GCResult {
            candidates: vec![],
            bytes_to_free: 0,
            images_to_remove: 0,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: GCResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, back);
    }

    #[test]
    fn test_image_digest_format() {
        let img = ImageMetadata {
            id: Uuid::new_v4(),
            repository: "repo".to_string(),
            tag: "tag".to_string(),
            digest: "sha256:abc123".to_string(),
            size_bytes: 100,
            pushed_at: Utc::now(),
            pulled_at: None,
            labels: HashMap::new(),
        };
        assert!(img.digest.starts_with("sha256:"));
    }

    #[test]
    fn test_gc_policy_empty_keep_tags() {
        let policy = GCPolicy {
            keep_last_n: 0,
            keep_days: 0,
            keep_tags: vec![],
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: GCPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(policy, back);
    }
}
