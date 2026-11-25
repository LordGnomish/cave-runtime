// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Regression fixtures for the XML readers in `saml` and `wsfed`,
//! all driven through the public entry points.
//!
//! * A document type declaration is refused by every reader: SAML and
//!   WS-Federation messages do not carry one.
//! * Text and attribute values with references and whitespace keep the
//!   values and canonical bytes the readers produce today, so that a
//!   dependency update cannot change them unnoticed.

use std::fmt::Debug;

use cave_auth::saml::SamlError;
use cave_auth::saml::authn_request::AuthnRequest;
use cave_auth::saml::canonicalization::exc_c14n;
use cave_auth::saml::metadata::EntityDescriptor;
use cave_auth::saml::response::Response;
use cave_auth::wsfed::WsFedError;
use cave_auth::wsfed::saml11_assertion::parse_minimal;

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// Document type declarations: plain, lower-case keyword, with an
/// internal subset declaring an entity, with `>` inside a quoted
/// literal, with `>` inside a comment, and with `<` as system literal.
const DOCTYPES: [&str; 6] = [
    "<!DOCTYPE r>",
    "<!doctype r>",
    r#"<!DOCTYPE r [<!ENTITY a "b">]>"#,
    r#"<!DOCTYPE r [<!ENTITY a ">">]>"#,
    "<!DOCTYPE r [<!-- > -->]>",
    r#"<!DOCTYPE r SYSTEM "<">"#,
];

/// An internal subset that declares the entity `foo`.
const DOCTYPE_DECLARING_FOO: &str = r#"<!DOCTYPE r [<!ENTITY foo "bar">]>"#;

/// Part of the message every reader refuses a declaration with.
const REFUSAL: &str = "document type declaration";

/// A Response with one Assertion. `name_id` is the raw content of the
/// NameID element, `after_name_id` raw markup placed right behind it
/// and `attribute_name` the raw value of the Attribute `Name`.
fn response_doc(name_id: &str, after_name_id: &str, attribute_name: &str) -> String {
    format!(
        concat!(
            r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol""#,
            r#" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_r1" Version="2.0""#,
            r#" IssueInstant="2026-01-01T00:00:00Z" Destination="https://sp.example.com/acs">"#,
            "<saml:Issuer>https://idp.example.com</saml:Issuer>",
            "<samlp:Status>",
            r#"<samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/>"#,
            "</samlp:Status>",
            r#"<saml:Assertion ID="_a1" Version="2.0" IssueInstant="2026-01-01T00:00:00Z">"#,
            "<saml:Issuer>https://idp.example.com</saml:Issuer>",
            "<saml:Subject><saml:NameID>{name_id}</saml:NameID>{after_name_id}</saml:Subject>",
            r#"<saml:Conditions NotBefore="2026-01-01T00:00:00Z""#,
            r#" NotOnOrAfter="2026-01-01T00:05:00Z">"#,
            "<saml:AudienceRestriction>",
            "<saml:Audience>https://sp.example.com</saml:Audience>",
            "</saml:AudienceRestriction>",
            "</saml:Conditions>",
            r#"<saml:AttributeStatement><saml:Attribute Name="{attribute_name}">"#,
            "<saml:AttributeValue>member</saml:AttributeValue>",
            "</saml:Attribute></saml:AttributeStatement>",
            "</saml:Assertion></samlp:Response>",
        ),
        name_id = name_id,
        after_name_id = after_name_id,
        attribute_name = attribute_name,
    )
}

/// The Response fixture with plain values.
fn plain_response_doc() -> String {
    response_doc("alice@example.com", "", "role")
}

const AUTHN_REQUEST: &str = concat!(
    r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol""#,
    r#" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="_q1" Version="2.0""#,
    r#" IssueInstant="2026-01-01T00:00:00Z" Destination="https://idp.example.com/sso">"#,
    "<saml:Issuer>https://sp.example.com</saml:Issuer>",
    "</samlp:AuthnRequest>",
);

const METADATA: &str = concat!(
    r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata""#,
    r#" entityID="https://idp.example.com">"#,
    r#"<md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">"#,
    r#"<md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect""#,
    r#" Location="https://idp.example.com/sso"/>"#,
    "</md:IDPSSODescriptor></md:EntityDescriptor>",
);

const WSFED_ASSERTION: &str = concat!(
    r#"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:1.0:assertion""#,
    r#" MajorVersion="1" MinorVersion="1" AssertionID="_a1""#,
    r#" Issuer="https://idp.example.com" IssueInstant="2026-01-01T00:00:00Z">"#,
    r#"<saml:Conditions NotBefore="2026-01-01T00:00:00Z""#,
    r#" NotOnOrAfter="2026-01-01T00:05:00Z">"#,
    "<saml:AudienceRestrictionCondition>",
    "<saml:Audience>https://rp.example.com</saml:Audience>",
    "</saml:AudienceRestrictionCondition>",
    "</saml:Conditions>",
    r#"<saml:AuthenticationStatement AuthenticationInstant="2026-01-01T00:00:00Z""#,
    r#" AuthenticationMethod="urn:oasis:names:tc:SAML:1.0:am:password">"#,
    "<saml:Subject>",
    "<saml:NameIdentifier>alice@example.com</saml:NameIdentifier>",
    "</saml:Subject>",
    "</saml:AuthenticationStatement>",
    "</saml:Assertion>",
);

