// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 9 (RED→GREEN): semantic tool search.

use cave_tools::search::ToolSearchIndex;
use cave_tools::tool::{FnTool, ToolRegistry, ToolResult, ToolSpec};
use serde_json::json;

fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "file_read".into(),
            description: "Read the contents of a text file from disk".into(),
            input_schema: json!({}),
            toolset: "fs".into(),
        },
        ToolSpec {
            name: "web_search".into(),
            description: "Search the web and return ranked result links".into(),
            input_schema: json!({}),
            toolset: "net".into(),
        },
        ToolSpec {
            name: "calendar_add".into(),
            description: "Add an event to the calendar schedule".into(),
            input_schema: json!({}),
            toolset: "calendar".into(),
        },
    ]
}

#[test]
fn ranks_most_relevant_tool_first() {
    let idx = ToolSearchIndex::build(&specs());
    let hits = idx.search("read a file from disk", 3);
    assert!(!hits.is_empty());
    assert_eq!(hits[0].name, "file_read");
}

#[test]
fn different_query_picks_different_tool() {
    let idx = ToolSearchIndex::build(&specs());
    let hits = idx.search("search the web", 3);
    assert_eq!(hits[0].name, "web_search");
}

#[test]
fn matches_on_toolset_facet() {
    let idx = ToolSearchIndex::build(&specs());
    let hits = idx.search("calendar", 3);
    assert_eq!(hits[0].name, "calendar_add");
}

#[test]
fn top_k_limits_results() {
    let idx = ToolSearchIndex::build(&specs());
    let hits = idx.search("a the to from", 1);
    assert!(hits.len() <= 1);
}

#[test]
fn irrelevant_query_returns_no_hits() {
    let idx = ToolSearchIndex::build(&specs());
    let hits = idx.search("quantum chromodynamics", 3);
    assert!(hits.is_empty());
}

#[test]
fn scores_descend() {
    let idx = ToolSearchIndex::build(&specs());
    let hits = idx.search("file disk read web search", 3);
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "scores not descending: {hits:?}");
    }
}

#[test]
fn builds_from_registry() {
    let mut reg = ToolRegistry::new();
    reg.register(FnTool::new(
        "summarize",
        "Summarize a long document into bullet points",
        json!({"type": "object"}),
        |_| Ok(ToolResult::text("ok")),
    ));
    let idx = ToolSearchIndex::from_registry(&reg);
    let hits = idx.search("summarize document", 5);
    assert_eq!(hits[0].name, "summarize");
}
