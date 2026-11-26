// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Element text and attribute values in the XML readers of `saml` and
//! `wsfed`, all driven through the public entry points.
//!
//! The value a reader returns for an element is the character data of
//! that element: its text, resolved references and CDATA sections, in
//! document order. Comments and processing instructions contribute
//! nothing and do not divide the value. That is also what the canonical
//! form of the document carries between the start and the end tag, so
//! every test here compares the two.
//!
//! * Whitespace written at either end of the content is removed once,
//!   around the whole value. Whitespace written as a character reference
//!   or inside a CDATA section is part of the value.
//! * An element that is read as a text value has no child elements; a
//!   document that has one there is a parse error.
//! * An empty-element tag has no content, so the field stays unset.
//! * A malformed reference, or an attribute that cannot be decoded, is
//!   the reader's parse error.

use cave_auth::saml::SamlError;
use cave_auth::saml::authn_request::AuthnRequest;
use cave_auth::saml::canonicalization::exc_c14n;
use cave_auth::saml::metadata::EntityDescriptor;
use cave_auth::saml::response::Response;
use cave_auth::wsfed::WsFedError;
use cave_auth::wsfed::saml11_assertion::parse_minimal;

// ── Fixtures ─────────────────────────────────────────────────────────────────
//
// Each template has `{Slot}` placeholders, either for the content of an
// element or for the value of an attribute. `fill` puts the input under
// test into one slot and the default into every other one.

const RESPONSE: &str = concat!(
    r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol""#,
    r#" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{ID}" Version="2.0""#,
    r#" IssueInstant="2026-01-01T00:00:00Z" Destination="{Destination}""#,
    r#" InResponseTo="{InResponseTo}">"#,
    "<saml:Issuer>{ResponseIssuer}</saml:Issuer>",
    "<samlp:Status>",
    r#"<samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/>"#,
    "</samlp:Status>",
    r#"<saml:Assertion ID="{AssertionID}" Version="2.0" IssueInstant="2026-01-01T00:00:00Z">"#,
    "<saml:Issuer>{AssertionIssuer}</saml:Issuer>",
    "<saml:Subject><saml:NameID>{NameID}</saml:NameID></saml:Subject>",
    r#"<saml:Conditions NotBefore="2026-01-01T00:00:00Z""#,
    r#" NotOnOrAfter="2026-01-01T00:05:00Z">"#,
    "<saml:AudienceRestriction>",
    "<saml:Audience>{Audience}</saml:Audience>",
    "</saml:AudienceRestriction>",
    "</saml:Conditions>",
    r#"<saml:AuthnStatement AuthnInstant="2026-01-01T00:00:00Z""#,
    r#" SessionIndex="{SessionIndex}"/>"#,
    r#"<saml:AttributeStatement><saml:Attribute Name="{AttributeName}">"#,
    "<saml:AttributeValue>{AttributeValue}</saml:AttributeValue>",
    "</saml:Attribute></saml:AttributeStatement>",
    "</saml:Assertion></samlp:Response>",
);

const RESPONSE_DEFAULTS: &[(&str, &str)] = &[
    ("ID", "_r1"),
    ("Destination", "https://sp.example.com/acs"),
    ("InResponseTo", "_q1"),
    ("ResponseIssuer", "https://idp.example.com"),
    ("AssertionID", "_a1"),
    ("AssertionIssuer", "https://idp.example.com"),
    ("NameID", "alice@example.com"),
    ("Audience", "https://sp.example.com"),
    ("SessionIndex", "_s1"),
    ("AttributeName", "role"),
    ("AttributeValue", "member"),
];

const AUTHN_REQUEST: &str = concat!(
    r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol""#,
    r#" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{ID}" Version="2.0""#,
    r#" IssueInstant="2026-01-01T00:00:00Z" Destination="{Destination}""#,
    r#" AssertionConsumerServiceURL="{AssertionConsumerServiceURL}""#,
    r#" ProtocolBinding="{ProtocolBinding}">"#,
    "<saml:Issuer>{Issuer}</saml:Issuer>",
    r#"<samlp:NameIDPolicy AllowCreate="true""#,
    r#" Format="urn:oasis:names:tc:SAML:2.0:nameid-format:persistent"/>"#,
    "</samlp:AuthnRequest>",
);

const AUTHN_REQUEST_DEFAULTS: &[(&str, &str)] = &[
    ("ID", "_q1"),
    ("Destination", "https://idp.example.com/sso"),
    ("AssertionConsumerServiceURL", "https://sp.example.com/acs"),
    (
        "ProtocolBinding",
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST",
    ),
    ("Issuer", "https://sp.example.com"),
];

