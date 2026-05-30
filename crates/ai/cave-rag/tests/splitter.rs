// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recursive-character and code-aware text splitters.

use cave_rag::document::Document;
use cave_rag::splitter::RecursiveCharacterTextSplitter;

#[test]
fn splits_on_highest_priority_separator() {
    let text = "aaaa\n\nbbbb\n\ncccc";
    let s = RecursiveCharacterTextSplitter::default()
        .with_chunk_size(4)
        .with_chunk_overlap(0);
    let chunks = s.split_text(text);
    assert_eq!(chunks, vec!["aaaa", "bbbb", "cccc"]);
}

#[test]
fn character_level_chunking_with_overlap() {
    let s = RecursiveCharacterTextSplitter::new(vec!["".into()])
        .with_chunk_size(4)
        .with_chunk_overlap(2);
    let chunks = s.split_text("abcdefghij");
    assert_eq!(chunks, vec!["abcd", "cdef", "efgh", "ghij"]);
}

#[test]
fn every_chunk_within_size_when_separators_allow() {
    let text = "one two three four five six seven eight";
    let s = RecursiveCharacterTextSplitter::default()
        .with_chunk_size(10)
        .with_chunk_overlap(0);
    for c in s.split_text(text) {
        assert!(c.len() <= 10, "chunk {c:?} exceeds size 10");
    }
}

#[test]
fn split_documents_preserves_source_and_indexes_chunks() {
    let doc = Document::new("alpha\n\nbeta\n\ngamma").with_source("d.txt");
    let s = RecursiveCharacterTextSplitter::default()
        .with_chunk_size(5)
        .with_chunk_overlap(0);
    let out = s.split_documents(&[doc]);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].metadata.source.as_deref(), Some("d.txt"));
    assert_eq!(out[0].metadata.get("chunk"), Some("0"));
    assert_eq!(out[2].metadata.get("chunk"), Some("2"));
}

#[test]
fn code_splitter_uses_language_boundaries() {
    let code = "fn alpha() {\n    let x = 1;\n}\n\nfn beta() {\n    let y = 2;\n}\n";
    let s = RecursiveCharacterTextSplitter::for_language("rust")
        .with_chunk_size(28)
        .with_chunk_overlap(0);
    let chunks = s.split_text(code);
    assert!(chunks.len() >= 2, "expected the two fns to split apart");
    assert!(chunks.iter().any(|c| c.contains("fn alpha")));
    assert!(chunks.iter().any(|c| c.contains("fn beta")));
    // No chunk should begin partway through the `fn` keyword of beta.
    assert!(!chunks.iter().any(|c| c.starts_with("eta()")));
}

#[test]
fn empty_input_yields_no_chunks() {
    let s = RecursiveCharacterTextSplitter::default();
    assert!(s.split_text("").is_empty());
    assert!(s.split_text("   ").is_empty());
}
