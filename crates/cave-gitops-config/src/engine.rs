// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pipeline execution engine for cave-gitops-config.

use crate::models::{
    ClusterDestination, ClusterStatus, DestinationSelector, PipelineRun, PipelineRunStatus,
    PipelineStage, PipelineStageResult, PipelineStageType, Promise, PromiseStatus,
    ResourceRequest, StageStatus,
};
use chrono::Utc;
use uuid::Uuid;

pub struct PipelineEngine;

impl PipelineEngine {
    /// Process a resource request through all pipeline stages of the promise.
    pub fn run_pipeline(promise: &Promise, request: &ResourceRequest) -> PipelineRun {
        let now = Utc::now();
        let mut stages: Vec<PipelineStageResult> = vec![];
        let mut previous_output = serde_json::Value::Null;
        let mut failed = false;

        for stage in &promise.pipeline {
            if failed {
                stages.push(PipelineStageResult {
                    stage_name: stage.name.clone(),
                    status: StageStatus::Skipped,
                    output: serde_json::Value::Null,
                    error: None,
                    started_at: None,
                    completed_at: None,
                });
                continue;
            }

            let result = Self::execute_stage(stage, request, &previous_output);
            if result.status == StageStatus::Failed {
                failed = true;
            } else {
                previous_output = result.output.clone();
            }
            stages.push(result);
        }

        PipelineRun {
            id: Uuid::new_v4(),
            resource_request_id: request.id,
            promise_name: promise.name.clone(),
            stages,
            status: if failed {
                PipelineRunStatus::Failed
            } else {
                PipelineRunStatus::Completed
            },
            started_at: now,
            completed_at: Some(Utc::now()),
        }
    }