const METADATA: &str = concat!(
    r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata""#,
    r#" xmlns:ds="http://www.w3.org/2000/09/xmldsig#" entityID="{entityID}">"#,
    r#"<md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">"#,
    r#"<md:KeyDescriptor use="signing"><ds:KeyInfo><ds:X509Data>"#,
    "<ds:X509Certificate>{X509Certificate}</ds:X509Certificate>",
    "</ds:X509Data></ds:KeyInfo></md:KeyDescriptor>",
    r#"<md:SingleLogoutService Binding="{LogoutBinding}" Location="{LogoutLocation}"/>"#,
    r#"<md:SingleSignOnService Binding="{Binding}" Location="{Location}"/>"#,
    "</md:IDPSSODescriptor></md:EntityDescriptor>",
);

const METADATA_DEFAULTS: &[(&str, &str)] = &[
    ("entityID", "https://idp.example.com"),
    ("X509Certificate", "MIIBexampleCertificate=="),
    (
        "LogoutBinding",
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST",
    ),
    ("LogoutLocation", "https://idp.example.com/slo"),
    (
        "Binding",
        "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect",
    ),
    ("Location", "https://idp.example.com/sso"),
];

const WSFED_ASSERTION: &str = concat!(
    r#"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:1.0:assertion""#,
    r#" MajorVersion="1" MinorVersion="1" AssertionID="{AssertionID}""#,
    r#" Issuer="{Issuer}" IssueInstant="2026-01-01T00:00:00Z">"#,
    r#"<saml:Conditions NotBefore="2026-01-01T00:00:00Z""#,
    r#" NotOnOrAfter="2026-01-01T00:05:00Z">"#,
    "<saml:AudienceRestrictionCondition>",
    "<saml:Audience>{Audience}</saml:Audience>",
    "</saml:AudienceRestrictionCondition>",
    "</saml:Conditions>",
    r#"<saml:AuthenticationStatement AuthenticationInstant="2026-01-01T00:00:00Z""#,
    r#" AuthenticationMethod="{AuthenticationMethod}">"#,
    "<saml:Subject>",
    r#"<saml:NameIdentifier Format="{Format}">{NameIdentifier}</saml:NameIdentifier>"#,
    "</saml:Subject>",
    "</saml:AuthenticationStatement>",
    "<saml:AttributeStatement>",
    r#"<saml:Attribute AttributeName="{AttributeName}""#,
    r#" AttributeNamespace="{AttributeNamespace}">"#,
    "<saml:AttributeValue>{AttributeValue}</saml:AttributeValue>",
    "</saml:Attribute></saml:AttributeStatement>",
    "</saml:Assertion>",
);

const WSFED_ASSERTION_DEFAULTS: &[(&str, &str)] = &[
    ("AssertionID", "_a1"),
    ("Issuer", "https://idp.example.com"),
    ("Audience", "https://rp.example.com"),
    (
        "AuthenticationMethod",
        "urn:oasis:names:tc:SAML:1.0:am:password",
    ),
    (
        "Format",
        "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
    ),
    ("NameIdentifier", "alice@example.com"),
    ("AttributeName", "role"),
    ("AttributeNamespace", "urn:example:claims"),
    ("AttributeValue", "member"),
];

/// `template` with `input` in `slot` and the default in every other slot.
fn fill(template: &str, defaults: &[(&str, &str)], slot: &str, input: &str) -> String {
    assert!(
        defaults.iter().any(|(name, _)| *name == slot),
        "the template has no slot {slot}"
    );
    let mut doc = template.to_string();
    for (name, default) in defaults {
        let replacement = if *name == slot { input } else { default };
        doc = doc.replace(&format!("{{{name}}}"), replacement);
    }
    doc
}

fn response_doc(slot: &str, input: &str) -> String {
    fill(RESPONSE, RESPONSE_DEFAULTS, slot, input)
}

fn authn_request_doc(slot: &str, input: &str) -> String {
    fill(AUTHN_REQUEST, AUTHN_REQUEST_DEFAULTS, slot, input)
}

fn metadata_doc(slot: &str, input: &str) -> String {
    fill(METADATA, METADATA_DEFAULTS, slot, input)
}

fn wsfed_doc(slot: &str, input: &str) -> String {
    fill(WSFED_ASSERTION, WSFED_ASSERTION_DEFAULTS, slot, input)
}

// ── What a reader made of a document ─────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum Read {
    /// The values the reader returned for the field, in order. Empty if
    /// the reader found no value for it.
    Values(Vec<String>),
    /// The reader refused the document with its parse error.
    ParseError,
    /// The reader failed in another way.
    OtherError(String),
}

fn one(value: impl Into<String>) -> Vec<String> {
    vec![value.into()]
}

/// Reads `doc` as a Response. A missing-field error for `missing` means
/// that the reader found no value for the field under test.
fn read_response(doc: &str, missing: &str, pick: fn(Response) -> Vec<String>) -> Read {
    match Response::from_xml(doc.as_bytes()) {
        Ok(response) => Read::Values(pick(response)),
        Err(SamlError::MissingField(field)) if field == missing => Read::Values(Vec::new()),
        Err(SamlError::Parse(_)) => Read::ParseError,
        Err(other) => Read::OtherError(other.to_string()),
    }
}

