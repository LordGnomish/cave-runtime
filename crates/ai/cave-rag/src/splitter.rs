// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Text splitters.
//!
//! A faithful port of langchain's `RecursiveCharacterTextSplitter`: try a
//! prioritised list of separators, splitting on the first that appears, and
//! recurse into any piece still larger than `chunk_size`. Pieces are then
//! greedily merged back up to `chunk_size` with a sliding `chunk_overlap`.
//!
//! The separator is kept at the *start* of the following piece (langchain's
//! `keep_separator="start"`) and every emitted chunk is whitespace-trimmed —
//! which is what lets the same engine serve both prose (clean paragraph
//! chunks) and code ([`for_language`](RecursiveCharacterTextSplitter::for_language)
//! keeps `fn`/`struct`/… boundaries attached to their bodies).
//!
//! The semantic splitter ([`SemanticSplitter`]) lives in [`crate::embedding`]
//! because it needs an [`Embeddings`](crate::embedding::Embeddings) backend.

use crate::document::Document;

/// Recursive character text splitter.
#[derive(Debug, Clone)]
pub struct RecursiveCharacterTextSplitter {
    separators: Vec<String>,
    chunk_size: usize,
    chunk_overlap: usize,
}

impl Default for RecursiveCharacterTextSplitter {
    fn default() -> Self {
        RecursiveCharacterTextSplitter::new(vec![
            "\n\n".into(),
            "\n".into(),
            " ".into(),
            "".into(),
        ])
    }
}

impl RecursiveCharacterTextSplitter {
    /// Construct with an explicit separator priority list (last is usually
    /// `""` for the character-level fallback).
    pub fn new(separators: Vec<String>) -> Self {
        RecursiveCharacterTextSplitter {
            separators,
            chunk_size: 1000,
            chunk_overlap: 200,
        }
    }

    /// Target maximum chunk size (in characters).
    pub fn with_chunk_size(mut self, n: usize) -> Self {
        self.chunk_size = n;
        self
    }

    /// Overlap (in characters) carried between adjacent chunks.
    pub fn with_chunk_overlap(mut self, n: usize) -> Self {
        self.chunk_overlap = n;
        self
    }

    /// Language-aware separators (langchain `from_language`): split on
    /// top-level declarations first so functions/classes stay intact.
    pub fn for_language(language: &str) -> Self {
        let seps: Vec<&str> = match language {
            "rust" => vec![
                "\nfn ", "\nconst ", "\nlet ", "\nstatic ", "\nstruct ", "\nenum ", "\nunion ",
                "\nimpl ", "\ntrait ", "\nmod ", "\npub ", "\nmacro_rules!", "\n\n", "\n", " ", "",
            ],
            "python" => vec![
                "\nclass ", "\ndef ", "\n\tdef ", "\n\n", "\n", " ", "",
            ],
            "javascript" | "typescript" => vec![
                "\nfunction ", "\nconst ", "\nlet ", "\nvar ", "\nclass ", "\nif ", "\nfor ",
                "\nwhile ", "\n\n", "\n", " ", "",
            ],
            "go" => vec![
                "\nfunc ", "\nvar ", "\nconst ", "\ntype ", "\n\n", "\n", " ", "",
            ],
            "markdown" => vec![
                "\n## ", "\n### ", "\n#### ", "\n# ", "\n\n", "\n", " ", "",
            ],
            _ => vec!["\n\n", "\n", " ", ""],
        };
        RecursiveCharacterTextSplitter::new(seps.into_iter().map(String::from).collect())
    }

    /// Split a string into chunks.
    pub fn split_text(&self, text: &str) -> Vec<String> {
        let chunks = self.split_recursive(text, &self.separators);
        chunks
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect()
    }

    /// Split documents, preserving metadata and stamping a `chunk` index.
    pub fn split_documents(&self, docs: &[Document]) -> Vec<Document> {
        let mut out = Vec::new();
        for doc in docs {
            for (i, piece) in self.split_text(&doc.content).into_iter().enumerate() {
                let mut meta = doc.metadata.clone();
                meta.set("chunk", i.to_string());
                out.push(Document {
                    content: piece,
                    metadata: meta,
                });
            }
        }
        out
    }

    fn split_recursive(&self, text: &str, separators: &[String]) -> Vec<String> {
        let mut final_chunks = Vec::new();

        // Pick the first separator that occurs in `text` (or the last one).
        let mut separator = separators.last().cloned().unwrap_or_default();
        let mut remaining: &[String] = &[];
        for (i, sep) in separators.iter().enumerate() {
            if sep.is_empty() {
                separator = sep.clone();
                remaining = &separators[i + 1..];
                break;
            }
            if text.contains(sep.as_str()) {
                separator = sep.clone();
                remaining = &separators[i + 1..];
                break;
            }
        }

        let splits = split_keep_start(text, &separator);
        let mut good: Vec<String> = Vec::new();
        for s in splits {
            if char_len(&s) < self.chunk_size {
                good.push(s);
            } else {
                if !good.is_empty() {
                    final_chunks.extend(self.merge_splits(&good));
                    good.clear();
                }
                if remaining.is_empty() {
                    final_chunks.push(s);
                } else {
                    final_chunks.extend(self.split_recursive(&s, remaining));
                }
            }
        }
        if !good.is_empty() {
            final_chunks.extend(self.merge_splits(&good));
        }
        final_chunks
    }

    /// langchain `_merge_splits`: greedily pack pieces up to `chunk_size`,
    /// sliding a `chunk_overlap` window between adjacent chunks. The
    /// separator is already embedded in the pieces, so the join is empty.
    fn merge_splits(&self, splits: &[String]) -> Vec<String> {
        let mut docs = Vec::new();
        let mut current: Vec<&str> = Vec::new();
        let mut total = 0usize;
        for d in splits {
            let len = char_len(d);
            if total + len > self.chunk_size && !current.is_empty() {
                docs.push(current.concat());
                // Slide the window: drop from the front while we're over the
                // overlap budget, or while the next piece still won't fit.
                while total > self.chunk_overlap
                    || (total + len > self.chunk_size && total > 0)
                {
                    if current.is_empty() {
                        break;
                    }
                    total -= char_len(current[0]);
                    current.remove(0);
                }
            }
            current.push(d);
            total += len;
        }
        if !current.is_empty() {
            docs.push(current.concat());
        }
        docs
    }
}

/// Character length (langchain measures in code points, not bytes).
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Split `text` on `separator`, keeping the separator attached to the start
/// of each following piece (langchain `keep_separator="start"`). An empty
/// separator splits into individual characters.
fn split_keep_start(text: &str, separator: &str) -> Vec<String> {
    if separator.is_empty() {
        return text.chars().map(|c| c.to_string()).collect();
    }
    let mut out = Vec::new();
    let mut first = true;
    for part in text.split(separator) {
        if first {
            if !part.is_empty() {
                out.push(part.to_string());
            }
            first = false;
        } else {
            out.push(format!("{separator}{part}"));
        }
    }
    out
}
