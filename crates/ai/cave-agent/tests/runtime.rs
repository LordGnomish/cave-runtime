// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Default runtime assembly + the pure cores behind the HTTP/portal surface.

use cave_agent::router::{admin_page_html, invoke_json, plan_json, tool_catalog_json};
use cave_agent::{default_runtime, UPSTREAM_VERSION};
use serde_json::json;

#[test]
fn default_runtime_ships_builtin_tools_and_knobs() {
    let rt = default_runtime();
    let names = rt.tools.names();
    for must in ["calc", "str_upper", "str_len"] {
        assert!(names.iter().any(|n| n == must), "missing builtin {must}");
    }
    assert!(rt.knobs.max_tokens >= 256);
    assert_eq!(UPSTREAM_VERSION, "v2026.5.20");
}

#[test]
fn tool_catalog_json_lists_every_tool() {
    let rt = default_runtime();
    let cat = tool_catalog_json(&rt);
    let arr = cat["tools"].as_array().unwrap();
    assert_eq!(arr.len(), rt.tools.len());
    assert!(arr.iter().any(|t| t["name"] == "calc"));
}

#[test]
fn invoke_json_runs_a_builtin() {
    let rt = default_runtime();
    let out = invoke_json(&rt, &json!({"tool": "calc", "args": {"op":"mul","a":6,"b":7}}));
    assert_eq!(out["ok"], true);
    assert_eq!(out["result"]["result"], 42.0);
}

#[test]
fn invoke_json_reports_unknown_tool_as_error() {
    let rt = default_runtime();
    let out = invoke_json(&rt, &json!({"tool": "ghost", "args": {}}));
    assert_eq!(out["ok"], false);
    assert!(out["error"].as_str().unwrap().contains("unknown tool"));
}

#[test]
fn plan_json_decomposes_goal_into_steps() {
    let out = plan_json("fetch then clean then summarise");
    assert_eq!(out["goal"], "fetch then clean then summarise");
    assert_eq!(out["steps"].as_array().unwrap().len(), 3);
    assert_eq!(out["steps"][0]["description"], "fetch");
}

#[test]
fn admin_page_is_self_contained_html() {
    let rt = default_runtime();
    let html = admin_page_html(&rt);
    assert!(html.contains("<html"));
    assert!(html.contains("/admin/agent"));
    assert!(html.contains("calc"));
    assert!(html.contains("OpenJarvis"));
}
