// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gravitee API Designer — API-first studio with spec management, mock servers,
//! and automated quality scoring.

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct ApiDesignerStore {
    pub specs: HashMap<Uuid, ApiSpec>,
    pub quality_cache: HashMap<Uuid, ApiQualityScore>,
}

impl ApiDesignerStore {
    pub fn new() -> Self {
        Self {
            specs: HashMap::new(),
            quality_cache: HashMap::new(),
        }
    }

    pub fn create_spec(&mut self, req: CreateSpecRequest) -> ApiSpec {
        let now = chrono::Utc::now();
        let spec = ApiSpec {
            id: Uuid::new_v4(),
            name: req.name,
            version: req.version,
            format: req.format,
            content: req.content,
            description: req.description,
            tags: req.tags.unwrap_or_default(),
            created_at: now,
            updated_at: now,
        };
        self.specs.insert(spec.id, spec.clone());
        spec
    }

    pub fn update_spec(&mut self, id: Uuid, req: UpdateSpecRequest) -> Option<ApiSpec> {
        let spec = self.specs.get_mut(&id)?;
        if let Some(content) = req.content {
            spec.content = content;
        }
        if let Some(desc) = req.description {
            spec.description = Some(desc);
        }
        if let Some(tags) = req.tags {
            spec.tags = tags;
        }
        spec.updated_at = chrono::Utc::now();
        // Invalidate quality cache on update.
        self.quality_cache.remove(&id);
        Some(spec.clone())
    }

    pub fn delete_spec(&mut self, id: Uuid) -> bool {
        self.quality_cache.remove(&id);
        self.specs.remove(&id).is_some()
    }

    /// Compute or return cached quality score for a spec.
    pub fn quality_score(&mut self, id: Uuid) -> Option<ApiQualityScore> {
        if let Some(cached) = self.quality_cache.get(&id) {
            return Some(cached.clone());
        }
        let spec = self.specs.get(&id)?.clone();
        let score = compute_quality(&spec);
        self.quality_cache.insert(id, score.clone());
        Some(score)
    }

    /// Generate a plausible mock response for a given path and method.
    pub fn mock_response(&self, spec_id: Uuid, path: &str, method: &str) -> Option<serde_json::Value> {
        // Spec must exist.
        self.specs.get(&spec_id)?;
        Some(generate_mock(path, method))
    }
}

impl Default for ApiDesignerStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Quality scoring ───────────────────────────────────────────────────────────

fn compute_quality(spec: &ApiSpec) -> ApiQualityScore {
    let content = &spec.content;
    let mut issues = Vec::new();

    // --- Documentation dimension ---
    let mut doc = 0.0_f64;
    if spec.description.as_ref().map_or(false, |d| d.len() > 20) {
        doc += 20.0;
    } else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Warning,
            category: "documentation".into(),
            message: "API description is missing or too short".into(),
        });
    }
    if content.contains("contact") { doc += 15.0; }
    else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Info,
            category: "documentation".into(),
            message: "No contact information found in spec".into(),
        });
    }
    if content.contains("license") { doc += 10.0; }
    if !spec.tags.is_empty() { doc += 15.0; }
    if content.contains("example") || content.contains("examples") { doc += 20.0; }
    else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Warning,
            category: "documentation".into(),
            message: "No examples found — consumers will struggle to understand usage".into(),
        });
    }
    if content.contains("servers") || content.contains("host") { doc += 20.0; }

    // --- Security dimension ---
    let mut security = 0.0_f64;
    if content.contains("securitySchemes") || content.contains("securityDefinitions") {
        security += 30.0;
    } else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Error,
            category: "security".into(),
            message: "No security schemes defined".into(),
        });
    }
    if content.contains("security:") { security += 30.0; }
    if content.contains("https") { security += 20.0; }
    else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Warning,
            category: "security".into(),
            message: "No HTTPS server URL found — insecure transport".into(),
        });
    }
    if content.contains("scopes") || content.contains("oauth") { security += 20.0; }

    // --- Design dimension ---
    let mut design = 0.0_f64;
    if content.contains("version") { design += 20.0; }
    // Semantic versioning hint
    let version_parts: Vec<&str> = spec.version.split('.').collect();
    if version_parts.len() == 3 { design += 20.0; }
    else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Info,
            category: "design".into(),
            message: "Version does not follow semantic versioning (X.Y.Z)".into(),
        });
    }
    if content.contains("operationId") { design += 20.0; }
    else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Warning,
            category: "design".into(),
            message: "Operations are missing operationId — hard to reference in SDKs".into(),
        });
    }
    if content.contains("schema") { design += 20.0; }
    if content.contains("400") || content.contains("404") || content.contains("500") {
        design += 20.0;
    } else {
        issues.push(QualityIssue {
            severity: IssueSeverity::Warning,
            category: "design".into(),
            message: "Error responses (4xx/5xx) not documented".into(),
        });
    }

    // --- Completeness dimension ---
    let line_count = content.lines().count();
    let completeness = if line_count > 200 {
        100.0
    } else if line_count > 100 {
        75.0
    } else if line_count > 50 {
        50.0
    } else {
        25.0_f64
    };
    if line_count < 50 {
        issues.push(QualityIssue {
            severity: IssueSeverity::Warning,
            category: "completeness".into(),
            message: format!("Spec is only {} lines — likely incomplete", line_count),
        });
    }

    let overall = (doc + security + design + completeness) / 4.0;

    ApiQualityScore {
        spec_id: spec.id,
        overall: overall.min(100.0),
        documentation: doc.min(100.0),
        security: security.min(100.0),
        design: design.min(100.0),
        completeness,
        issues,
        computed_at: chrono::Utc::now(),
    }
}

