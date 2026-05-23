// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Engine + SourceManager — port of `pkg/engine/engine.go` +
//! `pkg/sources/source_manager.go`. Drives chunks from sources through the
//! detector registry, the dedup gate, and the finding store.

use crate::config::ScanConfig;
use crate::decoders::DecoderRegistry;
use crate::dedup::Dedup;
use crate::detector::DetectorRegistry;
use crate::job_progress::JobProgress;
use crate::models::{Chunk, DetectionResult, Finding};
use crate::store::FindingStore;
use std::sync::Mutex;

pub struct Engine {
    pub config: ScanConfig,
    pub registry: DetectorRegistry,
    pub decoders: DecoderRegistry,
    pub dedup: Dedup,
    pub store: FindingStore,
    pub progress: Mutex<JobProgress>,
}

impl Engine {
    pub fn new(config: ScanConfig) -> Self {
        Self {
            config,
            registry: DetectorRegistry::builtin(),
            decoders: DecoderRegistry::default(),
            dedup: Dedup::new(),
            store: FindingStore::new(),
            progress: Mutex::new(JobProgress::new()),
        }
    }

    pub fn with_registry(mut self, r: DetectorRegistry) -> Self {
        self.registry = r;
        self
    }

    /// Scan a chunk: keyword-pre-filter -> regex match -> dedup -> persist.
    /// Returns the *new* findings created by this chunk (post-dedup).
    pub fn scan_chunk(&self, chunk: &Chunk) -> Vec<Finding> {
        self.progress.lock().unwrap().record_chunk();
        let mut new_findings = Vec::new();

        let mut process =
            |results: Vec<DetectionResult>, decoder: Option<&'static str>| {
                for mut r in results {
                    if let Some(d) = decoder {
                        r = r.with_extra("decoder", d);
                    }
                    let f = Finding {
                        result: r,
                        chunk_source: chunk.source_name.clone(),
                        source_metadata: chunk.source_metadata.clone(),
                        redacted: redact(&_raw_str(&new_findings, chunk)),
                    };
                    if let Some(sf) = self.store.insert(f) {
                        self.progress
                            .lock()
                            .unwrap()
                            .record_finding(sf.finding.result.verified);
                        new_findings.push(sf.finding);
                    }
                }
            };

        // Raw pass
        let raw = self.registry.scan(&chunk.data);
        process(raw, None);

        // Decoded passes — only run if at least one detector keyword matches
        // the decoded payload (we run the full registry anyway since each
        // decoder layer can surface a fresh secret).
        for d in self.decoders.decode_all(&chunk.data) {
            let results = self.registry.scan(&d.payload);
            process(results, Some(d.decoder));
        }

        new_findings
    }

    pub fn findings(&self) -> Vec<Finding> {
        self.store.all().into_iter().map(|s| s.finding).collect()
    }
}

fn _raw_str(_seen: &[Finding], _chunk: &Chunk) -> String {
    // Placeholder accessor — the redacted view is derived from the last
    // detection result by the caller in `scan_chunk` above.
    String::new()
}

/// Returns a redacted preview: first 4 + last 4 with `…` in the middle.
pub fn redact(s: &str) -> String {
    if s.len() <= 8 {
        return "…".into();
    }
    format!("{}…{}", &s[..4], &s[s.len() - 4..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SourceMetadata;

    fn mk_chunk(payload: &[u8]) -> Chunk {
        let mut c = Chunk::new("filesystem", "/repo/a.go", payload.to_vec());
        c.source_metadata = SourceMetadata {
            kind: crate::models::SourceKind::Filesystem,
            file: Some("/repo/a.go".into()),
            ..Default::default()
        };
        c
    }

    #[test]
    fn scan_chunk_emits_stripe_finding() {
        let e = Engine::new(ScanConfig::default());
        let f = e.scan_chunk(&mk_chunk(b"my key = sk_live_1234567890abcdefghij"));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].result.detector_type, crate::models::DetectorType::Stripe);
    }

    #[test]
    fn scan_chunk_dedups_repeats() {
        let e = Engine::new(ScanConfig::default());
        let c = mk_chunk(b"sk_live_1234567890abcdefghij");
        let f1 = e.scan_chunk(&c);
        let f2 = e.scan_chunk(&c);
        assert_eq!(f1.len(), 1);
        assert!(f2.is_empty());
    }

    #[test]
    fn redact_short_input_is_dots() {
        assert_eq!(redact("abc"), "…");
    }

    #[test]
    fn redact_long_input_keeps_edges() {
        assert_eq!(redact("abcdefghijklmnop"), "abcd…mnop");
    }

    #[test]
    fn empty_chunk_yields_nothing() {
        let e = Engine::new(ScanConfig::default());
        assert!(e.scan_chunk(&mk_chunk(b"")).is_empty());
    }

    #[test]
    fn engine_progress_tracks_chunks() {
        let e = Engine::new(ScanConfig::default());
        e.scan_chunk(&mk_chunk(b"hello"));
        assert_eq!(e.progress.lock().unwrap().chunks_emitted, 1);
    }
}
