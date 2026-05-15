// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaffoldTemplate {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub language: String,
    pub category: TemplateCategory,
    pub variables: Vec<TemplateVariable>,
    pub created_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateCategory {
    Microservice,
    Library,
    Frontend,
    DataPipeline,
    InfraModule,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateVariable {
    pub name: String,
    pub description: String,
    pub var_type: VariableType,
    pub required: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    String,
    Boolean,
    Integer,
    Enum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScaffoldJob {
    pub id: Uuid,
    pub template_id: Uuid,
    pub parameters: HashMap<String, String>,
    pub status: JobStatus,
    pub output_repo: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_template() -> ScaffoldTemplate {
        ScaffoldTemplate {
            id: Uuid::new_v4(),
            name: "rust-service".to_string(),
            description: "A Rust microservice template".to_string(),
            language: "rust".to_string(),
            category: TemplateCategory::Microservice,
            variables: vec![
                TemplateVariable {
                    name: "service_name".to_string(),
                    description: "Name of the service".to_string(),
                    var_type: VariableType::String,
                    required: true,
                    default_value: None,
                },
            ],
            created_at: Utc::now(),
            tags: vec!["rust".to_string(), "backend".to_string()],
        }
    }

    #[test]
    fn test_template_serialization_roundtrip() {
        let template = make_template();
        let json = serde_json::to_string(&template).unwrap();
        let deserialized: ScaffoldTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(template, deserialized);
    }

    #[test]
    fn test_job_status_serialization() {
        let statuses = vec![
            JobStatus::Pending,
            JobStatus::Running,
            JobStatus::Completed,
            JobStatus::Failed,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let back: JobStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn test_template_category_serialization() {
        let json = serde_json::to_string(&TemplateCategory::DataPipeline).unwrap();
        assert_eq!(json, "\"data_pipeline\"");
        let back: TemplateCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TemplateCategory::DataPipeline);
    }

    #[test]
    fn test_scaffold_job_roundtrip() {
        let job = ScaffoldJob {
            id: Uuid::new_v4(),
            template_id: Uuid::new_v4(),
            parameters: {
                let mut m = HashMap::new();
                m.insert("service_name".to_string(), "my-svc".to_string());
                m
            },
            status: JobStatus::Pending,
            output_repo: Some("https://github.com/org/my-svc".to_string()),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&job).unwrap();
        let back: ScaffoldJob = serde_json::from_str(&json).unwrap();
        assert_eq!(job, back);
    }

    #[test]
    fn test_variable_type_serialization() {
        let types = vec![
            VariableType::String,
            VariableType::Boolean,
            VariableType::Integer,
            VariableType::Enum,
        ];
        for vt in types {
            let json = serde_json::to_string(&vt).unwrap();
            let back: VariableType = serde_json::from_str(&json).unwrap();
            assert_eq!(vt, back);
        }
    }
}
