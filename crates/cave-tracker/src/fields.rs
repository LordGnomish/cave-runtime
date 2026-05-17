// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{CustomFieldDef, CustomFieldType};
use uuid::Uuid;

pub fn create_field(name: &str, field_type: CustomFieldType, description: &str, required: bool) -> CustomFieldDef {
    CustomFieldDef {
        id: Uuid::new_v4(),
        name: name.to_string(),
        field_type,
        description: description.to_string(),
        required,
        options: vec![],
        default_value: None,
    }
}

pub fn validate_field_value(field: &CustomFieldDef, value: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();
    match &field.field_type {
        CustomFieldType::Number => {
            if !value.is_number() { errors.push(format!("Field '{}': expected number", field.name)); }
        }
        CustomFieldType::Select => {
            if let Some(s) = value.as_str() {
                if !field.options.is_empty() && !field.options.contains(&s.to_string()) {
                    errors.push(format!("Field '{}': '{}' not in allowed options", field.name, s));
                }
            } else {
                errors.push(format!("Field '{}': expected string", field.name));
            }
        }
        CustomFieldType::Checkbox => {
            if !value.is_boolean() { errors.push(format!("Field '{}': expected boolean", field.name)); }
        }
        _ => {} // Text, Date, User, Labels, MultiSelect — accept any for now
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_field() {
        let f = create_field("Story Points", CustomFieldType::Number, "Estimated complexity", false);
        assert_eq!(f.name, "Story Points");
    }

    #[test]
    fn test_validate_number_field() {
        let f = create_field("Points", CustomFieldType::Number, "", false);
        assert!(validate_field_value(&f, &serde_json::json!(5)).is_empty());
        assert!(!validate_field_value(&f, &serde_json::json!("not a number")).is_empty());
    }

    #[test]
    fn test_validate_select_field() {
        let mut f = create_field("Priority", CustomFieldType::Select, "", false);
        f.options = vec!["P1".to_string(), "P2".to_string()];
        assert!(validate_field_value(&f, &serde_json::json!("P1")).is_empty());
        assert!(!validate_field_value(&f, &serde_json::json!("P9")).is_empty());
    }
}
