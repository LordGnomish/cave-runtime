// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document loaders: text / markdown / html / code / pdf.

use cave_rag::loaders::{self, Loader};
use flate2::{write::ZlibEncoder, Compression};
use std::io::Write;

#[test]
fn text_loader_reads_file_and_sets_source() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("note.txt");
    std::fs::write(&p, "plain text body").unwrap();
    let docs = loaders::TextLoader::new(&p).load().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].content, "plain text body");
    assert_eq!(docs[0].metadata.source.as_deref(), Some(p.to_str().unwrap()));
}

#[test]
fn markdown_loader_strips_syntax_to_plain_text() {
    let md = "# Title\n\nSome **bold** and a [link](http://x). \n\n- item one\n- item two";
    let text = loaders::load_markdown_str(md);
    assert!(text.contains("Title"));
    assert!(text.contains("bold"));
    assert!(text.contains("link"));
    assert!(text.contains("item one"));
    // syntax markers gone
    assert!(!text.contains("**"));
    assert!(!text.contains("](http"));
    assert!(!text.contains('#'));
}

#[test]
fn html_loader_strips_tags_scripts_and_decodes_entities() {
    let html = "<html><head><style>.x{color:red}</style></head>\
        <body><script>alert('x')</script><h1>Heading</h1>\
        <p>Tom &amp; Jerry &lt;3 &nbsp;done</p></body></html>";
    let text = loaders::load_html_str(html);
    assert!(text.contains("Heading"));
    assert!(text.contains("Tom & Jerry <3"));
    assert!(text.contains("done"));
    assert!(!text.contains("alert"), "script body must be dropped");
    assert!(!text.contains("color:red"), "style body must be dropped");
    assert!(!text.contains('<'), "no residual tags");
}

#[test]
fn code_loader_keeps_source_and_detects_language() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.rs");
    std::fs::write(&p, "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    let docs = loaders::CodeLoader::new(&p).load().unwrap();
    assert_eq!(docs.len(), 1);
    assert!(docs[0].content.contains("fn main()"));
    assert_eq!(docs[0].metadata.get("language"), Some("rust"));
}

fn make_pdf(stream_body: &[u8], compressed: bool) -> Vec<u8> {
    // Minimal one-object content stream PDF. Enough for the extractor.
    let (filter, data) = if compressed {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(stream_body).unwrap();
        (" /Filter /FlateDecode", e.finish().unwrap())
    } else {
        ("", stream_body.to_vec())
    };
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n");
    pdf.extend_from_slice(
        format!("4 0 obj\n<< /Length {}{} >>\nstream\n", data.len(), filter).as_bytes(),
    );
    pdf.extend_from_slice(&data);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    pdf
}

#[test]
fn pdf_loader_extracts_text_from_uncompressed_stream() {
    let body = b"BT /F1 12 Tf (Hello PDF) Tj ET";
    let pdf = make_pdf(body, false);
    let text = loaders::extract_pdf_text(&pdf).unwrap();
    assert!(text.contains("Hello PDF"), "got: {text:?}");
}

#[test]
fn pdf_loader_inflates_flate_stream_and_handles_tj_array() {
    let body = b"BT [(Hel) -250 (lo) (World)] TJ ET";
    let pdf = make_pdf(body, true);
    let text = loaders::extract_pdf_text(&pdf).unwrap();
    assert!(text.contains("Hello"), "got: {text:?}");
    assert!(text.contains("World"), "got: {text:?}");
}

#[test]
fn load_path_dispatches_by_extension() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("a.md");
    std::fs::write(&md, "# H\n\nbody").unwrap();
    let docs = loaders::load_path(&md).unwrap();
    assert!(docs[0].content.contains("body"));
    assert!(!docs[0].content.contains('#'));
    assert_eq!(docs[0].metadata.source.as_deref(), Some(md.to_str().unwrap()));
}
