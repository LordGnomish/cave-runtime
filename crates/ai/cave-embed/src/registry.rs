// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding model registry.
//!
//! Every model infinity serves carries metadata the serving path depends on:
//! output dimensionality, context window, the pooling default, whether outputs
//! are L2-normalized, the modality, and — for asymmetric retrieval models — the
//! role prefixes prepended to queries versus documents (E5, BGE, nomic). The
//! registry resolves a request's `model` field (canonical id or short alias) to
//! a [`ModelCard`] and is extensible via [`ModelRegistry::register`].

use crate::pooling::Pooling;
use std::collections::BTreeMap;

/// What kind of input a model embeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    /// Text-only encoder (the common case).
    Text,
    /// Image-only encoder.
    Image,
    /// Shared text+image space (CLIP-style).
    Multimodal,
}

/// Static metadata describing one embedding model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCard {
    /// Canonical model id (e.g. `BAAI/bge-base-en-v1.5`).
    pub id: String,
    /// Short aliases that also resolve to this card.
    pub aliases: Vec<String>,
    /// Output embedding dimensionality.
    pub dimensions: usize,
    /// Maximum input tokens before truncation.
    pub max_seq_len: usize,
    /// Default pooling strategy for this encoder.
    pub pooling: Pooling,
    /// Whether the serving path L2-normalizes outputs by default.
    pub normalize: bool,
    /// Input modality.
    pub modality: Modality,
    /// Prefix prepended to query inputs (asymmetric retrieval). Empty = none.
    pub query_prefix: String,
    /// Prefix prepended to document/passage inputs. Empty = none.
    pub passage_prefix: String,
}

impl ModelCard {
    /// Build a symmetric (no role prefix) text model card.
    pub fn text(
        id: impl Into<String>,
        dimensions: usize,
        max_seq_len: usize,
        pooling: Pooling,
        normalize: bool,
    ) -> Self {
        ModelCard {
            id: id.into(),
            aliases: Vec::new(),
            dimensions,
            max_seq_len,
            pooling,
            normalize,
            modality: Modality::Text,
            query_prefix: String::new(),
            passage_prefix: String::new(),
        }
    }

    /// Attach short aliases.
    pub fn with_aliases(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Attach asymmetric query/passage prefixes.
    pub fn with_prefixes(mut self, query: &str, passage: &str) -> Self {
        self.query_prefix = query.to_string();
        self.passage_prefix = passage.to_string();
        self
    }

    /// Override the modality.
    pub fn with_modality(mut self, m: Modality) -> Self {
        self.modality = m;
        self
    }

    /// Apply the query prefix (e.g. `query: ...`). Identity when none is set.
    pub fn format_query(&self, text: &str) -> String {
        if self.query_prefix.is_empty() {
            text.to_string()
        } else {
            format!("{}{}", self.query_prefix, text)
        }
    }

    /// Apply the passage prefix (e.g. `passage: ...`). Identity when none is set.
    pub fn format_passage(&self, text: &str) -> String {
        if self.passage_prefix.is_empty() {
            text.to_string()
        } else {
            format!("{}{}", self.passage_prefix, text)
        }
    }
}

/// Resolves a request's `model` field to a [`ModelCard`].
#[derive(Debug, Clone, Default)]
pub struct ModelRegistry {
    by_id: BTreeMap<String, ModelCard>,
    // alias -> canonical id
    alias_index: BTreeMap<String, String>,
}

impl ModelRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-loaded with the built-in catalogue (one representative
    /// model per family the priority list names).
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        for card in builtin_cards() {
            r.register(card);
        }
        r
    }

    /// Insert (or replace) a model card, indexing its id and aliases.
    pub fn register(&mut self, card: ModelCard) {
        for a in &card.aliases {
            self.alias_index.insert(a.clone(), card.id.clone());
        }
        self.by_id.insert(card.id.clone(), card);
    }

    /// Resolve a canonical id or alias to its card.
    pub fn get(&self, name: &str) -> Option<&ModelCard> {
        if let Some(c) = self.by_id.get(name) {
            return Some(c);
        }
        let canonical = self.alias_index.get(name)?;
        self.by_id.get(canonical)
    }

    /// All canonical ids, sorted (stable `/v1/models` listing).
    pub fn list_ids(&self) -> Vec<String> {
        self.by_id.keys().cloned().collect()
    }

    /// Number of registered models.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether the registry holds no models.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// The built-in model catalogue. Metadata reflects the published model cards on
/// the Hugging Face hub (dimensionality, context window, pooling, normalization).
fn builtin_cards() -> Vec<ModelCard> {
    vec![
        // sentence-transformers MiniLM — mean pooling, 384-d, 256 ctx.
        ModelCard::text(
            "sentence-transformers/all-MiniLM-L6-v2",
            384,
            256,
            Pooling::Mean,
            true,
        )
        .with_aliases(&["all-MiniLM-L6-v2", "minilm"]),
        // sentence-transformers MPNet — mean pooling, 768-d.
        ModelCard::text(
            "sentence-transformers/all-mpnet-base-v2",
            768,
            384,
            Pooling::Mean,
            true,
        )
        .with_aliases(&["all-mpnet-base-v2", "mpnet"]),
        // BAAI BGE — CLS pooling, query instruction on the query side.
        ModelCard::text("BAAI/bge-base-en-v1.5", 768, 512, Pooling::Cls, true)
            .with_aliases(&["bge-base-en-v1.5", "bge-base"])
            .with_prefixes(
                "Represent this sentence for searching relevant passages: ",
                "",
            ),
        ModelCard::text("BAAI/bge-large-en-v1.5", 1024, 512, Pooling::Cls, true)
            .with_aliases(&["bge-large-en-v1.5", "bge-large"])
            .with_prefixes(
                "Represent this sentence for searching relevant passages: ",
                "",
            ),
        // intfloat E5 — mean pooling, query:/passage: prefixes.
        ModelCard::text("intfloat/e5-base-v2", 768, 512, Pooling::Mean, true)
            .with_aliases(&["e5-base-v2", "e5-base"])
            .with_prefixes("query: ", "passage: "),
        ModelCard::text(
            "intfloat/multilingual-e5-large",
            1024,
            512,
            Pooling::Mean,
            true,
        )
        .with_aliases(&["multilingual-e5-large", "me5-large"])
        .with_prefixes("query: ", "passage: "),
        // Mistral embed — last-token pooling, 1024-d, long context.
        ModelCard::text("mistralai/mistral-embed", 1024, 8192, Pooling::LastToken, true)
            .with_aliases(&["mistral-embed"]),
        // nomic — mean pooling, search_query/search_document task prefixes.
        ModelCard::text(
            "nomic-ai/nomic-embed-text-v1.5",
            768,
            8192,
            Pooling::Mean,
            true,
        )
        .with_aliases(&["nomic-embed-text-v1.5", "nomic-embed"])
        .with_prefixes("search_query: ", "search_document: "),
        // jina v2 — mean pooling, 768-d, 8k context.
        ModelCard::text(
            "jinaai/jina-embeddings-v2-base-en",
            768,
            8192,
            Pooling::Mean,
            true,
        )
        .with_aliases(&["jina-embeddings-v2-base-en", "jina-v2"]),
        // CLIP — shared text/image space, CLS-style pooling, 512-d.
        ModelCard::text("openai/clip-vit-base-patch32", 512, 77, Pooling::Cls, true)
            .with_aliases(&["clip-vit-base-patch32", "clip"])
            .with_modality(Modality::Multimodal),
    ]
}
