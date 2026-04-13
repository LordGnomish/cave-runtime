//! Inverted index core — BM25 scoring, posting lists, and document store.
//!
//! Each `IndexData` is a self-contained unit: it holds the document store, the
//! inverted index, per-field statistics used for BM25, and derived length maps.
//! The `BuiltinSearchEngine` holds all indices in a single `RwLock`-protected
//! `HashMap` and delegates every operation to the appropriate `IndexData`.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

use crate::index::{analyze_text, is_text_field};
use crate::models::{
    BulkActionMeta, BulkError, BulkItemResult, BulkResponse, CountResponse, Document, FieldType,
    IndexInfo, IndexMapping, IndexSettings, IndexSettingsWrapper, IndexStats, SearchHit,
    SearchRequest, SearchResponse, ShardStats, SortField, SortOrder, TotalHits, TotalRelation,
};
use crate::query::execute_query;
use crate::SearchError;

// ─────────────────────────────────────────────────────────────────────────────
// BM25 parameters
// ─────────────────────────────────────────────────────────────────────────────

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

// ─────────────────────────────────────────────────────────────────────────────
// Posting lists
// ─────────────────────────────────────────────────────────────────────────────

/// A single occurrence entry for a term in a document field.
#[derive(Debug, Clone)]
pub struct Posting {
    pub doc_id: String,
    pub term_frequency: u32,
    pub positions: Vec<u32>,
}

/// Complete posting list for a (field, term) pair.
#[derive(Debug, Clone, Default)]
pub struct PostingList {
    pub postings: Vec<Posting>,
}

impl PostingList {
    /// Number of documents that contain this term.
    pub fn doc_frequency(&self) -> usize {
        self.postings.len()
    }

    pub fn get(&self, doc_id: &str) -> Option<&Posting> {
        self.postings.iter().find(|p| p.doc_id == doc_id)
    }

    pub fn remove(&mut self, doc_id: &str) {
        self.postings.retain(|p| p.doc_id != doc_id);
    }

