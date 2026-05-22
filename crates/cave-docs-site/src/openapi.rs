// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiSpec {
    pub openapi: String,
    pub info: ApiInfo,
    pub paths: HashMap<String, PathItem>,
    pub components: Option<Components>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiInfo {
    pub title: String,
    pub version: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathItem {
    pub get: Option<Operation>,
    pub post: Option<Operation>,
    pub put: Option<Operation>,
    pub delete: Option<Operation>,
    pub patch: Option<Operation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub operation_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub parameters: Option<Vec<Parameter>>,
    pub request_body: Option<RequestBody>,
    pub responses: HashMap<String, Response>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "in")]
    pub location: String,
    pub required: Option<bool>,
    pub description: Option<String>,
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBody {
    pub description: Option<String>,
    pub required: Option<bool>,
    pub content: HashMap<String, MediaType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaType {
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub description: String,
    pub content: Option<HashMap<String, MediaType>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Components {
    pub schemas: Option<HashMap<String, serde_json::Value>>,
}

pub struct ApiRefGenerator;

impl ApiRefGenerator {
    /// Parse an OpenAPI JSON string
    pub fn parse(json: &str) -> DocsResult<OpenApiSpec> {
        serde_json::from_str(json).map_err(|e| DocsError::OpenApiError(e.to_string()))
    }

    /// Generate Markdown API reference from spec
    pub fn to_markdown(spec: &OpenApiSpec) -> String {
        let mut md = format!("# {}\n\n", spec.info.title);
        if let Some(desc) = &spec.info.description {
            md.push_str(&format!("{}\n\n", desc));
        }
        md.push_str(&format!("**Version:** {}\n\n", spec.info.version));
        md.push_str("## Endpoints\n\n");

        let mut paths: Vec<(&String, &PathItem)> = spec.paths.iter().collect();
        paths.sort_by_key(|(p, _)| *p);

        for (path, item) in paths {
            let ops: [(&str, &Option<Operation>); 5] = [
                ("GET", &item.get),
                ("POST", &item.post),
                ("PUT", &item.put),
                ("DELETE", &item.delete),
                ("PATCH", &item.patch),
            ];
            for (method, op) in &ops {
                if let Some(op) = op {
                    md.push_str(&format!("### {} {}\n\n", method, path));
                    if let Some(summary) = &op.summary {
                        md.push_str(&format!("**{}**\n\n", summary));
                    }
                    if let Some(desc) = &op.description {
                        md.push_str(&format!("{}\n\n", desc));
                    }
                    if let Some(params) = &op.parameters {
                        if !params.is_empty() {
                            md.push_str("**Parameters:**\n\n");
                            for p in params {
                                let req = if p.required.unwrap_or(false) {
                                    " *(required)*"
                                } else {
                                    ""
                                };
                                md.push_str(&format!(
                                    "- `{}` ({}){}: {}\n",
                                    p.name,
                                    p.location,
                                    req,
                                    p.description.as_deref().unwrap_or("")
                                ));
                            }
                            md.push('\n');
                        }
                    }
                    md.push_str("**Responses:**\n\n");
                    let mut resp_codes: Vec<(&String, &Response)> = op.responses.iter().collect();
                    resp_codes.sort_by_key(|(c, _)| *c);
                    for (code, resp) in resp_codes {
                        md.push_str(&format!("- `{}`: {}\n", code, resp.description));
                    }
                    md.push('\n');
                }
            }
        }
        md
    }

    /// Generate HTML API reference
    pub fn to_html(spec: &OpenApiSpec) -> String {
        let md = Self::to_markdown(spec);
        let renderer = crate::renderer::MarkdownRenderer::new();
        renderer.render(&md)
    }

    /// Create a docs Page from an OpenAPI spec
    pub fn to_page(spec: &OpenApiSpec, space_id: &str, version: &str) -> crate::types::Page {
        let markdown = Self::to_markdown(spec);
        let slug = format!(
            "api-reference-{}",
            spec.info.version.replace('.', "-")
        );
        crate::types::Page {
            id: uuid::Uuid::new_v4().to_string(),
            space_id: space_id.to_string(),
            slug,
            title: format!("{} API Reference", spec.info.title),
            markdown_content: markdown.clone(),
            html_content: Some(Self::to_html(spec)),
            group_id: None,
            parent_id: None,
            order: 999,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: version.to_string(),
            metadata: std::collections::HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OPENAPI: &str = r#"{
        "openapi": "3.0.0",
        "info": {
            "title": "Pet Store",
            "version": "1.0.0",
            "description": "A sample API"
        },
        "paths": {
            "/pets": {
                "get": {
                    "summary": "List all pets",
                    "parameters": [
                        {
                            "name": "limit",
                            "in": "query",
                            "required": false,
                            "description": "Max results to return"
                        }
                    ],
                    "responses": {
                        "200": {"description": "A list of pets"},
                        "400": {"description": "Bad request"}
                    }
                },
                "post": {
                    "summary": "Create a pet",
                    "responses": {
                        "201": {"description": "Pet created"}
                    }
                }
            }
        }
    }"#;

    #[test]
    fn openapi_parse_and_render() {
        let spec = ApiRefGenerator::parse(SAMPLE_OPENAPI).unwrap();
        assert_eq!(spec.info.title, "Pet Store");
        assert_eq!(spec.info.version, "1.0.0");

        let md = ApiRefGenerator::to_markdown(&spec);
        assert!(md.contains("Pet Store"), "title missing");
        assert!(md.contains("/pets"), "path missing");
        assert!(md.contains("List all pets"), "summary missing");
        assert!(md.contains("limit"), "parameter missing");
        assert!(md.contains("200"), "response code missing");
    }
}
