//! Per-index document store + inverted index over keyword fields.

use std::collections::{BTreeMap, BTreeSet};

use parking_lot::RwLock;
use serde_json::Value;

use crate::error::{SearchError, SearchResult};
use crate::mapping::{FieldType, IndexMapping};

#[derive(Debug, Clone)]
pub struct StoredDoc {
    pub id: String,
    pub source: Value,
    pub seq_no: u64,
    pub primary_term: u64,
    pub version: u64,
}

#[derive(Debug, Default)]
pub struct IndexData {
    pub mapping: IndexMapping,
    /// id → document
    pub docs: BTreeMap<String, StoredDoc>,
    /// (field, term) → set of doc ids — inverted index over keyword fields.
    pub inverted: BTreeMap<(String, String), BTreeSet<String>>,
    /// Per-index sequence number generator.
    pub seq_no: u64,
    pub primary_term: u64,
    pub created_at_ms: i64,
}

#[derive(Debug, Default)]
pub struct IndexRegistry {
    indices: RwLock<BTreeMap<String, IndexData>>,
}

impl IndexRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&self, name: &str, mapping: IndexMapping) -> SearchResult<()> {
        let mut idx = self.indices.write();
        if idx.contains_key(name) {
            return Err(SearchError::IndexExists(name.into()));
        }
        idx.insert(
            name.into(),
            IndexData {
                mapping,
                docs: BTreeMap::new(),
                inverted: BTreeMap::new(),
                seq_no: 0,
                primary_term: 1,
                created_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
        Ok(())
    }

    pub fn delete(&self, name: &str) -> SearchResult<()> {
        let mut idx = self.indices.write();
        idx.remove(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?;
        Ok(())
    }

    pub fn exists(&self, name: &str) -> bool {
        self.indices.read().contains_key(name)
    }

    pub fn list(&self) -> Vec<String> {
        self.indices.read().keys().cloned().collect()
    }

    pub fn doc_count(&self, name: &str) -> SearchResult<usize> {
        let idx = self.indices.read();
        Ok(idx
            .get(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?
            .docs
            .len())
    }

    /// Index (insert or replace) a document.
    pub fn index_doc(&self, name: &str, id: &str, source: Value) -> SearchResult<StoredDoc> {
        let mut idx = self.indices.write();
        let data = idx
            .get_mut(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?;
        data.mapping
            .validate_document(&source)
            .map_err(SearchError::InvalidQuery)?;
        let version = data.docs.get(id).map(|d| d.version + 1).unwrap_or(1);
        data.seq_no += 1;
        let stored = StoredDoc {
            id: id.into(),
            source: source.clone(),
            seq_no: data.seq_no,
            primary_term: data.primary_term,
            version,
        };
        // Remove previous postings for this id.
        for (_, ids) in data.inverted.iter_mut() {
            ids.remove(id);
        }
        // Re-add postings for keyword fields.
        if let Some(obj) = source.as_object() {
            for (k, v) in obj {
                if let Some(ft) = data.mapping.field_type(k) {
                    if matches!(ft, FieldType::Keyword | FieldType::Text) {
                        if let Some(s) = v.as_str() {
                            data.inverted
                                .entry((k.clone(), s.to_string()))
                                .or_default()
                                .insert(id.into());
                            if matches!(ft, FieldType::Text) {
                                for token in tokenize(s) {
                                    data.inverted
                                        .entry((k.clone(), token))
                                        .or_default()
                                        .insert(id.into());
                                }
                            }
                        }
                    }
                }
            }
        }
        data.docs.insert(id.into(), stored.clone());
        Ok(stored)
    }

    pub fn get(&self, name: &str, id: &str) -> SearchResult<StoredDoc> {
        let idx = self.indices.read();
        let data = idx
            .get(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?;
        data.docs
            .get(id)
            .cloned()
            .ok_or_else(|| SearchError::DocumentNotFound(name.into(), id.into()))
    }

    pub fn delete_doc(&self, name: &str, id: &str) -> SearchResult<()> {
        let mut idx = self.indices.write();
        let data = idx
            .get_mut(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?;
        let removed = data.docs.remove(id).is_some();
        if !removed {
            return Err(SearchError::DocumentNotFound(name.into(), id.into()));
        }
        for (_, ids) in data.inverted.iter_mut() {
            ids.remove(id);
        }
        Ok(())
    }

    /// Borrow the underlying data for query evaluation. Must be released
    /// before any further `index_doc`/`delete` call to avoid deadlock.
    pub fn with_index<F, T>(&self, name: &str, f: F) -> SearchResult<T>
    where
        F: FnOnce(&IndexData) -> T,
    {
        let idx = self.indices.read();
        let data = idx
            .get(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?;
        Ok(f(data))
    }

    /// Merge new field mappings into an existing index. Existing fields
    /// keep their type; conflicting redefinitions return InvalidMapping.
    pub fn put_mapping(&self, name: &str, mapping: IndexMapping) -> SearchResult<()> {
        let mut idx = self.indices.write();
        let data = idx
            .get_mut(name)
            .ok_or_else(|| SearchError::IndexNotFound(name.into()))?;
        for (field, fm) in mapping.properties {
            if let Some(existing) = data.mapping.properties.get(&field) {
                if existing.field_type != fm.field_type {
                    return Err(SearchError::InvalidMapping(format!(
                        "cannot redefine field [{}]: {} -> {}",
                        field,
                        existing.field_type.as_str(),
                        fm.field_type.as_str()
                    )));
                }
            }
            data.mapping.properties.insert(field, fm);
        }
        Ok(())
    }
}

pub fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mapping_with(name: &str, ty: FieldType) -> IndexMapping {
        let mut m = IndexMapping::new();
        m.add_field(name, ty);
        m
    }

    #[test]
    fn create_then_list_index() {
        let r = IndexRegistry::new();
        r.create("books", IndexMapping::new()).unwrap();
        assert_eq!(r.list(), vec!["books"]);
        assert!(r.exists("books"));
    }

    #[test]
    fn create_duplicate_index_errors() {
        let r = IndexRegistry::new();
        r.create("books", IndexMapping::new()).unwrap();
        let err = r.create("books", IndexMapping::new()).unwrap_err();
        assert_eq!(err, SearchError::IndexExists("books".into()));
    }

    #[test]
    fn delete_then_exists_false() {
        let r = IndexRegistry::new();
        r.create("books", IndexMapping::new()).unwrap();
        r.delete("books").unwrap();
        assert!(!r.exists("books"));
    }

    #[test]
    fn delete_unknown_errors() {
        let r = IndexRegistry::new();
        let err = r.delete("missing").unwrap_err();
        assert!(matches!(err, SearchError::IndexNotFound(_)));
    }

    #[test]
    fn index_doc_then_get() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("title", FieldType::Keyword)).unwrap();
        r.index_doc("b", "1", json!({"title": "rust"})).unwrap();
        let d = r.get("b", "1").unwrap();
        assert_eq!(d.source["title"], "rust");
        assert_eq!(d.version, 1);
    }

    #[test]
    fn re_index_increments_version() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("title", FieldType::Keyword)).unwrap();
        r.index_doc("b", "1", json!({"title": "v1"})).unwrap();
        let d = r.index_doc("b", "1", json!({"title": "v2"})).unwrap();
        assert_eq!(d.version, 2);
    }

    #[test]
    fn seq_no_is_monotonic() {
        let r = IndexRegistry::new();
        r.create("b", IndexMapping::new()).unwrap();
        let d1 = r.index_doc("b", "1", json!({})).unwrap();
        let d2 = r.index_doc("b", "2", json!({})).unwrap();
        assert!(d2.seq_no > d1.seq_no);
    }

    #[test]
    fn doc_count_reflects_index_state() {
        let r = IndexRegistry::new();
        r.create("b", IndexMapping::new()).unwrap();
        r.index_doc("b", "1", json!({})).unwrap();
        r.index_doc("b", "2", json!({})).unwrap();
        assert_eq!(r.doc_count("b").unwrap(), 2);
    }

    #[test]
    fn delete_doc_removes() {
        let r = IndexRegistry::new();
        r.create("b", IndexMapping::new()).unwrap();
        r.index_doc("b", "1", json!({})).unwrap();
        r.delete_doc("b", "1").unwrap();
        assert!(matches!(
            r.get("b", "1"),
            Err(SearchError::DocumentNotFound(_, _))
        ));
    }

    #[test]
    fn delete_doc_unknown_errors() {
        let r = IndexRegistry::new();
        r.create("b", IndexMapping::new()).unwrap();
        let err = r.delete_doc("b", "missing").unwrap_err();
        assert!(matches!(err, SearchError::DocumentNotFound(_, _)));
    }

    #[test]
    fn index_doc_invalid_type_errors() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("n", FieldType::Long)).unwrap();
        let err = r.index_doc("b", "1", json!({"n": "string"})).unwrap_err();
        assert!(matches!(err, SearchError::InvalidQuery(_)));
    }

