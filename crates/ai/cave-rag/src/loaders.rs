// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Document loaders.
//!
//! Mirrors langchain's `document_loaders` package: each [`Loader`] turns an
//! external artefact into one or more [`Document`]s with `source` metadata
//! set. Five concrete loaders ship here — text, markdown, html, source code,
//! and PDF — plus a [`load_path`] dispatcher that selects by file extension.
//!
//! All loaders are pure-Rust and offline: the markdown loader uses
//! `pulldown-cmark`, the PDF loader inflates `FlateDecode` content streams via
//! `flate2` and extracts `Tj` / `TJ` text-showing operators directly (no
//! external `pdftotext` binary).

use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;

use crate::document::Document;
use crate::error::{RagError, Result};

/// A source of [`Document`]s.
pub trait Loader {
    /// Load and return the documents this loader is bound to.
    fn load(&self) -> Result<Vec<Document>>;
}

/// Loads a UTF-8 text file verbatim.
pub struct TextLoader {
    path: PathBuf,
}

impl TextLoader {
    /// Bind to a path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        TextLoader {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Loader for TextLoader {
    fn load(&self) -> Result<Vec<Document>> {
        let body = std::fs::read_to_string(&self.path)?;
        Ok(vec![Document::new(body).with_source(self.path.to_string_lossy())])
    }
}

/// Loads a Markdown file, stripping syntax to plain prose.
pub struct MarkdownLoader {
    path: PathBuf,
}

impl MarkdownLoader {
    /// Bind to a path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        MarkdownLoader {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Loader for MarkdownLoader {
    fn load(&self) -> Result<Vec<Document>> {
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(vec![
            Document::new(load_markdown_str(&raw)).with_source(self.path.to_string_lossy()),
        ])
    }
}

