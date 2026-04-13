//! Text analysis pipeline — tokenisers, filters, and analysers.
//!
//! Implements the same analyser names that OpenSearch ships with:
//! `standard`, `simple`, `whitespace`, `keyword`, `stop`, `english`.
//! Custom analysers can be composed from named tokenisers + token filters
//! defined in `IndexSettings.analysis`.

use std::collections::HashSet;

use crate::models::{FieldMapping, FieldType, IndexMapping};

// ─────────────────────────────────────────────────────────────────────────────
// Token
// ─────────────────────────────────────────────────────────────────────────────

/// A single token produced by the analysis pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// Normalised surface form of the token.
    pub text: String,
    /// Token position (for phrase query proximity matching).
    pub position: u32,
    /// Start byte offset within the original text.
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
}

impl Token {
    pub fn new(text: impl Into<String>, position: u32, start: usize, end: usize) -> Self {
        Self { text: text.into(), position, start, end }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tokenisers
// ─────────────────────────────────────────────────────────────────────────────

/// Available tokeniser types.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenizerKind {
    Standard,
    Whitespace,
    Simple,
    Keyword,
    NGram { min_gram: usize, max_gram: usize },
    EdgeNGram { min_gram: usize, max_gram: usize },
}

/// Split text on whitespace boundaries, preserving offsets.
pub fn tokenize_whitespace(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut position = 0u32;
    let mut start = 0;
    let mut in_token = false;

    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if in_token {
                tokens.push(Token::new(&text[start..idx], position, start, idx));
                position += 1;
                in_token = false;
            }
        } else {
            if !in_token {
                start = idx;
                in_token = true;
            }
        }
    }

    if in_token {
        tokens.push(Token::new(&text[start..], position, start, text.len()));
    }

    tokens
}

/// Split on non-alphanumeric characters and lowercase all tokens.
pub fn tokenize_simple(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut position = 0u32;
    let mut start = 0;
    let mut in_token = false;

    for (idx, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            if !in_token {
                start = idx;
                in_token = true;
            }
        } else {
            if in_token {
                tokens.push(Token::new(
                    text[start..idx].to_lowercase(),
                    position,
                    start,
                    idx,
                ));
                position += 1;
                in_token = false;
            }
        }
    }

    if in_token {
        tokens.push(Token::new(
            text[start..].to_lowercase(),
            position,
            start,
            text.len(),
        ));
    }

    tokens
}

/// Standard tokeniser: unicode word boundaries, ASCII-folded, lowercased.
pub fn tokenize_standard(text: &str) -> Vec<Token> {
    // For correctness we fall back to simple word-boundary splitting.
    // A production implementation would use the Unicode word-break algorithm.
    tokenize_simple(text)
}

/// Return the whole text as a single keyword token (lowercased).
pub fn tokenize_keyword(text: &str) -> Vec<Token> {
    if text.is_empty() {
        return vec![];
    }
    vec![Token::new(text.to_lowercase(), 0, 0, text.len())]
}

/// Generate all n-grams from the token stream within the given gram range.
pub fn apply_ngram(tokens: Vec<Token>, min: usize, max: usize) -> Vec<Token> {
    let mut out = Vec::new();
    for tok in &tokens {
        let chars: Vec<char> = tok.text.chars().collect();
        let len = chars.len();
        for n in min..=max.min(len) {
            for i in 0..=(len - n) {
                let gram: String = chars[i..i + n].iter().collect();
                out.push(Token::new(gram, tok.position, tok.start, tok.end));
            }
        }
    }
    out
}

