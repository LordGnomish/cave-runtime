//! Relevance scoring (BM25).
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/index/similarity/

pub fn bm25_score(
    _term_freq: u32,
    _doc_len: u32,
    _avg_doc_len: f64,
    _doc_freq: u32,
    _num_docs: u32,
) -> f64 {
    unimplemented!("cave-search::scoring::bm25_score")
}