/// Strip Markdown to plain text (heading markers, emphasis, link URLs gone;
/// link/image alt text and code spans kept).
pub fn load_markdown_str(md: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let parser = Parser::new(md);
    let mut out = String::new();
    for ev in parser {
        match ev {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push('\n'),
            Event::Start(Tag::Item) => out.push_str("\n"),
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item)
            | Event::End(TagEnd::CodeBlock) => out.push('\n'),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Loads an HTML file as plain text.
pub struct HtmlLoader {
    path: PathBuf,
}

impl HtmlLoader {
    /// Bind to a path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        HtmlLoader {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Loader for HtmlLoader {
    fn load(&self) -> Result<Vec<Document>> {
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(vec![
            Document::new(load_html_str(&raw)).with_source(self.path.to_string_lossy()),
        ])
    }
}

/// Strip HTML markup to plain text: drop `<script>`/`<style>` bodies, remove
/// all tags, decode the common named/numeric entities, collapse whitespace.
pub fn load_html_str(html: &str) -> String {
    let without_blocks = strip_block(&strip_block(html, "script"), "style");
    // Remove tags.
    let mut text = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    for c in without_blocks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = decode_entities(&text);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove `<tag ...> ... </tag>` blocks (case-insensitive) including bodies.
fn strip_block(input: &str, tag: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::new();
    let mut i = 0usize;
    while i < input.len() {
        if lower[i..].starts_with(&open) {
            if let Some(rel) = lower[i..].find(&close) {
                i += rel + close.len();
                continue;
            } else {
                break; // unterminated block — drop the rest
            }
        }
        // advance one char (respect UTF-8 boundaries)
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Decode the handful of HTML entities that matter for prose extraction.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < s.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = s[i..].find(';') {
                let ent = &s[i + 1..i + semi];
                let replacement = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    _ if ent.starts_with('#') => ent[1..]
                        .parse::<u32>()
                        .ok()
                        .and_then(char::from_u32),
                    _ => None,
                };
                if let Some(c) = replacement {
                    out.push(c);
                    i += semi + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Loads a source-code file verbatim, tagging the detected language.
pub struct CodeLoader {
    path: PathBuf,
}

impl CodeLoader {
    /// Bind to a path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        CodeLoader {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Loader for CodeLoader {
    fn load(&self) -> Result<Vec<Document>> {
        let body = std::fs::read_to_string(&self.path)?;
        let mut doc = Document::new(body).with_source(self.path.to_string_lossy());
        if let Some(lang) = language_for_extension(&self.path) {
            doc.metadata.set("language", lang);
        }
        Ok(vec![doc])
    }
}

/// Map a file extension to a language label (used by [`CodeLoader`] and the
/// code-aware splitter).
pub fn language_for_extension(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|e| e.to_str())? {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "hpp" => Some("cpp"),
        "rb" => Some("ruby"),
        "sh" => Some("bash"),
        _ => None,
    }
}

/// Loads a PDF file, extracting text from its content streams.
pub struct PdfLoader {
    path: PathBuf,
}

impl PdfLoader {
    /// Bind to a path.
    pub fn new(path: impl AsRef<Path>) -> Self {
        PdfLoader {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl Loader for PdfLoader {
    fn load(&self) -> Result<Vec<Document>> {
        let bytes = std::fs::read(&self.path)?;
        let text = extract_pdf_text(&bytes)?;
        Ok(vec![Document::new(text).with_source(self.path.to_string_lossy())])
    }
}

/// Extract visible text from raw PDF bytes.
///
/// Locates every `stream … endstream` body, inflates it when the preceding
/// dictionary declares `/Filter /FlateDecode`, then pulls the operands of the
/// `Tj` (single string) and `TJ` (string/number array) text-showing operators.
/// Kerning numbers inside a `TJ` array are dropped; separate showing operators
/// are joined with a space.
pub fn extract_pdf_text(bytes: &[u8]) -> Result<String> {
    let mut fragments: Vec<String> = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = find(&bytes[search_from..], b"stream") {
        let dict_start = search_from;
        let mut s = search_from + rel + b"stream".len();
        // Skip the EOL after the `stream` keyword (\r\n or \n).
        if bytes.get(s) == Some(&b'\r') {
            s += 1;
        }
        if bytes.get(s) == Some(&b'\n') {
            s += 1;
        }
        let end_rel = find(&bytes[s..], b"endstream")
            .ok_or_else(|| RagError::Loader("unterminated PDF stream".into()))?;
        let raw = &bytes[s..s + end_rel];
        // Inspect the dictionary that precedes this stream for a flate filter.
        let dict = &bytes[dict_start..search_from + rel];
        let decoded: Vec<u8> = if find(dict, b"FlateDecode").is_some() {
            let mut d = ZlibDecoder::new(raw);
            let mut buf = Vec::new();
            d.read_to_end(&mut buf)
                .map_err(|e| RagError::Loader(format!("flate inflate: {e}")))?;
            buf
        } else {
            raw.to_vec()
        };
        fragments.extend(text_ops_from_content(&decoded));
        search_from = s + end_rel + b"endstream".len();
    }
    Ok(fragments.join(" "))
}

/// Find the first occurrence of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Parse `Tj` / `TJ` text-showing operators out of a content-stream body.
fn text_ops_from_content(content: &[u8]) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut pending = String::new(); // strings gathered since the last operator
    let mut i = 0usize;
    while i < content.len() {
        match content[i] {
            b'(' => {
                let (lit, next) = read_pdf_string(content, i + 1);
                pending.push_str(&lit);
                i = next;
            }
            b'T' if content.get(i + 1) == Some(&b'j') || content.get(i + 1) == Some(&b'J') => {
                if !pending.is_empty() {
                    fragments.push(std::mem::take(&mut pending));
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    if !pending.is_empty() {
        fragments.push(pending);
    }
    fragments
}

/// Read a PDF literal string starting just after the opening `(`.
/// Returns the decoded contents and the index just past the closing `)`.
fn read_pdf_string(content: &[u8], mut i: usize) -> (String, usize) {
    let mut out = String::new();
    let mut depth = 1i32;
    while i < content.len() {
        let c = content[i];
        match c {
            b'\\' => {
                i += 1;
                match content.get(i) {
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'b') => out.push('\u{8}'),
                    Some(b'f') => out.push('\u{c}'),
                    Some(b'(') => out.push('('),
                    Some(b')') => out.push(')'),
                    Some(b'\\') => out.push('\\'),
                    Some(d @ b'0'..=b'7') => {
                        // up to 3 octal digits
                        let mut val = (d - b'0') as u32;
                        let mut k = 0;
                        while k < 2 {
                            match content.get(i + 1) {
                                Some(d2 @ b'0'..=b'7') => {
                                    val = val * 8 + (d2 - b'0') as u32;
                                    i += 1;
                                    k += 1;
                                }
                                _ => break,
                            }
                        }
                        if let Some(ch) = char::from_u32(val) {
                            out.push(ch);
                        }
                    }
                    Some(other) => out.push(*other as char),
                    None => {}
                }
                i += 1;
            }
            b'(' => {
                depth += 1;
                out.push('(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return (out, i + 1);
                }
                out.push(')');
                i += 1;
            }
            _ => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    (out, i)
}

/// Dispatch to the right loader based on a file's extension.
pub fn load_path(path: impl AsRef<Path>) -> Result<Vec<Document>> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "md" | "markdown" => MarkdownLoader::new(path).load(),
        "html" | "htm" => HtmlLoader::new(path).load(),
        "pdf" => PdfLoader::new(path).load(),
        "txt" | "text" | "" => TextLoader::new(path).load(),
        _ if language_for_extension(path).is_some() => CodeLoader::new(path).load(),
        _ => TextLoader::new(path).load(),
    }
}