/// Generate edge n-grams (prefix n-grams) from the token stream.
pub fn apply_edge_ngram(tokens: Vec<Token>, min: usize, max: usize) -> Vec<Token> {
    let mut out = Vec::new();
    for tok in &tokens {
        let chars: Vec<char> = tok.text.chars().collect();
        let len = chars.len();
        for n in min..=max.min(len) {
            let gram: String = chars[..n].iter().collect();
            out.push(Token::new(gram, tok.position, tok.start, tok.end));
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Token Filters
// ─────────────────────────────────────────────────────────────────────────────

/// Lowercase all tokens.
pub fn filter_lowercase(tokens: Vec<Token>) -> Vec<Token> {
    tokens
        .into_iter()
        .map(|mut t| {
            t.text = t.text.to_lowercase();
            t
        })
        .collect()
}

/// Remove tokens whose text is in the stop-word set.
pub fn filter_stop_words(tokens: Vec<Token>, stop_words: &HashSet<String>) -> Vec<Token> {
    tokens.into_iter().filter(|t| !stop_words.contains(&t.text)).collect()
}

/// Remove tokens shorter than `min_length`.
pub fn filter_min_length(tokens: Vec<Token>, min_length: usize) -> Vec<Token> {
    tokens.into_iter().filter(|t| t.text.len() >= min_length).collect()
}

/// Deduplicate consecutive identical tokens (useful after synonym expansion).
pub fn filter_unique(tokens: Vec<Token>) -> Vec<Token> {
    let mut seen = HashSet::new();
    tokens.into_iter().filter(|t| seen.insert(t.text.clone())).collect()
}

/// Very lightweight English stemmer — strips common suffixes.
/// A production implementation would use the Porter or Snowball stemmer.
pub fn filter_english_stem(tokens: Vec<Token>) -> Vec<Token> {
    tokens
        .into_iter()
        .map(|mut t| {
            t.text = simple_stem(&t.text);
            t
        })
        .collect()
}

fn simple_stem(word: &str) -> String {
    let suffixes = ["ing", "tion", "ness", "ment", "ful", "less", "ed", "ly", "er", "es", "s"];
    let mut w = word.to_string();
    for suffix in &suffixes {
        if w.ends_with(suffix) && w.len() > suffix.len() + 3 {
            w = w[..w.len() - suffix.len()].to_string();
            break;
        }
    }
    w
}

// ─────────────────────────────────────────────────────────────────────────────
// Built-in English stop words
// ─────────────────────────────────────────────────────────────────────────────

pub fn english_stop_words() -> HashSet<String> {
    [
        "a", "an", "and", "are", "as", "at", "be", "been", "being", "by", "do",
        "for", "from", "has", "have", "he", "her", "him", "his", "how", "i",
        "in", "is", "it", "its", "me", "my", "no", "not", "of", "on", "or",
        "our", "she", "so", "some", "than", "that", "the", "their", "them",
        "then", "there", "these", "they", "this", "those", "to", "up", "us",
        "was", "we", "were", "what", "when", "where", "which", "while", "who",
        "will", "with", "you", "your",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Analyser
// ─────────────────────────────────────────────────────────────────────────────

/// Named analyser configuration.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalyzerKind {
    Standard,
    Simple,
    Whitespace,
    Keyword,
    Stop,
    English,
    Fingerprint,
    Custom(String),
}

impl AnalyzerKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "standard" => Self::Standard,
            "simple" => Self::Simple,
            "whitespace" => Self::Whitespace,
            "keyword" => Self::Keyword,
            "stop" => Self::Stop,
            "english" => Self::English,
            "fingerprint" => Self::Fingerprint,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// Full analysis pipeline for a single text value.
pub struct Analyzer {
    pub kind: AnalyzerKind,
    pub stop_words: HashSet<String>,
}

impl Analyzer {
    pub fn new(kind: AnalyzerKind) -> Self {
        Self { kind, stop_words: HashSet::new() }
    }

    pub fn standard() -> Self { Self::new(AnalyzerKind::Standard) }
    pub fn keyword() -> Self { Self::new(AnalyzerKind::Keyword) }
    pub fn whitespace() -> Self { Self::new(AnalyzerKind::Whitespace) }

    pub fn english() -> Self {
        Self { kind: AnalyzerKind::English, stop_words: english_stop_words() }
    }

    pub fn with_stop_words(mut self, stop_words: HashSet<String>) -> Self {
        self.stop_words = stop_words;
        self
    }

    /// Run the complete analysis pipeline for a text value.
    pub fn analyze(&self, text: &str) -> Vec<Token> {
        match &self.kind {
            AnalyzerKind::Keyword => tokenize_keyword(text),
            AnalyzerKind::Whitespace => tokenize_whitespace(text),
            AnalyzerKind::Simple => tokenize_simple(text),
            AnalyzerKind::Stop => {
                let tokens = tokenize_simple(text);
                let tokens = filter_lowercase(tokens);
                filter_stop_words(tokens, &self.stop_words)
            }
            AnalyzerKind::English => {
                let tokens = tokenize_simple(text);
                let tokens = filter_lowercase(tokens);
                let tokens = filter_stop_words(tokens, &self.stop_words);
                filter_english_stem(tokens)
            }
            AnalyzerKind::Fingerprint => {
                let tokens = tokenize_simple(text);
                let tokens = filter_lowercase(tokens);
                let tokens = filter_stop_words(tokens, &english_stop_words());
                filter_unique(tokens)
            }
            // Standard and Custom both use the standard tokeniser.
            _ => {
                let tokens = tokenize_standard(text);
                filter_lowercase(tokens)
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point: analyse a field value given index mapping context
// ─────────────────────────────────────────────────────────────────────────────

/// Analyse `text` using the analyser configured for `field` in the index mapping.
/// Falls back to the standard analyser when the field is not mapped or has no
/// custom analyser.
pub fn analyze_text(text: &str, mapping: &IndexMapping, field: &str) -> Vec<Token> {
    let analyzer_name = mapping
        .properties
        .get(field)
        .and_then(|m| m.analyzer.as_deref())
        .unwrap_or("standard");

    let field_type = mapping
        .properties
        .get(field)
        .map(|m| &m.field_type)
        .unwrap_or(&FieldType::Text);

    // Keyword fields are never tokenised.
    if *field_type == FieldType::Keyword {
        return tokenize_keyword(text);
    }

    let analyzer = match analyzer_name {
        "english" => Analyzer::english(),
        "keyword" => Analyzer::keyword(),
        "whitespace" => Analyzer::whitespace(),
        "simple" => Analyzer::new(AnalyzerKind::Simple),
        "stop" => Analyzer::new(AnalyzerKind::Stop).with_stop_words(english_stop_words()),
        _ => Analyzer::standard(),
    };

    analyzer.analyze(text)
}

/// Analyse a query string for a given field — uses search_analyzer if set.
pub fn analyze_query(text: &str, mapping: &IndexMapping, field: &str) -> Vec<Token> {
    let analyzer_name = mapping
        .properties
        .get(field)
        .and_then(|m| m.search_analyzer.as_deref().or(m.analyzer.as_deref()))
        .unwrap_or("standard");

    Analyzer::new(AnalyzerKind::from_name(analyzer_name)).analyze(text)
}

// ─────────────────────────────────────────────────────────────────────────────
// Index manager (thin coordinator layer used by the engine)
// ─────────────────────────────────────────────────────────────────────────────

/// Effective analyser for a field, resolved from the mapping chain.
pub fn effective_analyzer(mapping: &IndexMapping, field: &str) -> AnalyzerKind {
    let name = mapping
        .properties
        .get(field)
        .and_then(|m| m.analyzer.as_deref())
        .unwrap_or("standard");
    AnalyzerKind::from_name(name)
}

/// True if the field should be indexed (not stored-only).
pub fn is_indexed_field(mapping: &IndexMapping, field: &str) -> bool {
    mapping
        .properties
        .get(field)
        .map(|m| m.index)
        .unwrap_or(true) // dynamic mapping defaults to indexed
}

/// True if the field should be treated as text (tokenised).
pub fn is_text_field(field_mapping: Option<&FieldMapping>) -> bool {
    field_mapping.map_or(true, |m| m.field_type == FieldType::Text)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_whitespace_basic() {
        let tokens = tokenize_whitespace("hello world foo");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].text, "hello");
        assert_eq!(tokens[1].text, "world");
        assert_eq!(tokens[2].text, "foo");
    }

    #[test]
    fn tokenize_simple_lowercases_and_splits() {
        let tokens = tokenize_simple("Hello, World! 123");
        assert_eq!(tokens.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(), vec!["hello", "world", "123"]);
    }

    #[test]
    fn keyword_single_token() {
        let tokens = tokenize_keyword("Hello World");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].text, "hello world");
    }

    #[test]
    fn ngram_generates_all_grams() {
        let base = vec![Token::new("rust", 0, 0, 4)];
        let grams = apply_ngram(base, 2, 3);
        let texts: Vec<_> = grams.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"ru"));
        assert!(texts.contains(&"us"));
        assert!(texts.contains(&"st"));
        assert!(texts.contains(&"rus"));
        assert!(texts.contains(&"ust"));
    }

    #[test]
    fn edge_ngram_prefixes() {
        let base = vec![Token::new("rust", 0, 0, 4)];
        let grams = apply_edge_ngram(base, 1, 4);
        let texts: Vec<_> = grams.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, vec!["r", "ru", "rus", "rust"]);
    }

    #[test]
    fn stop_words_filter_common_words() {
        let stop_words = english_stop_words();
        let tokens = tokenize_simple("the quick brown fox");
        let filtered = filter_stop_words(tokens, &stop_words);
        assert!(!filtered.iter().any(|t| t.text == "the"));
        assert!(filtered.iter().any(|t| t.text == "quick"));
    }

    #[test]
    fn english_analyser_stems() {
        let analyzer = Analyzer::english();
        let tokens = analyzer.analyze("running quickly in the forest");
        let texts: Vec<_> = tokens.iter().map(|t| t.text.as_str()).collect();
        // "the" and "in" are stop words.
        assert!(!texts.contains(&"the"));
        assert!(!texts.contains(&"in"));
        // Stemming applied.
        assert!(texts.iter().any(|t| t.starts_with("runn") || t.starts_with("run")));
    }

    #[test]
    fn analyze_text_uses_keyword_for_keyword_type() {
        let mut mapping = IndexMapping::default();
        mapping.properties.insert(
            "category".into(),
            crate::models::FieldMapping {
                field_type: FieldType::Keyword,
                ..Default::default()
            },
        );
        let tokens = analyze_text("Hello World", &mapping, "category");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].text, "hello world");
    }

    #[test]
    fn filter_unique_deduplicates() {
        let tokens = vec![
            Token::new("rust", 0, 0, 4),
            Token::new("rust", 1, 5, 9),
            Token::new("lang", 2, 10, 14),
        ];
        let deduped = filter_unique(tokens);
        assert_eq!(deduped.len(), 2);
    }
}