fn read_authn_request(doc: &str, missing: &str, pick: fn(AuthnRequest) -> Vec<String>) -> Read {
    match AuthnRequest::from_xml(doc.as_bytes()) {
        Ok(request) => Read::Values(pick(request)),
        Err(SamlError::MissingField(field)) if field == missing => Read::Values(Vec::new()),
        Err(SamlError::Parse(_)) => Read::ParseError,
        Err(other) => Read::OtherError(other.to_string()),
    }
}

fn read_metadata(doc: &str, pick: fn(EntityDescriptor) -> Vec<String>) -> Read {
    match EntityDescriptor::from_xml(doc.as_bytes()) {
        Ok(metadata) => Read::Values(pick(metadata)),
        Err(SamlError::Parse(_)) => Read::ParseError,
        Err(other) => Read::OtherError(other.to_string()),
    }
}

fn read_wsfed(
    doc: &str,
    missing: &str,
    pick: fn(cave_auth::wsfed::saml11_assertion::Saml11Assertion) -> Vec<String>,
) -> Read {
    match parse_minimal(doc) {
        Ok(assertion) => Read::Values(pick(assertion)),
        Err(WsFedError::MissingField(field)) if field == missing => Read::Values(Vec::new()),
        Err(WsFedError::Parse(_)) => Read::ParseError,
        Err(other) => Read::OtherError(other.to_string()),
    }
}

/// One value of the documents above, where it sits and how to read it.
struct Field {
    /// Name for failure messages.
    name: &'static str,
    /// The template slot that holds the value.
    slot: &'static str,
    /// Builds the document with an input in `slot`.
    doc: fn(&str, &str) -> String,
    /// Qualified name of the element that carries the value, and which
    /// element of that name it is in document order.
    tag: &'static str,
    occurrence: usize,
    /// For a value carried by an attribute: the attribute name.
    attribute: Option<&'static str>,
    /// Reads the value out of a document.
    read: fn(&str) -> Read,
    /// The reader removes surrounding whitespace from the value it got.
    trims_value: bool,
}

impl Field {
    const fn element(
        name: &'static str,
        slot: &'static str,
        doc: fn(&str, &str) -> String,
        tag: &'static str,
        occurrence: usize,
        read: fn(&str) -> Read,
    ) -> Self {
        Self {
            name,
            slot,
            doc,
            tag,
            occurrence,
            attribute: None,
            read,
            trims_value: false,
        }
    }

    const fn attribute(
        name: &'static str,
        slot: &'static str,
        doc: fn(&str, &str) -> String,
        tag: &'static str,
        attribute: &'static str,
        read: fn(&str) -> Read,
    ) -> Self {
        Self {
            name,
            slot,
            doc,
            tag,
            occurrence: 0,
            attribute: Some(attribute),
            read,
            trims_value: false,
        }
    }

    fn document(&self, input: &str) -> String {
        (self.doc)(self.slot, input)
    }
}

/// Every value the readers take from the content of an element.
const ELEMENT_FIELDS: &[Field] = &[
    Field::element(
        "Response Issuer",
        "ResponseIssuer",
        response_doc,
        "saml:Issuer",
        0,
        |doc| read_response(doc, "Response/Issuer", |r| one(r.issuer)),
    ),
    Field::element(
        "Assertion Issuer",
        "AssertionIssuer",
        response_doc,
        "saml:Issuer",
        1,
        |doc| read_response(doc, "Assertion/Issuer", |r| one(assertion(r).issuer)),
    ),
    Field::element(
        "Assertion NameID",
        "NameID",
        response_doc,
        "saml:NameID",
        0,
        |doc| {
            read_response(doc, "Assertion/NameID", |r| {
                one(assertion(r).subject_name_id)
            })
        },
    ),
    Field::element(
        "Assertion Audience",
        "Audience",
        response_doc,
        "saml:Audience",
        0,
        |doc| read_response(doc, "", |r| assertion(r).audiences),
    ),
    Field::element(
        "Assertion AttributeValue",
        "AttributeValue",
        response_doc,
        "saml:AttributeValue",
        0,
        |doc| {
            read_response(doc, "", |r| {
                let mut attributes = assertion(r).attributes;
                let values = attributes.remove("role").unwrap_or_default();
                assert!(attributes.is_empty(), "{attributes:?}");
                values
            })
        },
    ),
    Field::element(
        "AuthnRequest Issuer",
        "Issuer",
        authn_request_doc,
        "saml:Issuer",
        0,
        |doc| read_authn_request(doc, "Issuer", |r| one(r.issuer)),
    ),
    Field {
        trims_value: true,
        ..Field::element(
            "metadata X509Certificate",
            "X509Certificate",
            metadata_doc,
            "ds:X509Certificate",
            0,
            |doc| read_metadata(doc, |m| m.signing_cert_b64.into_iter().collect()),
        )
    },
    Field::element(
        "WS-Fed Audience",
        "Audience",
        wsfed_doc,
        "saml:Audience",
        0,
        |doc| read_wsfed(doc, "", |a| a.audience.into_iter().collect()),
    ),
    Field::element(
        "WS-Fed NameIdentifier",
        "NameIdentifier",
        wsfed_doc,
        "saml:NameIdentifier",
        0,
        |doc| read_wsfed(doc, "NameIdentifier", |a| one(a.name_identifier)),
    ),
    Field::element(
        "WS-Fed AttributeValue",
        "AttributeValue",
        wsfed_doc,
        "saml:AttributeValue",
        0,
        |doc| {
            read_wsfed(doc, "", |a| {
                let mut attributes = a.attributes;
                let values = attributes.remove("role").unwrap_or_default();
                assert!(attributes.is_empty(), "{attributes:?}");
                values
            })
        },
    ),
];

