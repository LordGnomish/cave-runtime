// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Embedding-model catalog.
//!
//! infinity loads HuggingFace `sentence-transformers`-style models by id and
//! reads their pooling / normalization config + (for asymmetric retrieval
//! families) the query/passage instruction prefixes. We ship a static catalog
//! of the well-known families so the server can resolve a model id to its
//! native dimensionality, default pooling, and instruction template without a
//! network round-trip.

use crate::pooling::PoolingStrategy;

/// Model family — drives the default instruction-prefix convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    /// sentence-transformers symmetric models (all-MiniLM, all-mpnet, …).
    SentenceTransformers,
    /// BAAI BGE retrieval family (asymmetric query instruction).
    Bge,
    /// intfloat E5 family (`query:` / `passage:` prefixes).
    E5,
    /// nomic-embed-text family (`search_query:` / `search_document:`, Matryoshka).
    Nomic,
    /// Jina embeddings family (long context, mean pooling).
    Jina,
    /// Mistral-style decoder embedders (last-token pooling, 4096-d).
    Mistral,
}

/// Static description of a registered embedding model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCard {
    /// HuggingFace-style model id.
    pub id: &'static str,
    /// Family.
    pub family: ModelFamily,
    /// Native output dimensionality.
    pub dims: usize,
    /// Maximum input sequence length (tokens).
    pub max_seq_len: usize,
    /// Default pooling strategy.
    pub pooling: PoolingStrategy,
    /// Whether embeddings are L2-normalized by default.
    pub normalize: bool,
    /// Instruction prepended to queries (asymmetric retrieval), if any.
    pub query_instruction: Option<&'static str>,
    /// Instruction prepended to passages/documents, if any.
    pub passage_instruction: Option<&'static str>,
    /// Matryoshka-valid truncation dimensions (empty = no Matryoshka support).
    pub matryoshka_dims: &'static [usize],
}

impl ModelCard {
    /// Render a query with the family's query instruction applied.
    pub fn format_query(&self, text: &str) -> String {
        match self.query_instruction {
            Some(p) => format!("{p}{text}"),
            None => text.to_string(),
        }
    }

    /// Render a passage/document with the family's passage instruction applied.
    pub fn format_passage(&self, text: &str) -> String {
        match self.passage_instruction {
            Some(p) => format!("{p}{text}"),
            None => text.to_string(),
        }
    }

    /// Is `d` a valid output dimensionality for this model? Always true for
    /// the native dim; otherwise only Matryoshka-declared truncations.
    pub fn supports_dim(&self, d: usize) -> bool {
        d == self.dims || self.matryoshka_dims.contains(&d)
    }
}

/// Catalog of built-in model cards.
#[derive(Debug, Clone)]
pub struct ModelCatalog {
    cards: Vec<ModelCard>,
}

impl ModelCatalog {
    /// Build the static built-in catalog.
    pub fn builtin() -> Self {
        // PLACEHOLDER (RED): empty catalog.
        Self { cards: Vec::new() }
    }

    /// Look up a model by exact id.
    pub fn get(&self, id: &str) -> Option<&ModelCard> {
        self.cards.iter().find(|c| c.id == id)
    }

    /// All registered model ids.
    pub fn ids(&self) -> Vec<&'static str> {
        self.cards.iter().map(|c| c.id).collect()
    }

    /// Cards belonging to a family.
    pub fn by_family(&self, family: ModelFamily) -> Vec<&ModelCard> {
        self.cards.iter().filter(|c| c.family == family).collect()
    }

    /// Number of registered models.
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_all_six_families() {
        let c = ModelCatalog::builtin();
        for f in [
            ModelFamily::SentenceTransformers,
            ModelFamily::Bge,
            ModelFamily::E5,
            ModelFamily::Nomic,
            ModelFamily::Jina,
            ModelFamily::Mistral,
        ] {
            assert!(!c.by_family(f).is_empty(), "missing family {f:?}");
        }
    }

    #[test]
    fn minilm_is_384_mean_normalized() {
        let c = ModelCatalog::builtin();
        let m = c.get("sentence-transformers/all-MiniLM-L6-v2").unwrap();
        assert_eq!(m.dims, 384);
        assert_eq!(m.pooling, PoolingStrategy::Mean);
        assert!(m.normalize);
        assert_eq!(m.family, ModelFamily::SentenceTransformers);
    }

    #[test]
    fn bge_large_uses_cls_and_query_instruction() {
        let c = ModelCatalog::builtin();
        let m = c.get("BAAI/bge-large-en-v1.5").unwrap();
        assert_eq!(m.dims, 1024);
        assert_eq!(m.pooling, PoolingStrategy::Cls);
        let q = m.format_query("hello");
        assert!(q.starts_with("Represent this sentence"));
        assert!(q.ends_with("hello"));
        // BGE applies no passage instruction.
        assert_eq!(m.format_passage("doc"), "doc");
    }

    #[test]
    fn e5_uses_query_passage_prefixes() {
        let c = ModelCatalog::builtin();
        let m = c.get("intfloat/e5-large-v2").unwrap();
        assert_eq!(m.pooling, PoolingStrategy::Mean);
        assert_eq!(m.format_query("q"), "query: q");
        assert_eq!(m.format_passage("p"), "passage: p");
    }

    #[test]
    fn nomic_prefixes_and_matryoshka() {
        let c = ModelCatalog::builtin();
        let m = c.get("nomic-ai/nomic-embed-text-v1.5").unwrap();
        assert_eq!(m.dims, 768);
        assert_eq!(m.format_query("q"), "search_query: q");
        assert_eq!(m.format_passage("p"), "search_document: p");
        assert!(m.supports_dim(256), "nomic supports Matryoshka 256");
        assert!(m.supports_dim(768));
        assert!(!m.supports_dim(777));
    }

    #[test]
    fn mistral_uses_last_token_pooling() {
        let c = ModelCatalog::builtin();
        let m = c.get("intfloat/e5-mistral-7b-instruct").unwrap();
        assert_eq!(m.dims, 4096);
        assert_eq!(m.pooling, PoolingStrategy::LastToken);
        assert_eq!(m.family, ModelFamily::Mistral);
    }

    #[test]
    fn unknown_id_is_none() {
        assert!(ModelCatalog::builtin().get("nope/not-a-model").is_none());
    }
}
