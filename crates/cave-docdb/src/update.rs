// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Update operator evaluation for MongoDB-like operations.

use crate::bson::Document;
use serde_json::Value;

pub fn apply_update(doc: &mut Document, update: &Document) -> Result<(), String> {
    let mut uses_operators = false;

    for (key, value) in update {
        if key.starts_with('$') {
            uses_operators = true;
            match key.as_str() {
                "$set" => {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            doc.insert(k.clone(), v.clone());
                        }
                    }
                }
                "$unset" => {
                    if let Some(obj) = value.as_object() {
                        for k in obj.keys() {
                            doc.remove(k);
                        }
                    }
                }
                "$inc" => {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            if let Some(inc_val) = v.as_i64() {
                                let current = doc
                                    .get(k)
                                    .and_then(|val| val.as_i64())
                                    .unwrap_or(0);
                                doc.insert(k.clone(), Value::Number((current + inc_val).into()));
                            }
                        }
                    }
                }
                "$push" => {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            let mut arr = doc
                                .get(k)
                                .and_then(|val| val.as_array().map(|a| a.clone()))
                                .unwrap_or_default();
                            arr.push(v.clone());
                            doc.insert(k.clone(), Value::Array(arr));
                        }
                    }
                }
                "$pull" => {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            if let Some(arr) = doc.get(k).and_then(|val| val.as_array()) {
                                let filtered: Vec<Value> =
                                    arr.iter().filter(|item| *item != v).cloned().collect();
                                doc.insert(k.clone(), Value::Array(filtered));
                            }
                        }
                    }
                }
                "$addToSet" => {
                    if let Some(obj) = value.as_object() {
                        for (k, v) in obj {
                            let mut arr = doc
                                .get(k)
                                .and_then(|val| val.as_array().map(|a| a.clone()))
                                .unwrap_or_default();
                            if !arr.contains(v) {
                                arr.push(v.clone());
                            }
                            doc.insert(k.clone(), Value::Array(arr));
                        }
                    }
                }
                "$rename" => {
                    if let Some(obj) = value.as_object() {
                        for (old_name, new_name_val) in obj {
                            if let Some(new_name) = new_name_val.as_str() {
                                if let Some(val) = doc.remove(old_name) {
                                    doc.insert(new_name.to_string(), val);
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Unknown operator, ignore
                }
            }
        }
    }

    // If no operators found, do a replacement
    if !uses_operators {
        // Preserve _id
        let id = doc.remove("_id");
        doc.clear();
        for (key, value) in update {
            doc.insert(key.clone(), value.clone());
        }
        if let Some(id_val) = id {
            doc.insert("_id".to_string(), id_val);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("name".to_string(), Value::String("old".to_string()));

        let mut update_map = serde_json::Map::new();
        update_map.insert("name".to_string(), Value::String("new".to_string()));
        let mut update = Document::new();
        update.insert("$set".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        assert_eq!(
            doc.get("name"),
            Some(&Value::String("new".to_string()))
        );
    }

    #[test]
    fn test_unset_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("field".to_string(), Value::String("value".to_string()));

        let mut update_map = serde_json::Map::new();
        update_map.insert("field".to_string(), Value::Number(1.into()));
        let mut update = Document::new();
        update.insert("$unset".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        assert!(!doc.contains_key("field"));
    }

    #[test]
    fn test_inc_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("counter".to_string(), Value::Number(5.into()));

        let mut update_map = serde_json::Map::new();
        update_map.insert("counter".to_string(), Value::Number(3.into()));
        let mut update = Document::new();
        update.insert("$inc".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        assert_eq!(
            doc.get("counter"),
            Some(&Value::Number(8.into()))
        );
    }

    #[test]
    fn test_push_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert(
            "tags".to_string(),
            Value::Array(vec![Value::String("a".to_string())]),
        );

        let mut update_map = serde_json::Map::new();
        update_map.insert("tags".to_string(), Value::String("b".to_string()));
        let mut update = Document::new();
        update.insert("$push".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        let arr = doc.get("tags").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_pull_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ]),
        );

        let mut update_map = serde_json::Map::new();
        update_map.insert("tags".to_string(), Value::String("a".to_string()));
        let mut update = Document::new();
        update.insert("$pull".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        let arr = doc.get("tags").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1);
    }

    #[test]
    fn test_add_to_set_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert(
            "tags".to_string(),
            Value::Array(vec![Value::String("a".to_string())]),
        );

        let mut update_map = serde_json::Map::new();
        update_map.insert("tags".to_string(), Value::String("a".to_string()));
        let mut update = Document::new();
        update.insert("$addToSet".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        let arr = doc.get("tags").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1); // Should not duplicate
    }

    #[test]
    fn test_rename_operator() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("old_name".to_string(), Value::String("value".to_string()));

        let mut update_map = serde_json::Map::new();
        update_map.insert("old_name".to_string(), Value::String("new_name".to_string()));
        let mut update = Document::new();
        update.insert("$rename".to_string(), Value::Object(update_map));

        apply_update(&mut doc, &update).unwrap();
        assert!(!doc.contains_key("old_name"));
        assert_eq!(
            doc.get("new_name"),
            Some(&Value::String("value".to_string()))
        );
    }

    #[test]
    fn test_replacement_update() {
        let mut doc = Document::new();
        doc.insert("_id".to_string(), Value::String("1".to_string()));
        doc.insert("old_field".to_string(), Value::String("old".to_string()));

        let mut update = Document::new();
        update.insert("new_field".to_string(), Value::String("new".to_string()));

        apply_update(&mut doc, &update).unwrap();
        assert!(!doc.contains_key("old_field"));
        assert_eq!(
            doc.get("new_field"),
            Some(&Value::String("new".to_string()))
        );
        assert_eq!(doc.get("_id"), Some(&Value::String("1".to_string())));
    }
}
