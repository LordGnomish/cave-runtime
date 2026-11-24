// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Text-node and attribute-value helpers for the quick-xml event
//! parsers in `saml` and `wsfed`.
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
//! [`attr_value`] is the attribute-side counterpart: it resolves the
//! references of an attribute value and changes nothing else.
//! (`Attribute::normalized_value` additionally turns tab, CR and LF
//! into spaces.)

use std::borrow::Cow;

use quick_xml::encoding::EncodingError;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::events::attributes::Attribute;
use quick_xml::reader::Config;

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

/// Decodes the raw bytes of a text node as UTF-8 and resolves the
/// predefined entities and character references in it.
pub(crate) fn unescape_text(raw: &[u8]) -> Result<Cow<'_, str>, quick_xml::Error> {
    let decoded = std::str::from_utf8(raw).map_err(EncodingError::from)?;
    Ok(unescape(decoded)?)
}

/// Decodes an attribute value as UTF-8 and resolves the predefined
/// entities and character references in it. Whitespace is kept as
/// written.
pub(crate) fn attr_value<'v>(attr: &'v Attribute<'_>) -> Result<Cow<'v, str>, quick_xml::Error> {
    unescape_text(&attr.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::reader::Reader;

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
