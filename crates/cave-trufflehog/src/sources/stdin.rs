// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Stdin source — port of `pkg/sources/stdin/stdin.go`. Reads an
//! `impl Read` (defaulting to `std::io::stdin`) and chunks the payload.

use crate::chunker::Chunker;
use crate::error::Result;
use crate::models::{Chunk, SourceKind, SourceMetadata};

pub struct StdinSource {
    pub chunker: Chunker,
    pub payload: Option<Vec<u8>>,
}

impl Default for StdinSource {
    fn default() -> Self {
        Self {
            chunker: Chunker::default(),
            payload: None,
        }
    }
}

impl StdinSource {
    pub fn with_payload(payload: Vec<u8>) -> Self {
        Self {
            chunker: Chunker::default(),
            payload: Some(payload),
        }
    }

    pub fn name(&self) -> &str {
        "stdin"
    }

    pub fn chunks(&self) -> Result<Vec<Chunk>> {
        let payload = match &self.payload {
            Some(p) => p.clone(),
            None => {
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
                    .map_err(|e| crate::error::Error::Source(e.to_string()))?;
                buf
            }
        };
        Ok(self
            .chunker
            .chunk_bytes(&payload)
            .into_iter()
            .map(|cb| {
                let mut c = Chunk::new("stdin", "<stdin>", cb.data);
                c.source_metadata = SourceMetadata {
                    kind: SourceKind::Stdin,
                    file: Some("<stdin>".into()),
                    ..Default::default()
                };
                c
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_payload_chunks_input() {
        let s = StdinSource::with_payload(b"hello".to_vec());
        let c = s.chunks().unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].data, b"hello");
        assert_eq!(c[0].source_metadata.kind, SourceKind::Stdin);
    }

    #[test]
    fn empty_payload_yields_no_chunks() {
        let s = StdinSource::with_payload(Vec::new());
        assert!(s.chunks().unwrap().is_empty());
    }
}