    pub fn upsert(&mut self, doc_id: String, term_frequency: u32, positions: Vec<u32>) {
        if let Some(idx) = self.postings.iter().position(|p| p.doc_id == doc_id) {
            self.postings[idx].term_frequency = term_frequency;
            self.postings[idx].positions = positions;
        } else {
            self.postings.push(Posting { doc_id, term_frequency, positions });
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Field statistics
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregate statistics for a field across all indexed documents.
#[derive(Debug, Clone, Default)]
pub struct FieldStats {
    pub doc_count: usize,
    pub total_term_count: u64,
    pub avg_doc_length: f64,
}

impl FieldStats {
    pub fn add_doc(&mut self, term_count: usize) {
        self.doc_count += 1;
        self.total_term_count += term_count as u64;
        self.avg_doc_length = self.total_term_count as f64 / self.doc_count as f64;
    }

    pub fn remove_doc(&mut self, term_count: usize) {
        if self.doc_count == 0 {
            return;
        }
        self.doc_count -= 1;
        self.total_term_count = self.total_term_count.saturating_sub(term_count as u64);
        self.avg_doc_length = if self.doc_count > 0 {
            self.total_term_count as f64 / self.doc_count as f64
        } else {
            0.0
        };
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Index data
// ─────────────────────────────────────────────────────────────────────────────

/// All mutable state for a single logical index.
pub struct IndexData {
    pub name: String,
    pub mapping: IndexMapping,
    pub settings: IndexSettings,
    /// Primary document store: doc_id → Document.
    pub documents: HashMap<String, Document>,
    /// Inverted index: field → term → PostingList.
    pub inverted_index: HashMap<String, HashMap<String, PostingList>>,
    /// Field aggregate stats.
    pub field_stats: HashMap<String, FieldStats>,
    /// Per-document field term counts (for BM25 length normalisation).
    pub doc_field_lengths: HashMap<String, HashMap<String, usize>>,
    /// Alias names pointing to this index.
    pub aliases: Vec<String>,
    pub stats: IndexStats,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl IndexData {
    pub fn new(name: impl Into<String>, mapping: IndexMapping, settings: IndexSettings) -> Self {
        Self {
            name: name.into(),
            mapping,
            settings,
            documents: HashMap::new(),
            inverted_index: HashMap::new(),
            field_stats: HashMap::new(),
            doc_field_lengths: HashMap::new(),
            aliases: Vec::new(),
            stats: IndexStats::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn doc_count(&self) -> usize {
        self.documents.len()
    }

    // ── Indexing ────────────────────────────────────────────────────────────

    /// Index a document, returning its assigned ID.
    pub fn add_document(&mut self, mut doc: Document) -> String {
        let id = doc.id.clone();

        // Replace previous version if exists.
        if self.documents.contains_key(&id) {
            self.remove_document(&id);
            if let Some(v) = doc.version {
                doc.version = Some(v + 1);
            }
        }

        let text_fields = doc.all_text_fields();
        let mut doc_lengths: HashMap<String, usize> = HashMap::new();

        for (field, text) in text_fields {
            if !self.should_index(&field) {
                continue;
            }

            let field_mapping = self.mapping.properties.get(&field);
            let as_text = is_text_field(field_mapping);

            if !as_text {
                // Keyword / numeric: index as single lowercase token.
                let token = text.to_lowercase();
                self.index_term(&field, &token, &id, 1, vec![0]);
                *doc_lengths.entry(field).or_default() += 1;
                continue;
            }

            let tokens = analyze_text(&text, &self.mapping, &field);
            let mut term_positions: HashMap<String, (u32, Vec<u32>)> = HashMap::new();
            let mut pos = 0u32;

            for token in &tokens {
                let e = term_positions.entry(token.text.clone()).or_insert((0, Vec::new()));
                e.0 += 1;
                e.1.push(pos);
                pos += 1;
            }

            let term_count = tokens.len();
            *doc_lengths.entry(field.clone()).or_default() += term_count;
            self.field_stats.entry(field.clone()).or_default().add_doc(term_count);

            for (term, (freq, positions)) in term_positions {
                self.index_term(&field, &term, &id, freq, positions);
            }
        }

        // Also index keyword fields not covered by all_text_fields.
        // Collect first to avoid holding an immutable borrow on self.mapping while calling
        // self.index_term (which needs &mut self).
        let keyword_fields: Vec<(String, Option<String>)> = self.mapping.properties.iter()
            .filter(|(_, m)| m.field_type == FieldType::Keyword)
            .map(|(f, _)| (f.clone(), doc.get_field_text(f).map(|t| t.to_lowercase())))
            .collect();
        for (field, maybe_text) in keyword_fields {
            if let Some(lower) = maybe_text {
                self.index_term(&field, &lower, &id, 1, vec![0]);
                if !doc_lengths.contains_key(&field) {
                    *doc_lengths.entry(field.clone()).or_default() += 1;
                    self.field_stats.entry(field.clone()).or_default().add_doc(1);
                }
            }
        }

        self.doc_field_lengths.insert(id.clone(), doc_lengths);
        self.documents.insert(id.clone(), doc);
        self.stats.doc_count += 1;
        self.stats.index_total += 1;
        self.updated_at = Utc::now();

        debug!(index = %self.name, doc_id = %id, "document indexed");
        id
    }

    /// Remove a document from the index.  Returns `true` if the document existed.
    pub fn remove_document(&mut self, doc_id: &str) -> bool {
        if !self.documents.contains_key(doc_id) {
            return false;
        }

        for field_map in self.inverted_index.values_mut() {
            for pl in field_map.values_mut() {
                pl.remove(doc_id);
            }
        }

        if let Some(lengths) = self.doc_field_lengths.remove(doc_id) {
            for (field, length) in lengths {
                if let Some(stats) = self.field_stats.get_mut(&field) {
                    stats.remove_doc(length);
                }
            }
        }

        self.documents.remove(doc_id);

        self.stats.doc_count = self.stats.doc_count.saturating_sub(1);
        self.stats.deleted_count += 1;
        self.updated_at = Utc::now();
        true
    }

    fn index_term(
        &mut self,
        field: &str,
        term: &str,
        doc_id: &str,
        freq: u32,
        positions: Vec<u32>,
    ) {
        self.inverted_index
            .entry(field.to_string())
            .or_default()
            .entry(term.to_string())
            .or_default()
            .upsert(doc_id.to_string(), freq, positions);
    }

    fn should_index(&self, field: &str) -> bool {
        self.mapping.properties.get(field).map_or(self.mapping.dynamic, |m| m.index)
    }

    // ── Lookup ──────────────────────────────────────────────────────────────

    pub fn posting_list(&self, field: &str, term: &str) -> Option<&PostingList> {
        self.inverted_index.get(field)?.get(term)
    }

    pub fn field_terms(&self, field: &str) -> impl Iterator<Item = &String> {
        self.inverted_index
            .get(field)
            .into_iter()
            .flat_map(|m| m.keys())
    }

    // ── BM25 ────────────────────────────────────────────────────────────────

    pub fn bm25_score(&self, doc_id: &str, field: &str, term: &str) -> f64 {
        let Some(pl) = self.posting_list(field, term) else { return 0.0 };
        let Some(posting) = pl.get(doc_id) else { return 0.0 };

        let n = self.documents.len() as f64;
        if n == 0.0 {
            return 0.0;
        }

        let df = pl.doc_frequency() as f64;
        let tf = posting.term_frequency as f64;

        let dl = self
            .doc_field_lengths
            .get(doc_id)
            .and_then(|m| m.get(field))
            .copied()
            .unwrap_or(1) as f64;

        let avg_dl = self
            .field_stats
            .get(field)
            .map(|s| s.avg_doc_length)
            .unwrap_or(1.0)
            .max(1.0);

        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
        let tf_norm = tf * (BM25_K1 + 1.0) / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * dl / avg_dl));
        let boost = self.mapping.properties.get(field).map(|m| m.boost).unwrap_or(1.0);

        idf * tf_norm * boost
    }

    // ── Query helpers (called by query.rs) ──────────────────────────────────

    pub fn score_terms(
        &self,
        field: &str,
        terms: &[String],
        operator_and: bool,
    ) -> Vec<(String, f64)> {
        let mut doc_scores: HashMap<String, f64> = HashMap::new();
        let mut doc_term_hits: HashMap<String, usize> = HashMap::new();

        for term in terms {
            if let Some(pl) = self.posting_list(field, term) {
                for posting in &pl.postings {
                    if self.documents.contains_key(&posting.doc_id) {
                        let score = self.bm25_score(&posting.doc_id, field, term);
                        *doc_scores.entry(posting.doc_id.clone()).or_default() += score;
                        *doc_term_hits.entry(posting.doc_id.clone()).or_default() += 1;
                    }
                }
            }
        }

        if operator_and && terms.len() > 1 {
            doc_scores.retain(|id, _| {
                doc_term_hits.get(id).copied().unwrap_or(0) >= terms.len()
            });
        }

        sorted_by_score(doc_scores)
    }

    pub fn match_all(&self) -> Vec<(String, f64)> {
        self.documents.keys().map(|id| (id.clone(), 1.0)).collect()
    }

    pub fn exact_term(&self, field: &str, value: &str) -> Vec<(String, f64)> {
        let Some(pl) = self.posting_list(field, value) else { return vec![] };
        pl.postings
            .iter()
            .filter(|p| self.documents.contains_key(&p.doc_id))
            .map(|p| (p.doc_id.clone(), self.bm25_score(&p.doc_id, field, value)))
            .collect()
    }

    pub fn terms_any(&self, field: &str, values: &[String]) -> Vec<(String, f64)> {
        let mut scores: HashMap<String, f64> = HashMap::new();
        for v in values {
            for (id, s) in self.exact_term(field, v) {
                *scores.entry(id).or_default() += s;
            }
        }
        sorted_by_score(scores)
    }

    pub fn range_match(
        &self,
        field: &str,
        gte: Option<&Value>,
        gt: Option<&Value>,
        lte: Option<&Value>,
        lt: Option<&Value>,
    ) -> Vec<(String, f64)> {
        self.documents
            .iter()
            .filter_map(|(id, doc)| {
                let fv = doc.get_field(field)?;
                if check_range(fv, gte, gt, lte, lt) {
                    Some((id.clone(), 1.0))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn prefix_match(&self, field: &str, prefix: &str) -> Vec<(String, f64)> {
        let mut scores: HashMap<String, f64> = HashMap::new();
        if let Some(field_map) = self.inverted_index.get(field) {
            for (term, pl) in field_map {
                if term.starts_with(prefix) {
                    for p in &pl.postings {
                        if self.documents.contains_key(&p.doc_id) {
                            let score = self.bm25_score(&p.doc_id, field, term);
                            *scores.entry(p.doc_id.clone()).or_default() += score;
                        }
                    }
                }
            }
        }
        sorted_by_score(scores)
    }

    pub fn fuzzy_match(&self, field: &str, query_term: &str, fuzziness: u32) -> Vec<(String, f64)> {
        let max_dist = fuzziness.min(2) as usize;
        let mut scores: HashMap<String, f64> = HashMap::new();
        if let Some(field_map) = self.inverted_index.get(field) {
            for (term, pl) in field_map {
                let dist = levenshtein(query_term, term);
                if dist <= max_dist {
                    let boost = 1.0 / (dist as f64 + 1.0);
                    for p in &pl.postings {
                        if self.documents.contains_key(&p.doc_id) {
                            let score = self.bm25_score(&p.doc_id, field, term) * boost;
                            *scores.entry(p.doc_id.clone()).or_default() += score;
                        }
                    }
                }
            }
        }
        sorted_by_score(scores)
    }

    pub fn wildcard_match(&self, field: &str, pattern: &str) -> Vec<(String, f64)> {
        let re_pattern = wildcard_to_regex(pattern);
        let Ok(re) = regex::Regex::new(&re_pattern) else {
            warn!(pattern, "invalid wildcard pattern");
            return vec![];
        };
        let mut scores: HashMap<String, f64> = HashMap::new();
        if let Some(field_map) = self.inverted_index.get(field) {
            for (term, pl) in field_map {
                if re.is_match(term) {
                    for p in &pl.postings {
                        if self.documents.contains_key(&p.doc_id) {
                            let score = self.bm25_score(&p.doc_id, field, term);
                            *scores.entry(p.doc_id.clone()).or_default() += score;
                        }
                    }
                }
            }
        }
        sorted_by_score(scores)
    }

    pub fn exists_match(&self, field: &str) -> Vec<(String, f64)> {
        self.documents
            .iter()
            .filter(|(_, doc)| doc.get_field(field).map_or(false, |v| !v.is_null()))
            .map(|(id, _)| (id.clone(), 1.0))
            .collect()
    }

    pub fn ids_match(&self, ids: &[String]) -> Vec<(String, f64)> {
        ids.iter()
            .filter(|id| self.documents.contains_key(*id))
            .map(|id| (id.clone(), 1.0))
            .collect()
    }

    // ── Highlight ────────────────────────────────────────────────────────────

    /// Generate highlighted snippets for `field` given a set of query terms.
    pub fn highlight(
        &self,
        doc_id: &str,
        field: &str,
        terms: &[String],
        fragment_size: usize,
        pre_tag: &str,
        post_tag: &str,
    ) -> Vec<String> {
        let Some(doc) = self.documents.get(doc_id) else { return vec![] };
        let Some(text) = doc.get_field_text(field) else { return vec![] };

        let mut snippets = Vec::new();
        let lower = text.to_lowercase();
        let mut last_end = 0usize;

        for term in terms {
            let mut search_from = 0;
            while let Some(pos) = lower[search_from..].find(term.as_str()) {
                let abs_pos = search_from + pos;
                let start = abs_pos.saturating_sub(fragment_size / 2);
                let end = (abs_pos + term.len() + fragment_size / 2).min(text.len());

                // Clamp to valid char boundaries.
                let start = find_char_boundary(&text, start);
                let end = find_char_boundary_end(&text, end);

                let prefix = &text[start..abs_pos];
                let matched = &text[abs_pos..abs_pos + term.len()];
                let suffix = &text[abs_pos + term.len()..end];

                snippets.push(format!("{}{}{}{}{}", prefix, pre_tag, matched, post_tag, suffix));
                search_from = abs_pos + term.len();
                last_end = end;

                if snippets.len() >= 5 {
                    break;
                }
            }
            if snippets.len() >= 5 {
                break;
            }
        }

        snippets
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BuiltinSearchEngine
// ─────────────────────────────────────────────────────────────────────────────

/// In-memory search engine backed by an inverted index.
///
/// Thread-safe via `parking_lot::RwLock` — locking scope is kept to the
/// synchronous portion of each handler, so no lock is held across await points.
pub struct BuiltinSearchEngine {
    indices: RwLock<HashMap<String, IndexData>>,
    /// alias → index name.
    aliases: RwLock<HashMap<String, String>>,
}

impl BuiltinSearchEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            indices: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashMap::new()),
        })
    }

    // ── Index lifecycle ──────────────────────────────────────────────────────

    pub fn create_index(
        &self,
        name: &str,
        mapping: IndexMapping,
        settings: IndexSettings,
        alias_map: HashMap<String, crate::models::AliasConfig>,
    ) -> Result<(), SearchError> {
        let mut indices = self.indices.write();
        if indices.contains_key(name) {
            return Err(SearchError::IndexAlreadyExists(name.to_string()));
        }
        let mut idx = IndexData::new(name, mapping, settings);
        let mut aliases = self.aliases.write();
        for (alias, _cfg) in &alias_map {
            idx.aliases.push(alias.clone());
            aliases.insert(alias.clone(), name.to_string());
        }
        indices.insert(name.to_string(), idx);
        Ok(())
    }

    pub fn delete_index(&self, name: &str) -> Result<(), SearchError> {
        let mut indices = self.indices.write();
        let idx = indices.remove(name).ok_or_else(|| SearchError::IndexNotFound(name.to_string()))?;
        let mut aliases = self.aliases.write();
        for alias in &idx.aliases {
            aliases.remove(alias);
        }
        Ok(())
    }

    /// Resolve alias → index name, or return `name` unchanged.
    fn resolve(&self, name: &str) -> String {
        self.aliases.read().get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    pub fn index_exists(&self, name: &str) -> bool {
        let resolved = self.resolve(name);
        self.indices.read().contains_key(&resolved)
    }

    pub fn get_index_info(&self, name: &str) -> Result<IndexInfo, SearchError> {
        let resolved = self.resolve(name);
        let indices = self.indices.read();
        let idx = indices.get(&resolved).ok_or_else(|| SearchError::IndexNotFound(name.to_string()))?;
        Ok(IndexInfo {
            aliases: idx.aliases.iter().map(|a| (a.clone(), serde_json::json!({}))).collect(),
            mappings: idx.mapping.clone(),
            settings: IndexSettingsWrapper { index: idx.settings.clone() },
        })
    }

    pub fn get_index_stats(&self, name: &str) -> Result<IndexStats, SearchError> {
        let resolved = self.resolve(name);
        let indices = self.indices.read();
        let idx = indices.get(&resolved).ok_or_else(|| SearchError::IndexNotFound(name.to_string()))?;
        Ok(idx.stats.clone())
    }

    pub fn list_indices(&self) -> Vec<String> {
        self.indices.read().keys().cloned().collect()
    }

    // ── Document CRUD ────────────────────────────────────────────────────────

    pub fn index_document(&self, index: &str, doc: Document) -> Result<String, SearchError> {
        let resolved = self.resolve(index);
        let mut indices = self.indices.write();

        // Auto-create index if dynamic mapping is implied.
        if !indices.contains_key(&resolved) {
            indices.insert(
                resolved.clone(),
                IndexData::new(&resolved, IndexMapping::default(), IndexSettings::default()),
            );
        }

        let idx = indices.get_mut(&resolved).unwrap();
        Ok(idx.add_document(doc))
    }

    pub fn get_document(&self, index: &str, id: &str) -> Result<Option<Document>, SearchError> {
        let resolved = self.resolve(index);
        let indices = self.indices.read();
        let idx = indices.get(&resolved).ok_or_else(|| SearchError::IndexNotFound(index.to_string()))?;
        Ok(idx.documents.get(id).cloned())
    }

    pub fn delete_document(&self, index: &str, id: &str) -> Result<bool, SearchError> {
        let resolved = self.resolve(index);
        let mut indices = self.indices.write();
        let idx = indices.get_mut(&resolved).ok_or_else(|| SearchError::IndexNotFound(index.to_string()))?;
        Ok(idx.remove_document(id))
    }

    pub fn count(&self, index: &str) -> Result<u64, SearchError> {
        let resolved = self.resolve(index);
        let indices = self.indices.read();
        let idx = indices.get(&resolved).ok_or_else(|| SearchError::IndexNotFound(index.to_string()))?;
        Ok(idx.doc_count() as u64)
    }

    // ── Search ───────────────────────────────────────────────────────────────

    pub fn search(&self, index: &str, req: SearchRequest) -> Result<SearchResponse, SearchError> {
        let start = std::time::Instant::now();
        let resolved = self.resolve(index);
        let indices = self.indices.read();
        let idx = indices.get(&resolved).ok_or_else(|| SearchError::IndexNotFound(index.to_string()))?;

        // Execute query.
        let mut results: Vec<(String, f64)> = match &req.query {
            None => idx.match_all(),
            Some(q) if q.is_null() => idx.match_all(),
            Some(q) => execute_query(q, idx),
        };

        // Apply min_score filter.
        if let Some(min_score) = req.min_score {
            results.retain(|(_, score)| *score >= min_score);
        }

        // Apply sort.
        if !req.sort.is_empty() {
            let sort_fields: Vec<SortField> =
                req.sort.iter().filter_map(SortField::parse).collect();
            results = apply_sort(results, &sort_fields, idx);
        } else {
            results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        let total_count = results.len() as u64;

        // Paginate.
        let page: Vec<(String, f64)> = results
            .into_iter()
            .skip(req.from)
            .take(req.size)
            .collect();

        let max_score = page.first().map(|(_, s)| *s);

        // Build hits.
        let mut hits: Vec<SearchHit> = page
            .iter()
            .map(|(doc_id, score)| {
                let doc = idx.documents.get(doc_id).unwrap();
                let highlight = build_highlights(doc_id, &req, idx);
                SearchHit {
                    index: index.to_string(),
                    id: doc_id.clone(),
                    score: Some(*score),
                    source: doc.source.clone(),
                    version: doc.version,
                    highlight,
                    fields: HashMap::new(),
                    sort: Vec::new(),
                }
            })
            .collect();

        let took_ms = start.elapsed().as_millis() as u64;

        Ok(SearchResponse {
            took: took_ms,
            timed_out: false,
            shards: ShardStats::default(),
            hits: crate::models::HitsCollection {
                total: TotalHits { value: total_count, relation: TotalRelation::Eq },
                max_score,
                hits,
            },
            aggregations: build_aggregations(&req, idx),
            scroll_id: None,
        })
    }

    // ── Bulk API ─────────────────────────────────────────────────────────────

    pub fn bulk(
        &self,
        default_index: Option<&str>,
        ops: Vec<(String, BulkActionMeta, Option<Value>)>,
    ) -> BulkResponse {
        let start = std::time::Instant::now();
        let mut items = Vec::new();
        let mut has_errors = false;

        for (action, meta, body) in ops {
            let index_name = meta
                .index
                .as_deref()
                .or(default_index)
                .unwrap_or("_default");
            let resolved = self.resolve(index_name);

            match action.as_str() {
                "index" | "create" => {
                    let Some(body_val) = body else {
                        has_errors = true;
                        items.push(make_bulk_error(&action, index_name, "_unknown", 400, "missing body"));
                        continue;
                    };
                    let source: HashMap<String, Value> =
                        if let Value::Object(map) = body_val {
                            map.into_iter().collect()
                        } else {
                            has_errors = true;
                            items.push(make_bulk_error(&action, index_name, "_unknown", 400, "body must be an object"));
                            continue;
                        };
                    let doc = if let Some(id) = meta.id.clone() {
                        Document::with_id(id, resolved.clone(), source)
                    } else {
                        Document::new(resolved.clone(), source)
                    };
                    let id = doc.id.clone();
                    match self.index_document(index_name, doc) {
                        Ok(returned_id) => {
                            let mut map = HashMap::new();
                            map.insert(action.clone(), BulkItemResult {
                                index: index_name.to_string(),
                                id: returned_id,
                                version: 1,
                                result: "created".into(),
                                status: 201,
                                error: None,
                            });
                            items.push(map);
                        }
                        Err(e) => {
                            has_errors = true;
                            items.push(make_bulk_error(&action, index_name, &id, 500, &e.to_string()));
                        }
                    }
                }
                "delete" => {
                    let id = meta.id.as_deref().unwrap_or("_unknown");
                    match self.delete_document(index_name, id) {
                        Ok(true) => {
                            let mut map = HashMap::new();
                            map.insert(action.clone(), BulkItemResult {
                                index: index_name.to_string(),
                                id: id.to_string(),
                                version: 1,
                                result: "deleted".into(),
                                status: 200,
                                error: None,
                            });
                            items.push(map);
                        }
                        Ok(false) => {
                            has_errors = true;
                            items.push(make_bulk_error(&action, index_name, id, 404, "not found"));
                        }
                        Err(e) => {
                            has_errors = true;
                            items.push(make_bulk_error(&action, index_name, id, 500, &e.to_string()));
                        }
                    }
                }
                _ => {
                    has_errors = true;
                    items.push(make_bulk_error(&action, index_name, "_unknown", 400, "unsupported action"));
                }
            }
        }

        BulkResponse {
            took: start.elapsed().as_millis() as u64,
            errors: has_errors,
            items,
        }
    }
}

impl Default for BuiltinSearchEngine {
    fn default() -> Self {
        Self {
            indices: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashMap::new()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Sort helpers
// ─────────────────────────────────────────────────────────────────────────────

fn apply_sort(
    mut results: Vec<(String, f64)>,
    sort_fields: &[SortField],
    idx: &IndexData,
) -> Vec<(String, f64)> {
    results.sort_by(|a, b| {
        for sf in sort_fields {
            if sf.field == "_score" {
                let cmp = a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal);
                let cmp = if sf.order == SortOrder::Asc { cmp } else { cmp.reverse() };
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
                continue;
            }

            let va = idx.documents.get(&a.0).and_then(|d| d.get_field_text(&sf.field));
            let vb = idx.documents.get(&b.0).and_then(|d| d.get_field_text(&sf.field));

            let cmp = match (va, vb) {
                (Some(va), Some(vb)) => va.cmp(&vb),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };
            let cmp = if sf.order == SortOrder::Asc { cmp } else { cmp.reverse() };
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    });
    results
}

// ─────────────────────────────────────────────────────────────────────────────
// Highlight helpers
// ─────────────────────────────────────────────────────────────────────────────

fn build_highlights(
    doc_id: &str,
    req: &SearchRequest,
    idx: &IndexData,
) -> HashMap<String, Vec<String>> {
    let Some(highlight_cfg) = &req.highlight else { return HashMap::new() };

    let pre_tag = highlight_cfg.pre_tags.first().map(|s| s.as_str()).unwrap_or("<em>");
    let post_tag = highlight_cfg.post_tags.first().map(|s| s.as_str()).unwrap_or("</em>");

    // Collect query terms for highlight matching.
    let query_terms = extract_query_terms(req.query.as_ref());

    let mut result = HashMap::new();
    for (field, field_cfg) in &highlight_cfg.fields {
        let frag_size = field_cfg.fragment_size.unwrap_or(highlight_cfg.fragment_size);
        let snippets = idx.highlight(doc_id, field, &query_terms, frag_size, pre_tag, post_tag);
        if !snippets.is_empty() {
            result.insert(field.clone(), snippets);
        }
    }
    result
}

fn extract_query_terms(query: Option<&Value>) -> Vec<String> {
    let Some(q) = query else { return vec![] };
    let mut terms = Vec::new();
    collect_terms(q, &mut terms);
    terms
}

fn collect_terms(q: &Value, out: &mut Vec<String>) {
    if let Some(match_q) = q.get("match") {
        if let Some(obj) = match_q.as_object() {
            for v in obj.values() {
                match v {
                    Value::String(s) => out.extend(s.split_whitespace().map(str::to_lowercase)),
                    Value::Object(m) => {
                        if let Some(Value::String(s)) = m.get("query") {
                            out.extend(s.split_whitespace().map(str::to_lowercase));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(bool_q) = q.get("bool") {
        for clause in &["must", "should", "filter"] {
            if let Some(arr) = bool_q.get(clause).and_then(|v| v.as_array()) {
                for item in arr {
                    collect_terms(item, out);
                }
            }
        }
    }
    if let Some(term_q) = q.get("term") {
        if let Some(obj) = term_q.as_object() {
            for v in obj.values() {
                match v {
                    Value::String(s) => out.push(s.to_lowercase()),
                    Value::Object(m) => {
                        if let Some(Value::String(s)) = m.get("value") {
                            out.push(s.to_lowercase());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Aggregation stubs
// ─────────────────────────────────────────────────────────────────────────────

fn build_aggregations(
    req: &SearchRequest,
    idx: &IndexData,
) -> HashMap<String, Value> {
    let mut result = HashMap::new();
    for (name, agg_def) in &req.aggregations {
        let val = execute_aggregation(name, agg_def, idx);
        result.insert(name.clone(), val);
    }
    result
}

fn execute_aggregation(name: &str, agg_def: &Value, idx: &IndexData) -> Value {
    // Terms aggregation.
    if let Some(terms_agg) = agg_def.get("terms") {
        let field = terms_agg
            .get("field")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let size = terms_agg.get("size").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let mut counts: HashMap<String, u64> = HashMap::new();
        for doc in idx.documents.values() {
            if let Some(val) = doc.get_field_text(field) {
                *counts.entry(val).or_default() += 1;
            }
        }

        let mut buckets: Vec<_> = counts.into_iter().collect();
        buckets.sort_by(|a, b| b.1.cmp(&a.1));
        buckets.truncate(size);

        let bucket_vals: Vec<Value> = buckets
            .into_iter()
            .map(|(k, c)| serde_json::json!({"key": k, "doc_count": c}))
            .collect();

        return serde_json::json!({"buckets": bucket_vals});
    }

    // Value count aggregation.
    if let Some(vc_agg) = agg_def.get("value_count") {
        let field = vc_agg.get("field").and_then(|v| v.as_str()).unwrap_or("");
        let count = idx.documents.values().filter(|d| d.get_field(field).is_some()).count();
        return serde_json::json!({"value": count});
    }

    // Min/Max/Avg/Sum aggregations.
    for agg_type in &["min", "max", "avg", "sum"] {
        if let Some(stats_agg) = agg_def.get(agg_type) {
            let field = stats_agg.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let values: Vec<f64> = idx
                .documents
                .values()
                .filter_map(|d| d.get_field(field)?.as_f64())
                .collect();

            let result_val = match *agg_type {
                "min" => values.iter().cloned().fold(f64::INFINITY, f64::min),
                "max" => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                "avg" => {
                    if values.is_empty() { 0.0 } else { values.iter().sum::<f64>() / values.len() as f64 }
                }
                "sum" => values.iter().sum(),
                _ => 0.0,
            };

            return serde_json::json!({"value": result_val});
        }
    }

    serde_json::json!({})
}

// ─────────────────────────────────────────────────────────────────────────────
// Utility functions
// ─────────────────────────────────────────────────────────────────────────────

fn sorted_by_score(map: HashMap<String, f64>) -> Vec<(String, f64)> {
    let mut v: Vec<_> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

fn make_bulk_error(
    action: &str,
    index: &str,
    id: &str,
    status: u16,
    reason: &str,
) -> HashMap<String, BulkItemResult> {
    let mut map = HashMap::new();
    map.insert(action.to_string(), BulkItemResult {
        index: index.to_string(),
        id: id.to_string(),
        version: 0,
        result: "error".into(),
        status,
        error: Some(BulkError {
            error_type: "search_error".into(),
            reason: reason.to_string(),
            status,
        }),
    });
    map
}

/// Levenshtein edit distance (bounded to avoid O(n²) on long strings).
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 { return n; }
    if n == 0 { return m; }
    if (m as isize - n as isize).unsigned_abs() > 3 { return 4; }

    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[m][n]
}

/// Translate a wildcard pattern (`*`, `?`) into a regex pattern.
pub fn wildcard_to_regex(pattern: &str) -> String {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c if ".+()[]{}^$|\\".contains(c) => { re.push('\\'); re.push(c); }
            c => re.push(c),
        }
    }
    re.push('$');
    re
}

/// Numeric range comparison for `serde_json::Value`.
fn check_range(
    fv: &Value,
    gte: Option<&Value>,
    gt: Option<&Value>,
    lte: Option<&Value>,
    lt: Option<&Value>,
) -> bool {
    if let Some(num) = fv.as_f64() {
        let ok = |bound: Option<&Value>, inclusive: bool| {
            bound.map_or(true, |v| {
                let bv = v.as_f64().unwrap_or(0.0);
                if inclusive { num >= bv } else { num > bv }
            })
        };
        return ok(gte, true) && ok(gt, false) && ok(lte, true) && ok(lt, false);
    }
    if let Some(s) = fv.as_str() {
        let ok = |bound: Option<&Value>, inclusive: bool| {
            bound.map_or(true, |v| {
                let bv = v.as_str().unwrap_or("");
                if inclusive { s >= bv } else { s > bv }
            })
        };
        return ok(gte, true) && ok(gt, false) && ok(lte, true) && ok(lt, false);
    }
    false
}

fn find_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) { idx -= 1; }
    idx
}

fn find_char_boundary_end(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    while idx < len && !s.is_char_boundary(idx) { idx += 1; }
    idx.min(len)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FieldMapping, FieldType, IndexMapping};
    use serde_json::json;

    fn text_mapping(fields: &[&str]) -> IndexMapping {
        let mut m = IndexMapping::default();
        for f in fields {
            m.properties.insert(
                f.to_string(),
                FieldMapping { field_type: FieldType::Text, ..Default::default() },
            );
        }
        m
    }

    fn make_doc(id: &str, idx: &str, kv: Vec<(&str, &str)>) -> Document {
        let source = kv.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        Document::with_id(id, idx, source)
    }

    #[test]
    fn add_remove_doc() {
        let mut idx = IndexData::new("test", text_mapping(&["title"]), IndexSettings::default());
        idx.add_document(make_doc("1", "test", vec![("title", "hello world")]));
        assert_eq!(idx.doc_count(), 1);
        idx.remove_document("1");
        assert_eq!(idx.doc_count(), 0);
    }

    #[test]
    fn bm25_nonzero() {
        let mut idx = IndexData::new("test", text_mapping(&["title"]), IndexSettings::default());
        idx.add_document(make_doc("1", "test", vec![("title", "rust programming language")]));
        assert!(idx.bm25_score("1", "title", "rust") > 0.0);
    }

    #[test]
    fn score_terms_finds_docs() {
        let mut idx = IndexData::new("test", text_mapping(&["title"]), IndexSettings::default());
        idx.add_document(make_doc("1", "test", vec![("title", "rust lang")]));
        idx.add_document(make_doc("2", "test", vec![("title", "python data")]));
        let res = idx.score_terms("title", &["rust".to_string()], false);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, "1");
    }

    #[test]
    fn fuzzy_match_tolerates_typo() {
        let mut idx = IndexData::new("test", text_mapping(&["title"]), IndexSettings::default());
        idx.add_document(make_doc("1", "test", vec![("title", "rustlang programming")]));
        let res = idx.fuzzy_match("title", "ruslang", 1);
        assert!(!res.is_empty());
    }

    #[test]
    fn wildcard_star() {
        let mut idx = IndexData::new("test", text_mapping(&["title"]), IndexSettings::default());
        idx.add_document(make_doc("1", "test", vec![("title", "testing framework")]));
        let res = idx.wildcard_match("title", "test*");
        assert!(!res.is_empty());
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("rust", "rust"), 0);
        assert_eq!(levenshtein("rust", "bust"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn engine_create_and_search() {
        let engine = BuiltinSearchEngine::new();
        engine
            .create_index("blog", text_mapping(&["title", "body"]), IndexSettings::default(), HashMap::new())
            .unwrap();

        let doc = make_doc("1", "blog", vec![("title", "hello world"), ("body", "this is a test")]);
        engine.index_document("blog", doc).unwrap();

        let req = SearchRequest {
            query: Some(json!({"match": {"title": "hello"}})),
            ..Default::default()
        };
        let resp = engine.search("blog", req).unwrap();
        assert_eq!(resp.hits.total.value, 1);
    }

    #[test]
    fn engine_delete_document() {
        let engine = BuiltinSearchEngine::new();
        engine
            .create_index("test", IndexMapping::default(), IndexSettings::default(), HashMap::new())
            .unwrap();
        let doc = make_doc("1", "test", vec![("title", "hello")]);
        engine.index_document("test", doc).unwrap();
        assert!(engine.delete_document("test", "1").unwrap());
        assert!(!engine.delete_document("test", "1").unwrap());
    }
}
