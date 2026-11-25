// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Regression fixtures for the CycloneDX XML reader, driven through
//! `sbom::cyclonedx::parse_xml`.
//!
//! * A document type declaration is refused: a BOM does not need one.
//! * Component text with references keeps the value the reader
//!   produces today, so that a dependency update cannot change it
//!   unnoticed.

use cave_sbom::sbom::cyclonedx::{CycloneDxError, parse_xml};

/// Document type declarations: plain, lower-case keyword, with an
/// internal subset declaring an entity, with `>` inside a quoted
/// literal, with `>` inside a comment, and with `<` as system literal.
const DOCTYPES: [&str; 6] = [
    "<!DOCTYPE bom>",
    "<!doctype bom>",
    r#"<!DOCTYPE bom [<!ENTITY a "b">]>"#,
    r#"<!DOCTYPE bom [<!ENTITY a ">">]>"#,
    "<!DOCTYPE bom [<!-- > -->]>",
    r#"<!DOCTYPE bom SYSTEM "<">"#,
];

/// A BOM with one component. `before_root` is raw markup placed between
/// the XML declaration and the root element, `name` the raw content of
/// the component's `name` element.
fn bom_doc(before_root: &str, name: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            "{before_root}",
            r#"<bom xmlns="http://cyclonedx.org/schema/bom/1.5" version="1.5">"#,
            r#"<components><component type="library" bom-ref="pkg:npm/example@1.0.0">"#,
            "<name>{name}</name><version>1.0.0</version>",
            "</component></components></bom>",
        ),
        before_root = before_root,
        name = name,
    )
}

/// The name of the single component in `doc`, or `None` if the reader
/// refused the document as invalid XML.
fn component_name(doc: &str) -> Option<String> {
    match parse_xml(doc.as_bytes()) {
        Ok(bom) => {
            assert_eq!(bom.components.len(), 1, "{:?}", bom.components);
            bom.components.into_iter().next().map(|c| c.name)
        }
        Err(CycloneDxError::Xml(_)) => None,
        Err(other) => panic!("unexpected error for {doc:?}: {other:?}"),
    }
}

/// Whether `e` is the reader refusing a document type declaration.
fn is_refusal(e: &CycloneDxError) -> bool {
    matches!(e, CycloneDxError::Xml(message) if message.contains("document type declaration"))
}

#[test]
fn cyclonedx_xml_refuses_a_document_type_declaration() {
    assert_eq!(
        component_name(&bom_doc("", "example")).as_deref(),
        Some("example")
    );

    let not_refused: Vec<String> = DOCTYPES
        .iter()
        .filter_map(
            |doctype| match parse_xml(bom_doc(doctype, "example").as_bytes()) {
                Err(e) if is_refusal(&e) => None,
                Err(e) => Some(format!("{doctype} -> a different error: {e:?}")),
                Ok(_) => Some(format!("{doctype} -> accepted")),
            },
        )
        .collect();
    assert!(not_refused.is_empty(), "{not_refused:#?}");
}

#[test]
fn cyclonedx_xml_keeps_component_text_with_references() {
    let cases = [
        ("a &amp; b", Some("a & b")),
        (" &#32;x&#32; ", Some(" x ")),
        ("&lt;&gt;", Some("<>")),
        ("&#x41;&#66;", Some("AB")),
        ("a & b", None),
        ("&foo;", None),
    ];
    let mismatches: Vec<String> = cases
        .iter()
        .filter_map(|(raw, expected)| {
            let actual = component_name(&bom_doc("", raw));
            (actual.as_deref() != *expected)
                .then(|| format!("{raw:?}: expected {expected:?}, got {actual:?}"))
        })
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");

    // An entity declared in an internal subset is not resolved either.
    let doctype = r#"<!DOCTYPE bom [<!ENTITY foo "bar">]>"#;
    assert_eq!(component_name(&bom_doc(doctype, "&foo;")), None);
}
