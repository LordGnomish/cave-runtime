// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 2 (RED→GREEN): JSON Schema parameter validation.

use cave_tools::schema::validate;
use cave_tools::tool::{FnTool, ToolRegistry, ToolResult};
use serde_json::json;

#[test]
fn accepts_well_formed_object() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer", "minimum": 0}
        },
        "required": ["name"]
    });
    assert!(validate(&schema, &json!({"name": "ada", "age": 36})).is_ok());
}

#[test]
fn missing_required_field_is_rejected() {
    let schema = json!({
        "type": "object",
        "properties": {"name": {"type": "string"}},
        "required": ["name"]
    });
    let errs = validate(&schema, &json!({})).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("name")), "errs: {errs:?}");
}

#[test]
fn wrong_type_is_rejected() {
    let schema = json!({"type": "object", "properties": {"age": {"type": "integer"}}});
    let errs = validate(&schema, &json!({"age": "old"})).unwrap_err();
    assert!(errs.iter().any(|e| e.contains("age")));
}

#[test]
fn integer_vs_number_distinction() {
    let schema = json!({"type": "integer"});
    assert!(validate(&schema, &json!(5)).is_ok());
    assert!(validate(&schema, &json!(5.5)).is_err());
    assert!(validate(&json!({"type": "number"}), &json!(5.5)).is_ok());
}

#[test]
fn numeric_bounds_enforced() {
    let schema = json!({"type": "number", "minimum": 0, "maximum": 10});
    assert!(validate(&schema, &json!(5)).is_ok());
    assert!(validate(&schema, &json!(-1)).is_err());
    assert!(validate(&schema, &json!(11)).is_err());
    let excl = json!({"type": "number", "exclusiveMinimum": 0});
    assert!(validate(&excl, &json!(0)).is_err());
    assert!(validate(&excl, &json!(0.1)).is_ok());
}

#[test]
fn string_length_and_pattern() {
    let schema = json!({"type": "string", "minLength": 2, "maxLength": 4, "pattern": "^[a-z]+$"});
    assert!(validate(&schema, &json!("abc")).is_ok());
    assert!(validate(&schema, &json!("a")).is_err()); // too short
    assert!(validate(&schema, &json!("abcde")).is_err()); // too long
    assert!(validate(&schema, &json!("AB3")).is_err()); // pattern + len
}

#[test]
fn enum_constraint() {
    let schema = json!({"enum": ["red", "green", "blue"]});
    assert!(validate(&schema, &json!("green")).is_ok());
    assert!(validate(&schema, &json!("purple")).is_err());
}

#[test]
fn array_items_and_bounds() {
    let schema = json!({
        "type": "array",
        "items": {"type": "integer"},
        "minItems": 1,
        "maxItems": 3
    });
    assert!(validate(&schema, &json!([1, 2])).is_ok());
    assert!(validate(&schema, &json!([])).is_err()); // minItems
    assert!(validate(&schema, &json!([1, 2, 3, 4])).is_err()); // maxItems
    assert!(validate(&schema, &json!([1, "x"])).is_err()); // item type
}

#[test]
fn additional_properties_false_rejects_extras() {
    let schema = json!({
        "type": "object",
        "properties": {"a": {"type": "string"}},
        "additionalProperties": false
    });
    assert!(validate(&schema, &json!({"a": "x"})).is_ok());
    assert!(validate(&schema, &json!({"a": "x", "b": 1})).is_err());
}

#[test]
fn nested_object_validation() {
    let schema = json!({
        "type": "object",
        "properties": {
            "addr": {
                "type": "object",
                "properties": {"zip": {"type": "string"}},
                "required": ["zip"]
            }
        },
        "required": ["addr"]
    });
    assert!(validate(&schema, &json!({"addr": {"zip": "94110"}})).is_ok());
    assert!(validate(&schema, &json!({"addr": {}})).is_err());
}

#[test]
fn registry_invoke_validated_blocks_bad_args() {
    let mut reg = ToolRegistry::new();
    reg.register(FnTool::new(
        "greet",
        "greet someone",
        json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }),
        |args| Ok(ToolResult::text(format!("hi {}", args["name"].as_str().unwrap()))),
    ));
    // Valid call runs.
    assert_eq!(
        reg.invoke_validated("greet", &json!({"name": "x"}))
            .unwrap()
            .text_output(),
        "hi x"
    );
    // Missing required arg is rejected *before* the handler runs.
    let err = reg.invoke_validated("greet", &json!({})).unwrap_err();
    assert_eq!(err.code(), "invalid_arguments");
}
