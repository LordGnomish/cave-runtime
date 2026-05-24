// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OpenAPI 3.0 metadata.
//!
//! Mirrors `org.dependencytrack.resources.OpenApiResource` — exposes the
//! `/api/swagger.json` describing the REST v1 surface.

use serde_json::{Value, json};

pub fn openapi_spec() -> Value {
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "cave-dependency-track REST API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "SBOM / SCA platform — Dependency-Track v4.14.2 deep-port.",
            "license": {"name": "AGPL-3.0-or-later"},
            "contact": {"name": "Cave Runtime", "url": "https://github.com/cave-runtime/cave-runtime"}
        },
        "servers": [{"url": "/api/v1"}],
        "tags": [
            {"name": "project"}, {"name": "component"}, {"name": "vulnerability"},
            {"name": "bom"}, {"name": "vex"}, {"name": "policy"}, {"name": "analysis"},
            {"name": "notification"}, {"name": "integration"}, {"name": "search"},
            {"name": "license"}, {"name": "cpe"}, {"name": "purl"}, {"name": "repository"},
        ],
        "paths": {
            "/project":       {"get": op("project", "list"), "post": op("project", "create")},
            "/project/{uuid}":{"get": op("project", "get"),  "delete": op("project", "delete")},
            "/component":     {"get": op("component", "list")},
            "/vulnerability": {"get": op("vulnerability", "list")},
            "/bom/cyclonedx": {"post": op("bom", "upload-cdx")},
            "/bom/spdx":      {"post": op("bom", "upload-spdx")},
            "/vex":           {"get":  op("vex", "export")},
            "/bov":           {"get":  op("bov", "export")},
            "/policy":        {"get":  op("policy", "list"), "post": op("policy", "create")},
            "/analysis":      {"post": op("analysis", "upsert")},
            "/notification":  {"get":  op("notification", "list"), "post": op("notification", "create")},
            "/search":        {"get":  op("search", "query")},
            "/license":       {"get":  op("license", "catalog")},
            "/repository":    {"get":  op("repository", "list")},
            "/graphql":       {"post": op("graphql", "query")},
            "/swagger.json":  {"get":  op("openapi", "spec")},
            "/healthz":       {"get":  op("health", "live")},
        }
    })
}

fn op(tag: &str, op_id: &str) -> Value {
    json!({
        "tags": [tag],
        "operationId": format!("{}_{}", tag, op_id),
        "responses": {
            "200": {"description": "OK"},
            "400": {"description": "Bad Request"},
            "404": {"description": "Not Found"},
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_has_required_top_level() {
        let s = openapi_spec();
        for key in ["openapi", "info", "servers", "tags", "paths"] {
            assert!(s.get(key).is_some(), "missing {}", key);
        }
    }

    #[test]
    fn paths_cover_core_resources() {
        let s = openapi_spec();
        for path in ["/project", "/component", "/vulnerability", "/bom/cyclonedx", "/policy", "/vex", "/bov", "/search", "/graphql"] {
            assert!(s["paths"].get(path).is_some(), "missing path {}", path);
        }
    }

    #[test]
    fn project_has_crud_methods() {
        let s = openapi_spec();
        assert!(s["paths"]["/project"]["get"].is_object());
        assert!(s["paths"]["/project"]["post"].is_object());
        assert!(s["paths"]["/project/{uuid}"]["delete"].is_object());
    }

    #[test]
    fn agpl_license_listed() {
        let s = openapi_spec();
        assert_eq!(s["info"]["license"]["name"], "AGPL-3.0-or-later");
    }

    #[test]
    fn operation_ids_unique() {
        let s = openapi_spec();
        let mut ids = std::collections::HashSet::new();
        let paths = s["paths"].as_object().unwrap();
        for (_, ops) in paths {
            for (_, op) in ops.as_object().unwrap() {
                let id = op["operationId"].as_str().unwrap().to_string();
                assert!(ids.insert(id.clone()), "duplicate operationId: {}", id);
            }
        }
    }

    #[test]
    fn server_url_is_api_v1() {
        let s = openapi_spec();
        assert_eq!(s["servers"][0]["url"], "/api/v1");
    }
}
