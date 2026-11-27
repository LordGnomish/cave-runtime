// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Text-node, element-text and attribute helpers for the quick-xml
//! event parsers in `saml` and `wsfed`.
//!
//! Since quick-xml 0.38 the reader no longer hands out a text node as
//! one `Event::Text`: every reference (`&amp;`, `&#x20;`, …) arrives as
//! its own `Event::GeneralRef` between the `Event::Text` pieces around
//! it, text trimming is applied to each piece, and an `&` without a
//! closing `;` is a reader error. The parsers here are written against
//! whole text nodes, so [`TextRun`] puts the pieces of one node back
//! together, trims the node as a whole and leaves a dangling `&` to
//! [`unescape_text`], which rejects it exactly like an unknown entity.
//!
//! [`ElementText`] reads the value of an element whose content is one
//! text value (`<saml:Issuer>`, `<saml:NameID>`, …). An element like
//! that can hold several text nodes and CDATA sections with comments
//! and processing instructions between them; the value is all of its
//! character data, as the canonical form of the document carries it.
//!
//! [`attr_value`] is the attribute-side counterpart: it resolves the
//! references of an attribute value and changes nothing else.
//! (`Attribute::normalized_value` additionally turns tab, CR and LF
//! into spaces.) [`tag_attributes`] iterates the attributes of a start
//! tag and reports one that cannot be read instead of skipping it.

use std::borrow::Cow;
use std::io::BufRead;

use quick_xml::encoding::EncodingError;
use quick_xml::escape::unescape;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::{Config, Reader};

/// Error text for a document type declaration. SAML and WS-Federation
/// messages do not carry one, so the parsers refuse it instead of
/// skipping it.
pub(crate) const DOCTYPE_NOT_ALLOWED: &str = "a document type declaration is not allowed";

/// Error text for an element inside an element that is read as one
/// text value.
const CHILD_ELEMENT_NOT_ALLOWED: &str = "a child element is not allowed in a text value";

/// The raw bytes of one text node: everything between two pieces of
/// markup, references included.
pub(crate) struct TextRun {
    raw: Vec<u8>,
    trim: bool,
}

impl TextRun {
    /// Sets the reader `config` up for whole-node text handling and
    /// returns an empty run. With `trim`, XML whitespace is removed
    /// from both ends of every node and whitespace-only nodes are
    /// skipped — what `Config::trim_text(true)` did for single-event
    /// text nodes.
    pub(crate) fn for_reader(config: &mut Config, trim: bool) -> Self {
        // Trimming happens in `take`, on the reassembled node.
        config.trim_text(false);
        // Hand a dangling `&` over as text; `unescape_text` reports it.
        config.allow_dangling_amp = true;
        Self {
            raw: Vec::new(),
            trim,
        }
    }

    /// Appends `event` to the run if it is part of a text node.
    /// Returns whether the event was consumed.
    pub(crate) fn absorb(&mut self, event: &Event<'_>) -> bool {
        match event {
            Event::Text(text) => {
                self.raw.extend_from_slice(text);
                true
            }
            Event::GeneralRef(reference) => {
                self.raw.push(b'&');
                self.raw.extend_from_slice(reference);
                self.raw.push(b';');
                true
            }
            _ => false,
        }
    }

    /// Ends the current text node and returns its raw bytes, or `None`
    /// if there is no node (nothing absorbed, or only whitespace while
    /// trimming).
    pub(crate) fn take(&mut self) -> Option<Vec<u8>> {
        let raw = std::mem::take(&mut self.raw);
        let node: &[u8] = if self.trim {
            trim_xml_whitespace(&raw)
        } else {
            &raw
        };
        if node.is_empty() {
            None
        } else {
            Some(node.to_vec())
        }
    }
}