/// Every string value the readers take from an attribute.
const ATTRIBUTE_FIELDS: &[Field] = &[
    Field::attribute(
        "Response ID",
        "ID",
        response_doc,
        "samlp:Response",
        "ID",
        |doc| read_response(doc, "", |r| one(r.id)),
    ),
    Field::attribute(
        "Response Destination",
        "Destination",
        response_doc,
        "samlp:Response",
        "Destination",
        |doc| read_response(doc, "", |r| one(r.destination)),
    ),
    Field::attribute(
        "Response InResponseTo",
        "InResponseTo",
        response_doc,
        "samlp:Response",
        "InResponseTo",
        |doc| read_response(doc, "", |r| r.in_response_to.into_iter().collect()),
    ),
    Field::attribute(
        "Assertion ID",
        "AssertionID",
        response_doc,
        "saml:Assertion",
        "ID",
        |doc| read_response(doc, "", |r| one(assertion(r).id)),
    ),
    Field::attribute(
        "AuthnStatement SessionIndex",
        "SessionIndex",
        response_doc,
        "saml:AuthnStatement",
        "SessionIndex",
        |doc| {
            read_response(doc, "", |r| {
                assertion(r).session_index.into_iter().collect()
            })
        },
    ),
    Field::attribute(
        "Attribute Name",
        "AttributeName",
        response_doc,
        "saml:Attribute",
        "Name",
        |doc| read_response(doc, "", |r| assertion(r).attributes.into_keys().collect()),
    ),
    Field::attribute(
        "AuthnRequest ID",
        "ID",
        authn_request_doc,
        "samlp:AuthnRequest",
        "ID",
        |doc| read_authn_request(doc, "", |r| one(r.id)),
    ),
    Field::attribute(
        "AuthnRequest Destination",
        "Destination",
        authn_request_doc,
        "samlp:AuthnRequest",
        "Destination",
        |doc| read_authn_request(doc, "", |r| one(r.destination)),
    ),
    Field::attribute(
        "AuthnRequest AssertionConsumerServiceURL",
        "AssertionConsumerServiceURL",
        authn_request_doc,
        "samlp:AuthnRequest",
        "AssertionConsumerServiceURL",
        |doc| read_authn_request(doc, "", |r| r.acs_url.into_iter().collect()),
    ),
    Field::attribute(
        "AuthnRequest ProtocolBinding",
        "ProtocolBinding",
        authn_request_doc,
        "samlp:AuthnRequest",
        "ProtocolBinding",
        |doc| read_authn_request(doc, "", |r| r.protocol_binding.into_iter().collect()),
    ),
    Field::attribute(
        "metadata entityID",
        "entityID",
        metadata_doc,
        "md:EntityDescriptor",
        "entityID",
        |doc| read_metadata(doc, |m| one(m.entity_id)),
    ),
    Field::attribute(
        "metadata SingleSignOnService Binding",
        "Binding",
        metadata_doc,
        "md:SingleSignOnService",
        "Binding",
        |doc| {
            read_metadata(doc, |m| {
                m.endpoints.into_iter().map(|e| e.binding).collect()
            })
        },
    ),
    Field::attribute(
        "metadata SingleSignOnService Location",
        "Location",
        metadata_doc,
        "md:SingleSignOnService",
        "Location",
        |doc| {
            read_metadata(doc, |m| {
                m.endpoints.into_iter().map(|e| e.location).collect()
            })
        },
    ),
    Field::attribute(
        "metadata SingleLogoutService Binding",
        "LogoutBinding",
        metadata_doc,
        "md:SingleLogoutService",
        "Binding",
        |doc| {
            read_metadata(doc, |m| {
                m.slo_endpoint.into_iter().map(|e| e.binding).collect()
            })
        },
    ),
    Field::attribute(
        "metadata SingleLogoutService Location",
        "LogoutLocation",
        metadata_doc,
        "md:SingleLogoutService",
        "Location",
        |doc| {
            read_metadata(doc, |m| {
                m.slo_endpoint.into_iter().map(|e| e.location).collect()
            })
        },
    ),
    Field::attribute(
        "WS-Fed AssertionID",
        "AssertionID",
        wsfed_doc,
        "saml:Assertion",
        "AssertionID",
        |doc| read_wsfed(doc, "", |a| one(a.assertion_id)),
    ),
    Field::attribute(
        "WS-Fed Issuer",
        "Issuer",
        wsfed_doc,
        "saml:Assertion",
        "Issuer",
        |doc| read_wsfed(doc, "", |a| one(a.issuer)),
    ),
    Field::attribute(
        "WS-Fed AuthenticationMethod",
        "AuthenticationMethod",
        wsfed_doc,
        "saml:AuthenticationStatement",
        "AuthenticationMethod",
        |doc| read_wsfed(doc, "", |a| one(a.auth_method)),
    ),
    Field::attribute(
        "WS-Fed NameIdentifier Format",
        "Format",
        wsfed_doc,
        "saml:NameIdentifier",
        "Format",
        |doc| read_wsfed(doc, "", |a| one(a.name_identifier_format)),
    ),
    Field::attribute(
        "WS-Fed AttributeName",
        "AttributeName",
        wsfed_doc,
        "saml:Attribute",
        "AttributeName",
        |doc| read_wsfed(doc, "", |a| a.attributes.into_keys().collect()),
    ),
    Field::attribute(
        "WS-Fed AttributeNamespace",
        "AttributeNamespace",
        wsfed_doc,
        "saml:Attribute",
        "AttributeNamespace",
        |doc| read_wsfed(doc, "", |a| one(a.attribute_namespace)),
    ),
];

