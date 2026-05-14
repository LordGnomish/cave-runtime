// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiSpec {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub format: SpecFormat,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub published_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpecFormat {
    OpenApi3,
    OpenApi2,
    AsyncApi,
    GraphQL,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpecDiff {
    pub old_version: String,
    pub new_version: String,
    pub breaking_changes: Vec<String>,
    pub additions: Vec<String>,
    pub removals: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_spec(version: &str) -> ApiSpec {
        ApiSpec {
            id: Uuid::new_v4(),
            name: "my-api".to_string(),
            version: version.to_string(),
            format: SpecFormat::OpenApi3,
            content: "{}".to_string(),
            created_at: Utc::now(),
            published_by: "alice".to_string(),
        }
    }

    #[test]
    fn test_api_spec_roundtrip() {
        let spec = make_spec("1.0.0");
        let json = serde_json::to_string(&spec).unwrap();
        let decoded: ApiSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, decoded);
    }

    #[test]
    fn test_spec_format_openapi2_roundtrip() {
        let fmt = SpecFormat::OpenApi2;
        let json = serde_json::to_string(&fmt).unwrap();
        assert_eq!(json, "\"open_api2\"");
        let decoded: SpecFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, decoded);
    }

    #[test]
    fn test_spec_format_graphql_roundtrip() {
        let fmt = SpecFormat::GraphQL;
        let json = serde_json::to_string(&fmt).unwrap();
        let decoded: SpecFormat = serde_json::from_str(&json).unwrap();
        assert_eq!(fmt, decoded);
    }

    #[test]
    fn test_spec_diff_roundtrip() {
        let diff = SpecDiff {
            old_version: "1.0.0".to_string(),
            new_version: "2.0.0".to_string(),
            breaking_changes: vec!["removed /users endpoint".to_string()],
            additions: vec!["added /accounts endpoint".to_string()],
            removals: vec![],
        };
        let json = serde_json::to_string(&diff).unwrap();
        let decoded: SpecDiff = serde_json::from_str(&json).unwrap();
        assert_eq!(diff, decoded);
    }

    #[test]
    fn test_api_spec_async_api_roundtrip() {
        let mut spec = make_spec("0.1.0");
        spec.format = SpecFormat::AsyncApi;
        let json = serde_json::to_string(&spec).unwrap();
        let decoded: ApiSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec.format, decoded.format);
    }
}