// ── Helpers ──────────────────────────────────────────────────────────────────

/// What a reader made of one input.
#[derive(Debug, PartialEq)]
enum Read {
    /// The value it returned.
    Value(String),
    /// It refused the input with its parse error.
    ParseError,
}

fn value(s: &str) -> Read {
    Read::Value(s.to_string())
}

fn read_or_parse_error(input: &str, result: Result<String, SamlError>) -> Read {
    match result {
        Ok(value) => Read::Value(value),
        Err(SamlError::Parse(_)) => Read::ParseError,
        Err(other) => panic!("unexpected error for {input:?}: {other:?}"),
    }
}

/// The canonical form of `doc`.
fn canonical(doc: &str) -> Read {
    let result = exc_c14n(doc.as_bytes())
        .map(|bytes| String::from_utf8(bytes).expect("canonical form is UTF-8"));
    read_or_parse_error(doc, result)
}

/// The NameID of the Assertion in `doc`.
fn name_id(doc: &str) -> Read {
    let result = Response::from_xml(doc.as_bytes()).map(|response| {
        response
            .assertion
            .expect("the fixture has an assertion")
            .subject_name_id
    });
    read_or_parse_error(doc, result)
}

/// The name of the single attribute of the Assertion in `doc`.
fn attribute_name(doc: &str) -> Read {
    let result = Response::from_xml(doc.as_bytes()).map(|response| {
        let attributes = response
            .assertion
            .expect("the fixture has an assertion")
            .attributes;
        assert_eq!(attributes.len(), 1, "{attributes:?}");
        attributes
            .into_keys()
            .next()
            .expect("the fixture has an attribute")
    });
    read_or_parse_error(doc, result)
}