/// Reads the value of elements whose content is one text value.
///
/// The value of such an element is its character data: its text, the
/// characters its references stand for and the content of its CDATA
/// sections, in document order. Comments and processing instructions
/// contribute nothing and do not divide the value. This is what the
/// canonical form of the document has between the start and the end
/// tag of the element.
///
/// XML whitespace written before the first and after the last character
/// of the value is removed, once, around the whole value. Whitespace
/// written as a character reference or inside a CDATA section is part
/// of the value.
pub(crate) struct ElementText {
    run: TextRun,
    buf: Vec<u8>,
}

impl ElementText {
    /// Sets the reader `config` up like [`TextRun::for_reader`] and
    /// returns a reader for element values.
    pub(crate) fn for_reader(config: &mut Config) -> Self {
        Self {
            run: TextRun::for_reader(config, false),
            buf: Vec::new(),
        }
    }

    /// Reads the content of the element whose start tag `reader` has
    /// just returned, up to and including its end tag, and returns its
    /// value; `None` if the element has no character data besides
    /// surrounding whitespace.
    ///
    /// The element must not have child elements. The error is the text
    /// for the parse error of the calling parser: a reader error, a
    /// malformed reference, a child element or a document type
    /// declaration. A document that ends inside the element ends the
    /// value; the next event of `reader` is `Event::Eof` again.
    pub(crate) fn read<R: BufRead>(
        &mut self,
        reader: &mut Reader<R>,
    ) -> Result<Option<String>, String> {
        let mut data = CharData::default();
        loop {
            self.buf.clear();
            let event = reader.read_event_into(&mut self.buf);
            if event.as_ref().is_ok_and(|ev| self.run.absorb(ev)) {
                continue;
            }
            // A malformed reference is reported before the event that
            // ends its text node is looked at.
            if let Some(raw) = self.run.take() {
                data.push_text(&raw).map_err(|err| err.to_string())?;
            }
            match event {
                Err(err) => return Err(format!("xml: {err}")),
                Ok(Event::End(_) | Event::Eof) => return Ok(data.into_value()),
                Ok(Event::CData(content)) => {
                    data.push_cdata(&content).map_err(|err| err.to_string())?;
                }
                Ok(Event::Comment(_) | Event::PI(_) | Event::Decl(_)) => {
                    // No character data.
                }
                Ok(Event::Start(_) | Event::Empty(_)) => {
                    return Err(format!("xml: {CHILD_ELEMENT_NOT_ALLOWED}"));
                }
                Ok(Event::DocType(_)) => {
                    return Err(format!("xml: {DOCTYPE_NOT_ALLOWED}"));
                }
                Ok(Event::Text(_) | Event::GeneralRef(_)) => {
                    // Absorbed into `self.run` above.
                }
            }
        }
    }
}

/// The character data of one element while it is being read.
#[derive(Default)]
struct CharData {
    /// The value so far, up to its last character that is not
    /// whitespace written as such.
    value: String,
    /// Whitespace written behind `value`. It belongs to the value if
    /// more character data follows and is dropped if the element ends.
    pending: Vec<u8>,
}

impl CharData {
    /// Appends a text node, given as its raw bytes.
    fn push_text(&mut self, raw: &[u8]) -> Result<(), quick_xml::Error> {
        // Whitespace in front of the first character of the value is
        // not part of it.
        let raw = if self.value.is_empty() {
            &raw[leading_xml_whitespace(raw)..]
        } else {
            raw
        };
        let (text, whitespace) = raw.split_at(raw.len() - trailing_xml_whitespace(raw));
        if !text.is_empty() {
            let text = unescape_text(text)?;
            self.keep_pending();
            self.value.push_str(&text);
        }
        self.pending.extend_from_slice(whitespace);
        Ok(())
    }

    /// Appends the content of a CDATA section.
    fn push_cdata(&mut self, content: &[u8]) -> Result<(), quick_xml::Error> {
        if !content.is_empty() {
            let content = decode(content)?;
            self.keep_pending();
            self.value.push_str(content);
        }
        Ok(())
    }

    /// Makes the pending whitespace part of the value.
    fn keep_pending(&mut self) {
        self.value.extend(self.pending.drain(..).map(char::from));
    }