fn assertion(response: Response) -> cave_auth::saml::response::Assertion {
    response.assertion.expect("the fixture has an assertion")
}

// ── The canonical form, read independently of the readers ────────────────────

/// The canonical form of `doc`, or `None` if `exc_c14n` refuses it.
fn canonical(doc: &str) -> Option<String> {
    exc_c14n(doc.as_bytes())
        .ok()
        .map(|bytes| String::from_utf8(bytes).expect("the canonical form is UTF-8"))
}

/// Byte offsets of one element in `xml`: where its start tag begins,
/// where its content begins and ends, and where the element ends.
struct Span {
    start: usize,
    content_start: usize,
    content_end: usize,
    end: usize,
}

/// The `occurrence`-th element named `tag` in `xml`. The element has to
/// be written with a start and an end tag and must not contain another
/// element of the same name.
fn element_span(xml: &str, tag: &str, occurrence: usize) -> Span {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut seen = 0;
    let mut from = 0;
    loop {
        let start = from
            + xml[from..]
                .find(&open)
                .unwrap_or_else(|| panic!("no element {tag} #{occurrence} in {xml}"));
        let after_name = start + open.len();
        from = after_name;
        // `<saml:Attribute` is also how `<saml:AttributeValue` begins.
        if !xml[after_name..].starts_with([' ', '>']) {
            continue;
        }
        if seen < occurrence {
            seen += 1;
            continue;
        }
        let content_start = after_name + xml[after_name..].find('>').expect("start tag") + 1;
        let content_end = content_start + xml[content_start..].find(&close).expect("end tag");
        return Span {
            start,
            content_start,
            content_end,
            end: content_end + close.len(),
        };
    }
}