/// Runs `read` over every case and fails once, listing every case whose
/// outcome differs from the expected one.
fn assert_cases(cases: &[(&str, Read)], read: impl Fn(&str) -> Read) {
    let mismatches: Vec<String> = cases
        .iter()
        .filter_map(|(input, expected)| {
            let actual = read(input);
            (actual != *expected)
                .then(|| format!("{input:?}: expected {expected:?}, got {actual:?}"))
        })
        .collect();
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

/// Builds one document per entry of [`DOCTYPES`] with `place` and fails
/// unless `read` refuses every one of them, returning no value, with an
/// error that `is_refusal` recognises.
fn assert_doctypes_refused<T, E: Debug>(
    place: impl Fn(&str) -> String,
    read: impl Fn(&str) -> Result<T, E>,
    is_refusal: impl Fn(&E) -> bool,
) {
    let not_refused: Vec<String> = DOCTYPES
        .iter()
        .filter_map(|doctype| match read(&place(doctype)) {
            Err(e) if is_refusal(&e) => None,
            Err(e) => Some(format!("{doctype} -> a different error: {e:?}")),
            Ok(_) => Some(format!("{doctype} -> accepted")),
        })
        .collect();
    assert!(not_refused.is_empty(), "{not_refused:#?}");
}

fn saml_refusal(e: &SamlError) -> bool {
    matches!(e, SamlError::Parse(message) if message.contains(REFUSAL))
}

fn wsfed_refusal(e: &WsFedError) -> bool {
    matches!(e, WsFedError::Parse(message) if message.contains(REFUSAL))
}

// ── The fixtures themselves are valid ────────────────────────────────────────

#[test]
fn fixtures_are_accepted_without_a_document_type_declaration() {
    assert_eq!(canonical("<r/>"), value("<r></r>"));
    assert_eq!(
        canonical("<r><a>x</a><b>y</b></r>"),
        value("<r><a>x</a><b>y</b></r>")
    );

    let response = Response::from_xml(plain_response_doc().as_bytes()).expect("response");
    assert_eq!(response.destination, "https://sp.example.com/acs");
    let assertion = response.assertion.expect("assertion");
    assert_eq!(assertion.subject_name_id, "alice@example.com");
    assert_eq!(
        assertion.attributes.get("role"),
        Some(&vec!["member".to_string()])
    );

    let request = AuthnRequest::from_xml(AUTHN_REQUEST.as_bytes()).expect("authn request");
    assert_eq!(request.issuer, "https://sp.example.com");

    let metadata = EntityDescriptor::from_xml(METADATA.as_bytes()).expect("metadata");
    assert_eq!(metadata.entity_id, "https://idp.example.com");

    let assertion = parse_minimal(WSFED_ASSERTION).expect("ws-fed assertion");
    assert_eq!(assertion.name_identifier, "alice@example.com");
}

// ── A document type declaration is refused ───────────────────────────────────

#[test]
fn exc_c14n_refuses_a_document_type_declaration_before_the_root() {
    assert_doctypes_refused(
        |doctype| format!("{doctype}<r/>"),
        |doc| exc_c14n(doc.as_bytes()),
        saml_refusal,
    );
}

#[test]
fn exc_c14n_refuses_a_document_type_declaration_inside_the_document() {
    assert_doctypes_refused(
        |doctype| format!("<r><a>x</a>{doctype}<b>y</b></r>"),
        |doc| exc_c14n(doc.as_bytes()),
        saml_refusal,
    );
}

#[test]
fn response_refuses_a_document_type_declaration_before_the_root() {
    assert_doctypes_refused(
        |doctype| format!("{doctype}{}", plain_response_doc()),
        |doc| Response::from_xml(doc.as_bytes()),
        saml_refusal,
    );
}

#[test]
fn response_refuses_a_document_type_declaration_inside_the_document() {
    assert_doctypes_refused(
        |doctype| response_doc("alice@example.com", doctype, "role"),
        |doc| Response::from_xml(doc.as_bytes()),
        saml_refusal,
    );
}

#[test]
fn authn_request_refuses_a_document_type_declaration() {
    assert_doctypes_refused(
        |doctype| format!("{doctype}{AUTHN_REQUEST}"),
        |doc| AuthnRequest::from_xml(doc.as_bytes()),
        saml_refusal,
    );
}

#[test]
fn metadata_refuses_a_document_type_declaration() {
    assert_doctypes_refused(
        |doctype| format!("{doctype}{METADATA}"),
        |doc| EntityDescriptor::from_xml(doc.as_bytes()),
        saml_refusal,
    );
}

#[test]
fn wsfed_assertion_refuses_a_document_type_declaration() {
    assert_doctypes_refused(
        |doctype| format!("{doctype}{WSFED_ASSERTION}"),
        parse_minimal,
        wsfed_refusal,
    );
}

// ── Values and canonical bytes stay as they are ──────────────────────────────

#[test]
fn exc_c14n_keeps_text_with_references() {
    assert_cases(
        &[
            ("<r>a &amp; b</r>", value("<r>a &amp; b</r>")),
            ("<r> &#32;x&#32; </r>", value("<r>  x  </r>")),
            ("<r>&lt;&gt;</r>", value("<r>&lt;&gt;</r>")),
            ("<r>&#x41;&#66;</r>", value("<r>AB</r>")),
            ("<r>a & b</r>", Read::ParseError),
            ("<r>&foo;</r>", Read::ParseError),
        ],
        canonical,
    );
}

#[test]
fn exc_c14n_keeps_attribute_values_with_references_and_whitespace() {
    assert_cases(
        &[
            (r#"<r a="a &amp; b"/>"#, value(r#"<r a="a &amp; b"></r>"#)),
            (r#"<r a=" &#32;x&#32; "/>"#, value(r#"<r a="  x  "></r>"#)),
            (r#"<r a="&lt;&gt;"/>"#, value(r#"<r a="&lt;&gt;"></r>"#)),
            (r#"<r a="&#x41;&#66;"/>"#, value(r#"<r a="AB"></r>"#)),
            (
                "<r a=\"x\ty\nz&#10;w\"/>",
                value("<r a=\"x\ty\nz\nw\"></r>"),
            ),
            (r#"<r a="a & b"/>"#, Read::ParseError),
            (r#"<r a="&foo;"/>"#, Read::ParseError),
        ],
        canonical,
    );
}

#[test]
fn response_keeps_name_id_text_with_references() {
    assert_cases(
        &[
            ("a &amp; b", value("a & b")),
            (" &#32;x&#32; ", value(" x ")),
            ("&lt;&gt;", value("<>")),
            ("&#x41;&#66;", value("AB")),
            ("a & b", Read::ParseError),
            ("&foo;", Read::ParseError),
        ],
        |text| name_id(&response_doc(text, "", "role")),
    );
}

#[test]
fn response_keeps_attribute_values_with_references_and_whitespace() {
    assert_cases(
        &[
            ("a &amp; b", value("a & b")),
            (" &#32;x&#32; ", value("  x  ")),
            ("&lt;&gt;", value("<>")),
            ("&#x41;&#66;", value("AB")),
            ("x\ty\nz&#10;w", value("x\ty\nz\nw")),
            ("a & b", Read::ParseError),
            ("&foo;", Read::ParseError),
        ],
        |raw| attribute_name(&response_doc("alice@example.com", "", raw)),
    );
}

#[test]
fn an_entity_declared_in_an_internal_subset_stays_an_error() {
    assert_eq!(
        canonical(&format!("{DOCTYPE_DECLARING_FOO}<r>&foo;</r>")),
        Read::ParseError
    );
    assert_eq!(
        canonical(&format!("{DOCTYPE_DECLARING_FOO}<r a=\"&foo;\"/>")),
        Read::ParseError
    );
    assert_eq!(
        name_id(&format!(
            "{DOCTYPE_DECLARING_FOO}{}",
            response_doc("&foo;", "", "role")
        )),
        Read::ParseError
    );
    assert_eq!(
        attribute_name(&format!(
            "{DOCTYPE_DECLARING_FOO}{}",
            response_doc("alice@example.com", "", "&foo;")
        )),
        Read::ParseError
    );
}