fn generate_mock(path: &str, method: &str) -> serde_json::Value {
    let resource = path.split('/').filter(|s| !s.is_empty() && !s.starts_with(':')).last()
        .unwrap_or("resource");
    match method.to_uppercase().as_str() {
        "GET" => serde_json::json!({
            "data": {
                "id": Uuid::new_v4().to_string(),
                "type": resource,
                "attributes": {
                    "name": format!("Mock {}", resource),
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "status": "active"
                }
            },
            "meta": { "mock": true, "path": path }
        }),
        "POST" | "PUT" | "PATCH" => serde_json::json!({
            "id": Uuid::new_v4().to_string(),
            "type": resource,
            "success": true,
            "meta": { "mock": true, "path": path }
        }),
        "DELETE" => serde_json::json!({
            "deleted": true,
            "type": resource,
            "meta": { "mock": true, "path": path }
        }),
        _ => serde_json::json!({ "mock": true, "path": path, "method": method }),
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/v1/gateway/specs", get(list_specs).post(create_spec))
        .route("/api/v1/gateway/specs/{id}", get(get_spec).put(update_spec).delete(delete_spec))
        .route("/api/v1/gateway/specs/{id}/quality", get(quality_score))
        .route("/api/v1/gateway/specs/{id}/mock", post(mock_response))
}

async fn list_specs(State(state): State<Arc<GatewayState>>) -> Json<Vec<ApiSpec>> {
    let store = state.designer.lock().unwrap();
    Json(store.specs.values().cloned().collect())
}

async fn create_spec(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateSpecRequest>,
) -> Json<ApiSpec> {
    let mut store = state.designer.lock().unwrap();
    Json(store.create_spec(req))
}

async fn get_spec(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.designer.lock().unwrap();
    match store.specs.get(&id) {
        Some(s) => Json(serde_json::to_value(s).unwrap()),
        None => Json(serde_json::json!({ "error": "spec not found" })),
    }
}

async fn update_spec(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateSpecRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.designer.lock().unwrap();
    match store.update_spec(id, req) {
        Some(s) => Json(serde_json::to_value(s).unwrap()),
        None => Json(serde_json::json!({ "error": "spec not found" })),
    }
}

async fn delete_spec(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.designer.lock().unwrap();
    Json(serde_json::json!({ "deleted": store.delete_spec(id) }))
}

async fn quality_score(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.designer.lock().unwrap();
    match store.quality_score(id) {
        Some(q) => Json(serde_json::to_value(q).unwrap()),
        None => Json(serde_json::json!({ "error": "spec not found" })),
    }
}

async fn mock_response(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<MockRequest>,
) -> Json<serde_json::Value> {
    let store = state.designer.lock().unwrap();
    match store.mock_response(id, &req.path, &req.method) {
        Some(mock) => Json(mock),
        None => Json(serde_json::json!({ "error": "spec not found" })),
    }
}