    #[test]
    fn inverted_index_records_keyword_postings() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("color", FieldType::Keyword)).unwrap();
        r.index_doc("b", "1", json!({"color": "red"})).unwrap();
        r.index_doc("b", "2", json!({"color": "blue"})).unwrap();
        r.with_index("b", |d| {
            assert!(d.inverted.contains_key(&("color".into(), "red".into())));
            assert!(d.inverted.contains_key(&("color".into(), "blue".into())));
        })
        .unwrap();
    }

    #[test]
    fn re_index_replaces_old_postings() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("color", FieldType::Keyword)).unwrap();
        r.index_doc("b", "1", json!({"color": "red"})).unwrap();
        r.index_doc("b", "1", json!({"color": "green"})).unwrap();
        r.with_index("b", |d| {
            let red = d
                .inverted
                .get(&("color".into(), "red".into()))
                .map(|s| s.contains("1"))
                .unwrap_or(false);
            assert!(!red);
            let green = d
                .inverted
                .get(&("color".into(), "green".into()))
                .map(|s| s.contains("1"))
                .unwrap_or(false);
            assert!(green);
        })
        .unwrap();
    }

    #[test]
    fn text_field_emits_tokens() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("body", FieldType::Text)).unwrap();
        r.index_doc("b", "1", json!({"body": "Hello, World!"})).unwrap();
        r.with_index("b", |d| {
            assert!(d.inverted.contains_key(&("body".into(), "hello".into())));
            assert!(d.inverted.contains_key(&("body".into(), "world".into())));
        })
        .unwrap();
    }

    #[test]
    fn tokenize_splits_on_non_alnum_lowercases() {
        assert_eq!(
            tokenize("Hello, World! 123"),
            vec!["hello", "world", "123"]
        );
    }

    #[test]
    fn put_mapping_adds_new_field() {
        let r = IndexRegistry::new();
        r.create("b", IndexMapping::new()).unwrap();
        let mut update = IndexMapping::new();
        update.add_field("color", FieldType::Keyword);
        r.put_mapping("b", update).unwrap();
        r.with_index("b", |d| {
            assert_eq!(d.mapping.field_type("color"), Some(FieldType::Keyword));
        })
        .unwrap();
    }

    #[test]
    fn put_mapping_rejects_type_change() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("n", FieldType::Long)).unwrap();
        let mut update = IndexMapping::new();
        update.add_field("n", FieldType::Keyword);
        let err = r.put_mapping("b", update).unwrap_err();
        assert!(matches!(err, SearchError::InvalidMapping(_)));
    }

    #[test]
    fn put_mapping_unknown_index_errors() {
        let r = IndexRegistry::new();
        let err = r.put_mapping("missing", IndexMapping::new()).unwrap_err();
        assert!(matches!(err, SearchError::IndexNotFound(_)));
    }

    #[test]
    fn delete_doc_clears_inverted_postings() {
        let r = IndexRegistry::new();
        r.create("b", mapping_with("color", FieldType::Keyword)).unwrap();
        r.index_doc("b", "1", json!({"color": "red"})).unwrap();
        r.delete_doc("b", "1").unwrap();
        r.with_index("b", |d| {
            let still_there = d
                .inverted
                .get(&("color".into(), "red".into()))
                .map(|s| s.contains("1"))
                .unwrap_or(false);
            assert!(!still_there);
        })
        .unwrap();
    }
}
