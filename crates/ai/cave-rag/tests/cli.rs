// SPDX-License-Identifier: AGPL-3.0-or-later
//! CLI surface: `cave-rag <command>` over the offline library.

use cave_rag::cli::run_args;

#[test]
fn graph_extract_emits_entities_relationships_communities() {
    let out = run_args([
        "cave-rag",
        "graph",
        "extract",
        "--text",
        "Alice works with Bob at Acme. Carol leads Globex with Dave.",
    ])
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entities = v["entities"].as_array().unwrap();
    assert!(entities.iter().any(|e| e == "Alice"));
    assert!(entities.iter().any(|e| e == "Globex"));
    // Two disconnected sentences → two communities.
    assert_eq!(v["communities"].as_array().unwrap().len(), 2);
    assert!(v["relationship_count"].as_u64().unwrap() >= 3);
}

#[test]
fn graph_search_returns_local_neighborhood() {
    let out = run_args([
        "cave-rag",
        "graph",
        "search",
        "--text",
        "Alice works with Bob at Acme. Carol leads Globex with Dave.",
        "--query",
        "tell me about Alice",
        "--hops",
        "1",
    ])
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let ctx = v["context"].as_array().unwrap();
    assert!(ctx.iter().any(|e| e == "Bob"));
    assert!(ctx.iter().any(|e| e == "Acme"));
    assert!(!ctx.iter().any(|e| e == "Dave"));
}

#[test]
fn split_chunks_text_by_size() {
    let out = run_args([
        "cave-rag",
        "split",
        "--text",
        "The first sentence. The second sentence. The third one here.",
        "--size",
        "25",
    ])
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["chunk_count"].as_u64().unwrap() >= 2);
    assert!(v["chunks"].as_array().unwrap().len() >= 2);
}

#[test]
fn unknown_command_is_an_error_not_a_panic() {
    let err = run_args(["cave-rag", "definitely-not-a-command"]);
    assert!(err.is_err());
}