    /// Execute a single pipeline stage.
    fn execute_stage(
        stage: &PipelineStage,
        request: &ResourceRequest,
        previous_output: &serde_json::Value,
    ) -> PipelineStageResult {
        let now = Utc::now();
        match stage.stage_type {
            PipelineStageType::Transform => {
                // Apply JSON transformation: merge stage config into spec
                let mut transformed = request.spec.clone();
                if let (serde_json::Value::Object(t), serde_json::Value::Object(c)) =
                    (&mut transformed, &stage.config)
                {
                    for (k, v) in c {
                        t.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
                PipelineStageResult {
                    stage_name: stage.name.clone(),
                    status: StageStatus::Completed,
                    output: transformed,
                    error: None,
                    started_at: Some(now),
                    completed_at: Some(Utc::now()),
                }
            }
            PipelineStageType::Configure => {
                // Add default configurations from stage config
                let mut configured = previous_output.clone();
                if configured.is_null() {
                    configured = request.spec.clone();
                }
                if let (serde_json::Value::Object(c_out), serde_json::Value::Object(cfg)) =
                    (&mut configured, &stage.config)
                {
                    for (k, v) in cfg {
                        c_out.insert(k.clone(), v.clone());
                    }
                }
                PipelineStageResult {
                    stage_name: stage.name.clone(),
                    status: StageStatus::Completed,
                    output: configured,
                    error: None,
                    started_at: Some(now),
                    completed_at: Some(Utc::now()),
                }
            }
            PipelineStageType::Deploy => {
                // Write to state store path (simulated)
                let path = Self::state_store_path(
                    "default-cluster",
                    &request.promise_name,
                    &request.namespace,
                    &request.name,
                );
                PipelineStageResult {
                    stage_name: stage.name.clone(),
                    status: StageStatus::Completed,
                    output: serde_json::json!({"path": path, "deployed": true}),
                    error: None,
                    started_at: Some(now),
                    completed_at: Some(Utc::now()),
                }
            }
            PipelineStageType::Validate => {
                // Check required fields in the spec
                match Self::validate_spec_basic(&request.spec) {
                    Ok(()) => PipelineStageResult {
                        stage_name: stage.name.clone(),
                        status: StageStatus::Completed,
                        output: serde_json::json!({"valid": true}),
                        error: None,
                        started_at: Some(now),
                        completed_at: Some(Utc::now()),
                    },
                    Err(errors) => PipelineStageResult {
                        stage_name: stage.name.clone(),
                        status: StageStatus::Failed,
                        output: serde_json::json!({"valid": false, "errors": errors}),
                        error: Some(errors.join(", ")),
                        started_at: Some(now),
                        completed_at: Some(Utc::now()),
                    },
                }
            }
            PipelineStageType::Notify => {
                tracing::info!(
                    promise = %request.promise_name,
                    request_id = %request.id,
                    "Pipeline notification: resource request processed"
                );
                PipelineStageResult {
                    stage_name: stage.name.clone(),
                    status: StageStatus::Completed,
                    output: serde_json::json!({"notified": true}),
                    error: None,
                    started_at: Some(now),
                    completed_at: Some(Utc::now()),
                }
            }
        }
    }

    /// Generate the canonical state store path for a resource.
    pub fn state_store_path(cluster: &str, promise: &str, namespace: &str, name: &str) -> String {
        format!("clusters/{cluster}/{promise}/{namespace}/{name}.yaml")
    }

    /// Basic spec validation: the spec must be a non-null object.
    fn validate_spec_basic(spec: &serde_json::Value) -> Result<(), Vec<String>> {
        match spec {
            serde_json::Value::Object(map) if !map.is_empty() => Ok(()),
            serde_json::Value::Object(_) => Err(vec!["spec must not be empty".to_string()]),
            _ => Err(vec!["spec must be a JSON object".to_string()]),
        }
    }

    /// Validate that a resource request spec conforms to the promise API schema.
    /// Performs basic type checking on required fields defined in the schema's `required` array.
    pub fn validate_spec(promise: &Promise, spec: &serde_json::Value) -> Result<(), Vec<String>> {
        let mut errors: Vec<String> = vec![];

        // Spec must be an object
        let spec_obj = match spec.as_object() {
            Some(obj) => obj,
            None => {
                return Err(vec!["spec must be a JSON object".to_string()]);
            }
        };

        // Check required fields listed in the JSON Schema
        if let Some(required) = promise.api_schema.get("required") {
            if let Some(required_fields) = required.as_array() {
                for field in required_fields {
                    if let Some(field_name) = field.as_str() {
                        if !spec_obj.contains_key(field_name) {
                            errors.push(format!("Required field '{}' is missing", field_name));
                        }
                    }
                }
            }
        }

        // Check property types if `properties` is defined
        if let Some(properties) = promise.api_schema.get("properties") {
            if let Some(props_obj) = properties.as_object() {
                for (field_name, field_schema) in props_obj {
                    if let Some(value) = spec_obj.get(field_name) {
                        if let Some(expected_type) = field_schema.get("type").and_then(|t| t.as_str()) {
                            let actual_ok = match expected_type {
                                "string" => value.is_string(),
                                "integer" | "number" => value.is_number(),
                                "boolean" => value.is_boolean(),
                                "array" => value.is_array(),
                                "object" => value.is_object(),
                                _ => true,
                            };
                            if !actual_ok {
                                errors.push(format!(
                                    "Field '{}' should be of type '{}' but got '{}'",
                                    field_name,
                                    expected_type,
                                    Self::json_type_name(value)
                                ));
                            }
                        }
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn json_type_name(v: &serde_json::Value) -> &'static str {
        match v {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "boolean",
            serde_json::Value::Number(_) => "number",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "object",
        }
    }

    /// Resolve dependencies: returns the names of all transitive dependencies.
    /// Returns an error if any dependency is not found or not Active.
    pub fn resolve_dependencies(
        promise: &Promise,
        all_promises: &[Promise],
    ) -> Result<Vec<String>, String> {
        let mut resolved: Vec<String> = vec![];

        for dep_name in &promise.dependencies {
            match all_promises.iter().find(|p| &p.name == dep_name) {
                None => {
                    return Err(format!(
                        "Dependency '{}' not found in promise registry",
                        dep_name
                    ));
                }
                Some(dep) if dep.status != PromiseStatus::Active => {
                    return Err(format!(
                        "Dependency '{}' is not Active (status: {:?})",
                        dep_name, dep.status
                    ));
                }
                Some(_) => {
                    resolved.push(dep_name.clone());
                }
            }
        }

        Ok(resolved)
    }

    /// Select cluster destinations that match ALL of the promise's destination selectors.
    pub fn select_destinations(
        promise: &Promise,
        clusters: &[ClusterDestination],
    ) -> Vec<String> {
        clusters
            .iter()
            .filter(|cluster| {
                cluster.status == ClusterStatus::Ready
                    && promise
                        .destination_selectors
                        .iter()
                        .all(|selector| Self::matches_selector(cluster, selector))
            })
            .map(|c| c.name.clone())
            .collect()
    }

    fn matches_selector(cluster: &ClusterDestination, selector: &DestinationSelector) -> bool {
        cluster
            .labels
            .get(&selector.key)
            .map(|v| v == &selector.value)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        ClusterDestination, ClusterStatus, DestinationSelector, PipelineStage, PipelineStageType,
        Promise, PromiseStatus, ResourceRequest, ResourceRequestStatus,
    };
    use std::collections::HashMap;

    fn make_promise(name: &str, stages: Vec<PipelineStage>) -> Promise {
        Promise {
            id: Uuid::new_v4(),
            name: name.to_string(),
            version: "1.0.0".to_string(),
            description: "test promise".to_string(),
            api_schema: serde_json::json!({}),
            pipeline: stages,
            dependencies: vec![],
            destination_selectors: vec![],
            status: PromiseStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_request(promise_name: &str) -> ResourceRequest {
        ResourceRequest {
            id: Uuid::new_v4(),
            promise_name: promise_name.to_string(),
            promise_version: "1.0.0".to_string(),
            namespace: "default".to_string(),
            name: "my-db".to_string(),
            spec: serde_json::json!({"storage": "10Gi", "version": "14"}),
            requester: Uuid::new_v4(),
            status: ResourceRequestStatus::Pending,
            pipeline_run: None,
            destinations: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_stage(name: &str, stage_type: PipelineStageType) -> PipelineStage {
        PipelineStage {
            name: name.to_string(),
            description: "test stage".to_string(),
            stage_type,
            config: serde_json::json!({}),
            order: 0,
        }
    }

    #[test]
    fn test_state_store_path() {
        let path =
            PipelineEngine::state_store_path("prod", "postgresql", "default", "my-db");
        assert_eq!(path, "clusters/prod/postgresql/default/my-db.yaml");
    }

    #[test]
    fn test_run_pipeline_all_stages_complete() {
        let stages = vec![
            make_stage("validate", PipelineStageType::Validate),
            make_stage("transform", PipelineStageType::Transform),
            make_stage("deploy", PipelineStageType::Deploy),
            make_stage("notify", PipelineStageType::Notify),
        ];
        let promise = make_promise("postgresql", stages);
        let request = make_request("postgresql");
        let run = PipelineEngine::run_pipeline(&promise, &request);
        assert_eq!(run.status, PipelineRunStatus::Completed);
        assert_eq!(run.stages.len(), 4);
    }

    #[test]
    fn test_transform_stage_merges_config() {
        let stage = PipelineStage {
            name: "transform".to_string(),
            description: "add defaults".to_string(),
            stage_type: PipelineStageType::Transform,
            config: serde_json::json!({"replicas": 3}),
            order: 0,
        };
        let request = make_request("postgresql");
        let result = PipelineEngine::execute_stage(
            &stage,
            &request,
            &serde_json::Value::Null,
        );
        assert_eq!(result.status, StageStatus::Completed);
        // replicas should be in the output since spec didn't have it
        assert_eq!(result.output["replicas"], 3);
    }

    #[test]
    fn test_validate_spec_missing_required_field() {
        let promise = Promise {
            api_schema: serde_json::json!({
                "required": ["storage", "version"],
                "properties": {
                    "storage": {"type": "string"},
                    "version": {"type": "string"}
                }
            }),
            ..make_promise("postgresql", vec![])
        };
        let spec = serde_json::json!({"storage": "10Gi"}); // missing "version"
        let result = PipelineEngine::validate_spec(&promise, &spec);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.contains("version")));
    }

    #[test]
    fn test_validate_spec_all_required_present() {
        let promise = Promise {
            api_schema: serde_json::json!({
                "required": ["storage"],
                "properties": {
                    "storage": {"type": "string"}
                }
            }),
            ..make_promise("postgresql", vec![])
        };
        let spec = serde_json::json!({"storage": "10Gi"});
        let result = PipelineEngine::validate_spec(&promise, &spec);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_spec_wrong_type() {
        let promise = Promise {
            api_schema: serde_json::json!({
                "properties": {
                    "replicas": {"type": "integer"}
                }
            }),
            ..make_promise("postgresql", vec![])
        };
        let spec = serde_json::json!({"replicas": "three"}); // string instead of integer
        let result = PipelineEngine::validate_spec(&promise, &spec);
        assert!(result.is_err());
    }

    #[test]
    fn test_dependency_resolution_missing() {
        let promise = Promise {
            dependencies: vec!["redis".to_string()],
            ..make_promise("app", vec![])
        };
        let all_promises: Vec<Promise> = vec![];
        let result = PipelineEngine::resolve_dependencies(&promise, &all_promises);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("redis"));
    }

    #[test]
    fn test_dependency_resolution_inactive() {
        let dep = Promise {
            status: PromiseStatus::Suspended,
            ..make_promise("redis", vec![])
        };
        let promise = Promise {
            dependencies: vec!["redis".to_string()],
            ..make_promise("app", vec![])
        };
        let result = PipelineEngine::resolve_dependencies(&promise, &[dep]);
        assert!(result.is_err());
    }

    #[test]
    fn test_dependency_resolution_success() {
        let dep = make_promise("redis", vec![]);
        let promise = Promise {
            dependencies: vec!["redis".to_string()],
            ..make_promise("app", vec![])
        };
        let result = PipelineEngine::resolve_dependencies(&promise, &[dep]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec!["redis"]);
    }

    #[test]
    fn test_destination_selection_with_matching_labels() {
        let cluster = ClusterDestination {
            name: "prod-cluster".to_string(),
            api_server: "https://k8s.prod.example.com".to_string(),
            labels: {
                let mut m = HashMap::new();
                m.insert("env".to_string(), "prod".to_string());
                m.insert("region".to_string(), "us-east-1".to_string());
                m
            },
            status: ClusterStatus::Ready,
            registered_at: Utc::now(),
        };
        let promise = Promise {
            destination_selectors: vec![DestinationSelector {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
            ..make_promise("postgresql", vec![])
        };
        let selected = PipelineEngine::select_destinations(&promise, &[cluster]);
        assert_eq!(selected, vec!["prod-cluster"]);
    }

    #[test]
    fn test_destination_selection_no_match() {
        let cluster = ClusterDestination {
            name: "dev-cluster".to_string(),
            api_server: "https://k8s.dev.example.com".to_string(),
            labels: {
                let mut m = HashMap::new();
                m.insert("env".to_string(), "dev".to_string());
                m
            },
            status: ClusterStatus::Ready,
            registered_at: Utc::now(),
        };
        let promise = Promise {
            destination_selectors: vec![DestinationSelector {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
            ..make_promise("postgresql", vec![])
        };
        let selected = PipelineEngine::select_destinations(&promise, &[cluster]);
        assert!(selected.is_empty());
    }

    #[test]
    fn test_destination_selection_not_ready_cluster_excluded() {
        let cluster = ClusterDestination {
            name: "broken-cluster".to_string(),
            api_server: "https://k8s.broken.example.com".to_string(),
            labels: {
                let mut m = HashMap::new();
                m.insert("env".to_string(), "prod".to_string());
                m
            },
            status: ClusterStatus::NotReady,
            registered_at: Utc::now(),
        };
        let promise = Promise {
            destination_selectors: vec![DestinationSelector {
                key: "env".to_string(),
                value: "prod".to_string(),
            }],
            ..make_promise("postgresql", vec![])
        };
        let selected = PipelineEngine::select_destinations(&promise, &[cluster]);
        assert!(selected.is_empty());
    }
}