    /// The value, without the pending whitespace.
    fn into_value(self) -> Option<String> {
        (!self.value.is_empty()).then_some(self.value)
    }
}

fn is_xml_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn trim_xml_whitespace(raw: &[u8]) -> &[u8] {
    let start = raw
        .iter()
        .position(|&b| !is_xml_whitespace(b))
        .unwrap_or(raw.len());
    let end = raw
        .iter()
        .rposition(|&b| !is_xml_whitespace(b))
        .map_or(start, |last| last + 1);
    &raw[start..end]
}

/// Number of XML whitespace bytes at the start of `raw`.
fn leading_xml_whitespace(raw: &[u8]) -> usize {
    raw.iter().take_while(|&&b| is_xml_whitespace(b)).count()
}

/// Number of XML whitespace bytes at the end of `raw`.
fn trailing_xml_whitespace(raw: &[u8]) -> usize {
    raw.iter()
        .rev()
        .take_while(|&&b| is_xml_whitespace(b))
        .count()
}

/// Decodes raw bytes of the document as UTF-8.
fn decode(raw: &[u8]) -> Result<&str, quick_xml::Error> {
    Ok(std::str::from_utf8(raw).map_err(EncodingError::from)?)
}

/// Decodes the raw bytes of a text node as UTF-8 and resolves the
/// predefined entities and character references in it.
pub(crate) fn unescape_text(raw: &[u8]) -> Result<Cow<'_, str>, quick_xml::Error> {
    Ok(unescape(decode(raw)?)?)
}

/// Decodes an attribute value as UTF-8 and resolves the predefined
/// entities and character references in it. Whitespace is kept as
/// written.
pub(crate) fn attr_value<'v>(attr: &'v Attribute<'_>) -> Result<Cow<'v, str>, quick_xml::Error> {
    unescape_text(&attr.value)
}

