// SPDX-License-Identifier: AGPL-3.0-or-later
//! Foundation types: Document + Metadata + RagError.

use cave_rag::{Document, Metadata, RagError};

#[test]
fn document_carries_content_and_metadata() {
    let doc = Document::new("hello world").with_source("a.txt");
    assert_eq!(doc.content, "hello world");
    assert_eq!(doc.metadata.source.as_deref(), Some("a.txt"));
    assert!(doc.id().len() == 64, "id is a sha256 hex digest");
}

#[test]
fn document_id_is_content_addressed() {
    let a = Document::new("same");
    let b = Document::new("same");
    let c = Document::new("different");
    assert_eq!(a.id(), b.id(), "identical content -> identical id");
    assert_ne!(a.id(), c.id());
}

#[test]
fn metadata_roundtrips_custom_fields() {
    let mut m = Metadata::default();
    m.set("page", "3");
    m.source = Some("doc.pdf".into());
    assert_eq!(m.get("page"), Some("3"));
    assert_eq!(m.get("missing"), None);
}

#[test]
fn error_is_constructible_and_display() {
    let e = RagError::Loader("bad pdf".into());
    assert!(e.to_string().contains("bad pdf"));
}