/// Resolves the predefined entities and character references of
/// canonical text or of a canonical attribute value. Processing
/// instructions are skipped; any other markup is not character data.
fn resolve(escaped: &str) -> String {
    let mut out = String::new();
    let mut rest = escaped;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("<?") {
            let end = rest.find("?>").expect("processing instruction end");
            rest = &rest[end + 2..];
        } else if c == '<' {
            panic!("markup inside a text value: {escaped}");
        } else if c == '&' {
            let end = rest.find(';').expect("reference end");
            out.push(match &rest[1..end] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                reference => {
                    let code = match reference.strip_prefix("#x") {
                        Some(hex) => u32::from_str_radix(hex, 16),
                        None => reference
                            .strip_prefix('#')
                            .expect("a character reference")
                            .parse(),
                    };
                    char::from_u32(code.expect("a number")).expect("a character")
                }
            });
            rest = &rest[end + 1..];
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// What the canonical form of `doc` carries for `field`: the character
/// data of its element, or the value of its attribute. `None` if the
/// document has no canonical form.
fn canonical_value(field: &Field, doc: &str) -> Option<String> {
    let canonical = canonical(doc)?;
    let span = element_span(&canonical, field.tag, field.occurrence);
    Some(match field.attribute {
        None => resolve(&canonical[span.content_start..span.content_end]),
        Some(attribute) => {
            let start_tag = &canonical[span.start..span.content_start];
            let open = format!(" {attribute}=\"");
            let value_start = start_tag.find(&open).expect("the attribute") + open.len();
            let value_end = value_start + start_tag[value_start..].find('"').expect("quote");
            resolve(&start_tag[value_start..value_end])
        }
    })
}

/// `text` without XML whitespace at either end.
fn trim_xml_whitespace(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\r', '\n'])
}

/// Fails once, listing every mismatch.
fn assert_no_mismatches(mismatches: Vec<String>) {
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

// ── Element content ──────────────────────────────────────────────────────────

/// An element content and the value a reader returns for it; `None` if
/// the content leaves the field without a value.
type Content = (&'static str, Option<&'static str>);

/// Puts every content into every field of [`ELEMENT_FIELDS`] and
/// compares the value the reader returns with the expected one and with
/// the character data of that element in the canonical form.
fn assert_element_text(contents: &[Content]) {
    let mut mismatches = Vec::new();
    for field in ELEMENT_FIELDS {
        for (content, value) in contents {
            let doc = field.document(content);
            let value = value.map(|v| if field.trims_value { v.trim() } else { v });
            let expected = Read::Values(value.into_iter().map(String::from).collect());
            let read = (field.read)(&doc);
            let carried = canonical_value(field, &doc).expect("a canonical form");
            if read != expected {
                mismatches.push(format!(
                    "{} {content:?}: expected {expected:?}, got {read:?}; \
                     the canonical form carries {carried:?}",
                    field.name
                ));
                continue;
            }
            // The canonical form keeps the surrounding whitespace that
            // the reader removes; apart from that the two are equal.
            let returned = value.unwrap_or("");
            if trim_xml_whitespace(&carried) != trim_xml_whitespace(returned)
                || !carried.contains(returned)
            {
                mismatches.push(format!(
                    "{} {content:?}: the reader returned {returned:?}, \
                     the canonical form carries {carried:?}",
                    field.name
                ));
            }
        }
    }
    assert_no_mismatches(mismatches);
}

#[test]
fn text_and_references_are_the_character_data_of_the_canonical_form() {
    assert_element_text(&[
        ("alice@example.com", Some("alice@example.com")),
        ("a &amp; b &lt;c&gt;", Some("a & b <c>")),
        ("&#x41;&#66;C", Some("ABC")),
        ("\n  alice@example.com\t", Some("alice@example.com")),
        (" &#32;x&#32; ", Some(" x ")),
        ("", None),
        (" \n\t ", None),
    ]);
}

#[test]
fn a_comment_does_not_divide_element_text() {
    assert_element_text(&[
        ("alice<!-- c -->@example.com", Some("alice@example.com")),
        ("<!-- c -->alice@example.com", Some("alice@example.com")),
        ("alice@example.com<!-- c -->", Some("alice@example.com")),
        (
            "al<!-- 1 -->ice@<!-- 2 -->example<!-- 3 -->.com",
            Some("alice@example.com"),
        ),
        ("a&amp;<!-- c -->&lt;b", Some("a&<b")),
        ("a<!-- <x/> &amp; &bogus; ]]> -->b", Some("ab")),
        ("ali<!-- é -->cé", Some("alicé")),
        ("<!-- c -->", None),
        (" <!-- c --> ", None),
    ]);
}

#[test]
fn a_cdata_section_is_part_of_element_text() {
    assert_element_text(&[
        ("<![CDATA[alice@example.com]]>", Some("alice@example.com")),
        ("alice<![CDATA[@]]>example.com", Some("alice@example.com")),
        (
            "<![CDATA[alice]]><![CDATA[@example.com]]>",
            Some("alice@example.com"),
        ),
        ("alice@<![CDATA[example.com]]>", Some("alice@example.com")),
        ("<![CDATA[alice]]>@example.com", Some("alice@example.com")),
        // The content of a CDATA section is taken as written.
        (
            "<![CDATA[a<b>&amp;<!-- c -->]]>",
            Some("a<b>&amp;<!-- c -->"),
        ),
        ("&amp;<![CDATA[&]]>&#38;", Some("&&&")),
        ("a<![CDATA[]]>b", Some("ab")),
        ("<![CDATA[]]>", None),
    ]);
}

#[test]
fn a_processing_instruction_does_not_divide_element_text() {
    assert_element_text(&[
        ("alice<?pi data?>@example.com", Some("alice@example.com")),
        ("<?pi data?>alice@example.com", Some("alice@example.com")),
        ("al<?a?>ice@<?b c?>example.com", Some("alice@example.com")),
        ("<?pi data?>", None),
    ]);
}

#[test]
fn whitespace_is_removed_once_around_the_whole_value() {
    assert_element_text(&[
        ("  alice <!-- c --> bob  ", Some("alice  bob")),
        (" <!-- c --> alice <?pi?> ", Some("alice")),
        ("alice <![CDATA[and]]> bob", Some("alice and bob")),
        // Whitespace written as a reference or in a CDATA section is
        // part of the value.
        (" <![CDATA[ x ]]> ", Some(" x ")),
        (" <!-- c -->&#32;x<!-- c --> ", Some(" x")),
        (" <![CDATA[]]> x <![CDATA[]]> ", Some("x")),
        // Everything together.
        (
            " a<!-- c -->&#32;<![CDATA[b ]]><?pi x?> &amp; c ",
            Some("a b  & c"),
        ),
    ]);
}

#[test]
fn comments_and_processing_instructions_between_elements_change_nothing() {
    fn with_markup_between_elements(doc: &str) -> String {
        doc.replace("><", "><!-- c --><?pi data?><")
    }

    let doc = response_doc("NameID", "alice@example.com");
    assert_eq!(
        Response::from_xml(with_markup_between_elements(&doc).as_bytes()).expect("response"),
        Response::from_xml(doc.as_bytes()).expect("response"),
    );

    let doc = authn_request_doc("Issuer", "https://sp.example.com");
    assert_eq!(
        AuthnRequest::from_xml(with_markup_between_elements(&doc).as_bytes()).expect("request"),
        AuthnRequest::from_xml(doc.as_bytes()).expect("request"),
    );

    let doc = metadata_doc("entityID", "https://idp.example.com");
    assert_eq!(
        EntityDescriptor::from_xml(with_markup_between_elements(&doc).as_bytes())
            .expect("metadata"),
        EntityDescriptor::from_xml(doc.as_bytes()).expect("metadata"),
    );

    let doc = wsfed_doc("NameIdentifier", "alice@example.com");
    assert_eq!(
        format!(
            "{:?}",
            parse_minimal(&with_markup_between_elements(&doc)).expect("assertion")
        ),
        format!("{:?}", parse_minimal(&doc).expect("assertion")),
    );
}

#[test]
fn a_malformed_reference_in_element_text_is_a_parse_error() {
    let contents = [
        "alice &bogus; bob",
        "alice & bob",
        "alice &amp bob",
        "&#xZZ;",
        "&bogus;",
        "alice<!-- c -->&bogus;",
        "&bogus;<![CDATA[alice]]>",
        "<![CDATA[alice]]>&bogus;",
        // A reference does not continue behind a comment, a processing
        // instruction or a CDATA section.
        "&am<!-- c -->p;",
        "&am<?pi?>p;",
        "&<![CDATA[amp;]]>",
    ];
    let mut mismatches = Vec::new();
    for field in ELEMENT_FIELDS {
        for content in contents {
            let doc = field.document(content);
            let read = (field.read)(&doc);
            if read != Read::ParseError {
                mismatches.push(format!(
                    "{} {content:?}: expected a parse error, got {read:?}",
                    field.name
                ));
            }
            if let Some(canonical) = canonical(&doc) {
                mismatches.push(format!(
                    "{} {content:?}: exc_c14n accepted it: {canonical}",
                    field.name
                ));
            }
        }
    }
    assert_no_mismatches(mismatches);
}

#[test]
fn a_child_element_in_a_text_value_is_a_parse_error() {
    let contents = [
        "alice<x/>bob",
        "alice<x>and</x>bob",
        "<x>alice</x>",
        "<x/>",
        "alice<!-- c --><x/>",
        "<![CDATA[alice]]><x/>",
        // Elements the readers know are no exception.
        "<saml:NameID>bob@example.org</saml:NameID>",
        "<saml:Issuer>https://idp.example.org</saml:Issuer>",
        "<saml:Audience>https://sp.example.org</saml:Audience>",
        "<saml:AttributeValue>owner</saml:AttributeValue>",
        "<saml:NameIdentifier>bob@example.org</saml:NameIdentifier>",
        "alice<saml:NameID/>",
    ];
    let mut mismatches = Vec::new();
    for field in ELEMENT_FIELDS {
        for content in contents {
            let read = (field.read)(&field.document(content));
            if read != Read::ParseError {
                mismatches.push(format!(
                    "{} {content:?}: expected a parse error, got {read:?}",
                    field.name
                ));
            }
        }
    }
    assert_no_mismatches(mismatches);
}

#[test]
fn an_empty_element_tag_has_no_value() {
    // The element is written as an empty-element tag and followed by
    // another element with text. The text belongs to that other element.
    let mut mismatches = Vec::new();
    for field in ELEMENT_FIELDS {
        let plain = field.document("alice@example.com");
        let span = element_span(&plain, field.tag, field.occurrence);
        let doc = format!(
            "{}<{}/><x>bob@example.org</x>{}",
            &plain[..span.start],
            field.tag,
            &plain[span.end..]
        );
        let read = (field.read)(&doc);
        if read != Read::Values(Vec::new()) {
            mismatches.push(format!("{}: expected no value, got {read:?}", field.name));
        }
        let carried = canonical_value(field, &doc).expect("a canonical form");
        if !carried.is_empty() {
            mismatches.push(format!(
                "{}: the canonical form carries {carried:?}",
                field.name
            ));
        }
    }
    assert_no_mismatches(mismatches);
}

// ── Attributes ───────────────────────────────────────────────────────────────

/// Puts every raw attribute value into every field of
/// [`ATTRIBUTE_FIELDS`] and compares the value the reader returns with
/// the expected one and with the value of that attribute in the
/// canonical form. `None` stands for a value that cannot be decoded: a
/// parse error, and no canonical form.
fn assert_attribute_values(values: &[(&str, Option<&str>)]) {
    let mut mismatches = Vec::new();
    for field in ATTRIBUTE_FIELDS {
        for (raw, value) in values {
            let doc = field.document(raw);
            let read = (field.read)(&doc);
            let carried = canonical_value(field, &doc);
            let expected = match value {
                Some(value) => Read::Values(one(*value)),
                None => Read::ParseError,
            };
            if read != expected || carried.as_deref() != *value {
                mismatches.push(format!(
                    "{} {raw:?}: expected {expected:?}, got {read:?}; \
                     the canonical form carries {carried:?}",
                    field.name
                ));
            }
        }
    }
    assert_no_mismatches(mismatches);
}

#[test]
fn attribute_values_are_the_values_of_the_canonical_form() {
    assert_attribute_values(&[
        (
            "https://example.com/a?b=1",
            Some("https://example.com/a?b=1"),
        ),
        ("a &amp; b &lt;c&gt;", Some("a & b <c>")),
        ("&#x41;&#66;C", Some("ABC")),
        (" &#32;x&#32; ", Some("  x  ")),
        ("x\ty\nz&#10;w", Some("x\ty\nz\nw")),
        ("&quot;a&quot; &apos;b&apos;", Some("\"a\" 'b'")),
    ]);
}

#[test]
fn a_malformed_reference_in_an_attribute_value_is_a_parse_error() {
    assert_attribute_values(&[
        ("alice &bogus; bob", None),
        ("alice & bob", None),
        ("alice &amp bob", None),
        ("&#xZZ;", None),
    ]);
}

/// `doc` with `attributes` added to the first start tag of `tag`.
fn with_attributes(doc: &str, tag: &str, attributes: &str) -> String {
    let at = after_tag_name(doc, tag);
    format!("{} {attributes}{}", &doc[..at], &doc[at..])
}

/// Offset just behind the name in the first start tag or empty-element
/// tag of `tag`.
fn after_tag_name(doc: &str, tag: &str) -> usize {
    let open = format!("<{tag}");
    let mut from = 0;
    loop {
        let start = from
            + doc[from..]
                .find(&open)
                .unwrap_or_else(|| panic!("no element {tag} in {doc}"));
        let after_name = start + open.len();
        if doc[after_name..].starts_with([' ', '>', '/']) {
            return after_name;
        }
        from = after_name;
    }
}

#[test]
fn an_attribute_that_cannot_be_decoded_is_a_parse_error() {
    // Every element whose attributes a reader looks at.
    type Reader = (
        &'static str,
        String,
        fn(&str) -> Read,
        &'static [&'static str],
    );
    let readers: [Reader; 4] = [
        (
            "Response",
            response_doc("NameID", "alice@example.com"),
            |doc| read_response(doc, "", |_| Vec::new()),
            &[
                "samlp:Response",
                "samlp:StatusCode",
                "saml:Assertion",
                "saml:NameID",
                "saml:Conditions",
                "saml:AuthnStatement",
                "saml:Attribute",
            ],
        ),
        (
            "AuthnRequest",
            authn_request_doc("Issuer", "https://sp.example.com"),
            |doc| read_authn_request(doc, "", |_| Vec::new()),
            &["samlp:AuthnRequest", "samlp:NameIDPolicy"],
        ),
        (
            "metadata",
            metadata_doc("entityID", "https://idp.example.com"),
            |doc| read_metadata(doc, |_| Vec::new()),
            &[
                "md:EntityDescriptor",
                "md:KeyDescriptor",
                "md:SingleLogoutService",
                "md:SingleSignOnService",
            ],
        ),
        (
            "WS-Fed",
            wsfed_doc("NameIdentifier", "alice@example.com"),
            |doc| read_wsfed(doc, "", |_| Vec::new()),
            &[
                "saml:Assertion",
                "saml:Conditions",
                "saml:AuthenticationStatement",
                "saml:NameIdentifier",
                "saml:Attribute",
            ],
        ),
    ];
    // An attribute written twice, one without a value and one whose
    // value is not quoted.
    let undecodable = [r#"extra="1" extra="2""#, "extra", "extra=1"];

    let mut mismatches = Vec::new();
    for (name, plain, read, tags) in &readers {
        assert_eq!(read(plain), Read::Values(Vec::new()), "{name}: the fixture");
        for tag in *tags {
            let decodable = with_attributes(plain, tag, r#"extra="1""#);
            assert_eq!(read(&decodable), Read::Values(Vec::new()), "{decodable}");
            for attributes in undecodable {
                let doc = with_attributes(plain, tag, attributes);
                let outcome = read(&doc);
                if outcome != Read::ParseError {
                    mismatches.push(format!(
                        "{name} <{tag} {attributes}>: expected a parse error, got {outcome:?}"
                    ));
                }
                if canonical(&doc).is_some() {
                    mismatches.push(format!("{name} <{tag} {attributes}>: exc_c14n accepted it"));
                }
            }
        }
    }
    assert_no_mismatches(mismatches);
}