/// The attributes of a start tag. An attribute that cannot be read
/// (written twice, without a value, with an unquoted value, …) is the
/// text for the parse error of the calling parser.
pub(crate) fn tag_attributes<'a>(
    start: &'a BytesStart<'_>,
) -> impl Iterator<Item = Result<Attribute<'a>, String>> {
    start
        .attributes()
        .map(|attribute| attribute.map_err(|err| format!("xml: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collects the text nodes of `xml` the way the parsers do.
    fn text_nodes(xml: &str, trim: bool) -> Vec<Result<String, String>> {
        let mut reader = Reader::from_reader(xml.as_bytes());
        let mut run = TextRun::for_reader(reader.config_mut(), trim);
        let mut nodes = Vec::new();
        let mut buf = Vec::new();
        loop {
            let event = reader.read_event_into(&mut buf);
            if event.as_ref().is_ok_and(|ev| run.absorb(ev)) {
                buf.clear();
                continue;
            }
            if let Some(raw) = run.take() {
                nodes.push(
                    unescape_text(&raw)
                        .map(Cow::into_owned)
                        .map_err(|e| e.to_string()),
                );
            }
            match event {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("reader error: {e}"),
            }
            buf.clear();
        }
        nodes
    }

    fn ok(nodes: &[&str]) -> Vec<Result<String, String>> {
        nodes.iter().map(|s| Ok((*s).to_string())).collect()
    }

    #[test]
    fn references_stay_inside_their_text_node() {
        assert_eq!(
            text_nodes("<a>Tom &amp; Jerry &lt;tj@example.com&gt;</a>", true),
            ok(&["Tom & Jerry <tj@example.com>"])
        );
        assert_eq!(text_nodes("<a>&#65;&#x42;C</a>", true), ok(&["ABC"]));
    }

    #[test]
    fn trimming_applies_to_the_node_not_to_its_pieces() {
        assert_eq!(text_nodes("<a>\n  x &amp; y\t</a>", true), ok(&["x & y"]));
        // A whitespace character reference is content, not padding.
        assert_eq!(text_nodes("<a> &#32;x&#32; </a>", true), ok(&[" x "]));
        assert_eq!(text_nodes("<a>\n  <b/>\n</a>", true), ok(&[]));
    }

    #[test]
    fn untrimmed_nodes_are_verbatim() {
        assert_eq!(text_nodes("<a> x &amp; y </a>", false), ok(&[" x & y "]));
        assert_eq!(text_nodes("<a> <b/></a>", false), ok(&[" "]));
    }

    #[test]
    fn markup_ends_a_text_node() {
        assert_eq!(
            text_nodes("<a>one<!-- c -->two<b/> three </a>", true),
            ok(&["one", "two", "three"])
        );
    }

    #[test]
    fn malformed_references_are_unescape_errors() {
        for xml in ["<a>x &nbsp; y</a>", "<a>x & y</a>", "<a>x &amp y</a>"] {
            let nodes = text_nodes(xml, true);
            assert_eq!(nodes.len(), 1, "{xml}");
            assert!(nodes[0].is_err(), "{xml}: {:?}", nodes[0]);
        }
    }

    /// Reads the value of the root element of `xml` with [`ElementText`].
    fn element_value(xml: &str) -> Result<Option<String>, String> {
        let mut reader = Reader::from_reader(xml.as_bytes());
        let mut element_text = ElementText::for_reader(reader.config_mut());
        let mut buf = Vec::new();
        assert!(matches!(
            reader.read_event_into(&mut buf),
            Ok(Event::Start(_))
        ));
        element_text.read(&mut reader)
    }

    fn some(value: &str) -> Result<Option<String>, String> {
        Ok(Some(value.to_string()))
    }

    #[test]
    fn element_value_is_all_character_data_in_document_order() {
        assert_eq!(element_value("<a>one</a>"), some("one"));
        assert_eq!(element_value("<a>x &amp; y&#x21;</a>"), some("x & y!"));
        assert_eq!(element_value("<a>one<!-- c -->two</a>"), some("onetwo"));
        assert_eq!(element_value("<a>one<?pi data?>two</a>"), some("onetwo"));
        assert_eq!(
            element_value("<a>one<![CDATA[<two>&amp;]]>three</a>"),
            some("one<two>&amp;three")
        );
        assert_eq!(
            element_value("<a><!-- c --><![CDATA[o]]>n<?pi?>e<!-- c --></a>"),
            some("one")
        );
    }

    #[test]
    fn element_value_is_trimmed_once_around_the_whole_value() {
        assert_eq!(element_value("<a>\n  one\t</a>"), some("one"));
        assert_eq!(
            element_value("<a> one <!-- c --> two </a>"),
            some("one  two")
        );
        assert_eq!(element_value("<a> <!-- c --> one <?pi?> </a>"), some("one"));
        // Whitespace written as a reference or in a CDATA section is
        // content, not padding.
        assert_eq!(element_value("<a> &#32;one&#32; </a>"), some(" one "));
        assert_eq!(element_value("<a> <![CDATA[ one ]]> </a>"), some(" one "));
        assert_eq!(element_value("<a>one <![CDATA[]]> </a>"), some("one"));
    }

    #[test]
    fn element_without_character_data_has_no_value() {
        for xml in [
            "<a></a>",
            "<a> \n </a>",
            "<a><!-- c --></a>",
            "<a> <?pi data?> </a>",
            "<a><![CDATA[]]></a>",
        ] {
            assert_eq!(element_value(xml), Ok(None), "{xml}");
        }
    }

    #[test]
    fn element_value_ends_with_the_end_tag_of_the_element() {
        let mut reader = Reader::from_reader("<r><a>one</a><b>two</b></r>".as_bytes());
        let mut element_text = ElementText::for_reader(reader.config_mut());
        let mut buf = Vec::new();
        let mut names = Vec::new();
        let mut values = Vec::new();
        loop {
            let event = reader.read_event_into(&mut buf).unwrap();
            let name = match &event {
                Event::Start(start) => String::from_utf8(start.name().as_ref().to_vec()).unwrap(),
                Event::Eof => break,
                _ => continue,
            };
            if name != "r" {
                values.push(element_text.read(&mut reader));
            }
            names.push(name);
            buf.clear();
        }
        assert_eq!(names, ["r", "a", "b"]);
        assert_eq!(values, [some("one"), some("two")]);
    }

    #[test]
    fn element_value_ends_with_the_document() {
        let mut reader = Reader::from_reader("<a>one".as_bytes());
        let mut element_text = ElementText::for_reader(reader.config_mut());
        let mut buf = Vec::new();
        assert!(matches!(
            reader.read_event_into(&mut buf),
            Ok(Event::Start(_))
        ));
        assert_eq!(element_text.read(&mut reader), some("one"));
        assert!(matches!(reader.read_event_into(&mut buf), Ok(Event::Eof)));
    }

    #[test]
    fn element_value_errors_are_parse_error_texts() {
        for xml in ["<a>one<b/>two</a>", "<a><b>one</b></a>", "<a>one<b></a>"] {
            assert_eq!(
                element_value(xml),
                Err(format!("xml: {CHILD_ELEMENT_NOT_ALLOWED}")),
                "{xml}"
            );
        }
        assert_eq!(
            element_value("<a>one<!DOCTYPE a>two</a>"),
            Err(format!("xml: {DOCTYPE_NOT_ALLOWED}"))
        );
        // A malformed reference, also where the two halves of what looks
        // like a reference are not in the same text node.
        for xml in [
            "<a>x &nbsp; y</a>",
            "<a>x & y</a>",
            "<a>x<!-- c -->&nbsp;</a>",
            "<a>&am<!-- c -->p;</a>",
            "<a>&<![CDATA[amp;]]></a>",
            "<a>&nbsp;<b/></a>",
        ] {
            let error = element_value(xml).expect_err(xml);
            assert!(!error.starts_with("xml: "), "{xml}: {error}");
        }
        // Reader errors keep the prefix the parsers give them.
        for xml in ["<a>one</b>", "<a>one<!-- c"] {
            let error = element_value(xml).expect_err(xml);
            assert!(error.starts_with("xml: "), "{xml}: {error}");
            assert!(!error.contains(CHILD_ELEMENT_NOT_ALLOWED), "{xml}: {error}");
        }
    }

    #[test]
    fn tag_attributes_reports_an_attribute_that_cannot_be_read() {
        fn keys(xml: &str) -> Result<Vec<String>, String> {
            let mut reader = Reader::from_str(xml);
            let Ok(Event::Empty(e)) = reader.read_event() else {
                panic!("expected an empty element");
            };
            tag_attributes(&e)
                .map(|a| a.map(|a| String::from_utf8(a.key.as_ref().to_vec()).unwrap()))
                .collect()
        }

        assert_eq!(
            keys(r#"<a x="1" y='2'/>"#),
            Ok(vec!["x".to_string(), "y".to_string()])
        );
        for xml in [r#"<a x="1" x="2"/>"#, r#"<a x="1" y/>"#, "<a x=1/>"] {
            let error = keys(xml).expect_err(xml);
            assert!(error.starts_with("xml: "), "{xml}: {error}");
        }
    }

    #[test]
    fn attr_value_resolves_references_and_keeps_whitespace() {
        let mut reader = Reader::from_str("<a v=\"x&amp;y&#x20;\tz\n\"/>");
        let Ok(Event::Empty(e)) = reader.read_event() else {
            panic!("expected an empty element");
        };
        let attr = e.attributes().next().unwrap().unwrap();
        assert_eq!(attr_value(&attr).unwrap(), "x&y \tz\n");

        let mut reader = Reader::from_str("<a v=\"x &nbsp; y\"/>");
        let Ok(Event::Empty(e)) = reader.read_event() else {
            panic!("expected an empty element");
        };
        let attr = e.attributes().next().unwrap().unwrap();
        assert!(attr_value(&attr).is_err());
    }
}
