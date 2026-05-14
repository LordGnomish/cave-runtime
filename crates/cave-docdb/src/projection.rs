// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Projection evaluation for MongoDB-like field inclusion/exclusion.

use crate::bson::Document;
use serde_json::Value;

pub fn apply_projection(doc: &Document, projection: Option<&Document>) -> Document {
    match projection {
        None => doc.clone(),
        Some(proj) => {
            let mut has_includes = false;

            for (_key, value) in proj {
                if let Some(num) = value.as_i64() {
                    if num == 1 {
                        has_includes = true;
                    }
                } else if let Some(b) = value.as_bool() {
                    if b {
                        has_includes = true;
                    }
                }
            }

            if has_includes {
                // Inclusion mode: start empty, add specified fields
                let mut result = Document::new();
                for (key, value) in proj {
                    if let Some(num) = value.as_i64() {
                        if num == 1 {
                            if let Some(v) = doc.get(key) {
                                result.insert(key.clone(), v.clone());
                            }
                        }
                    }
                }
                // Always include _id unless explicitly excluded
                if !proj.contains_key("_id") || proj.get("_id").map(|v| v != &Value::Number(0.into())).unwrap_or(true) {
                    if let Some(id) = doc.get("_id") {
                        result.insert("_id".to_string(), id.clone());
                    }
                }
                result
            } else {
                // Exclusion mode: start with all, remove specified fields
                let mut result = doc.clone();
                for (key, value) in proj {
                    if let Some(num) = value.as_i64() {
                        if num == 0 {
                            result.remove(key);
                        }
                    } else if let Some(false) = value.as_bool() {
                        result.remove(key);
                    }
                }
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inclusion_projection() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("alice".to_string()));
        doc.insert("email".to_string(), Value::String("alice@example.com".to_string()));

        let mut proj = Document::new();
        proj.insert("name".to_string(), Value::Number(1.into()));

        let result = apply_projection(&doc, Some(&proj));
        assert!(result.contains_key("name"));
        assert!(result.contains_key("_id")); // _id included by default
        assert!(!result.contains_key("email"));
    }

    #[test]
    fn test_exclusion_projection() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("alice".to_string()));
        doc.insert("email".to_string(), Value::String("alice@example.com".to_string()));
        doc.insert("password".to_string(), Value::String("secret".to_string()));

        let mut proj = Document::new();
        proj.insert("password".to_string(), Value::Number(0.into()));

        let result = apply_projection(&doc, Some(&proj));
        assert!(result.contains_key("name"));
        assert!(result.contains_key("_id"));
        assert!(result.contains_key("email"));
        assert!(!result.contains_key("password"));
    }

    #[test]
    fn test_no_projection() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("alice".to_string()));

        let result = apply_projection(&doc, None);
        assert_eq!(result, doc);
    }
}
