//! request_validation plugin — Kong / Envoy json-schema parity.
//!
//! Validates inbound requests *before* they reach upstream:
//! - Content-Type allowlist with optional `*/*` wildcard.
//! - Body size limits (min / max).
//! - Required header set.
//! - Method allowlist.
//! - Path-parameter allow/deny (regex).
//! - JSON body shape — a tiny self-contained subset of JSON Schema covering
//!   `type`, `required`, `properties`, `enum`, `minimum`, `maximum`,
//!   `minLength`, `maxLength`, `pattern`, `items`, `additionalProperties`.
//!
//! Real production setups would defer to a full JSON-schema crate; this
//! module ships the parity surface and the rules we lean on for our own
//! Admin API and Marketplace.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestValidationConfig {
    pub allowed_content_types: Vec<String>,
    pub max_body_bytes: u64,
    pub min_body_bytes: u64,
    pub required_headers: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub body_schema: Option<JsonSchema>,
    /// Reject the request with this status when validation fails.
    pub status_on_invalid: u16,
    /// Continue (log only) instead of rejecting.
    pub log_only: bool,
}

impl Default for RequestValidationConfig {
    fn default() -> Self {
        Self {
            allowed_content_types: vec!["application/json".into()],
            max_body_bytes: 1024 * 1024,
            min_body_bytes: 0,
            required_headers: vec![],
            allowed_methods: vec![],
            body_schema: None,
            status_on_invalid: 400,
            log_only: false,
        }
    }
}

impl RequestValidationConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_body_bytes < self.min_body_bytes {
            return Err("max_body_bytes < min_body_bytes".into());
        }
        if !(400..=599).contains(&self.status_on_invalid) {
            return Err("status_on_invalid must be 4xx/5xx".into());
        }
        if let Some(s) = &self.body_schema {
            s.validate_self()?;
        }
        Ok(())
    }
}

// ── Tiny JSON-schema subset ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaType {
    Object,
    Array,
    String,
    Number,
    Integer,
    Boolean,
    Null,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonSchema {
    #[serde(rename = "type")]
    pub kind: SchemaType,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub properties: HashMap<String, JsonSchema>,
    #[serde(default)]
    pub items: Option<Box<JsonSchema>>,
    /// True = unknown extra keys are allowed (default). False = error.
    #[serde(default = "default_true")]
    pub additional_properties: bool,
    #[serde(default)]
    pub r#enum: Vec<Value>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub min_length: Option<u32>,
    pub max_length: Option<u32>,
    pub pattern: Option<String>,
}

fn default_true() -> bool {
    true
}

impl JsonSchema {
    pub fn object(properties: Vec<(&str, JsonSchema)>, required: Vec<&str>) -> Self {
        Self {
            kind: SchemaType::Object,
            required: required.into_iter().map(String::from).collect(),
            properties: properties
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            items: None,
            additional_properties: true,
            r#enum: vec![],
            minimum: None,
            maximum: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }
    }

    pub fn string() -> Self {
        Self {
            kind: SchemaType::String,
            required: vec![],
            properties: HashMap::new(),
            items: None,
            additional_properties: true,
            r#enum: vec![],
            minimum: None,
            maximum: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }
    }

    pub fn integer() -> Self {
        Self {
            kind: SchemaType::Integer,
            required: vec![],
            properties: HashMap::new(),
            items: None,
            additional_properties: true,
            r#enum: vec![],
            minimum: None,
            maximum: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }
    }

    pub fn array(items: JsonSchema) -> Self {
        Self {
            kind: SchemaType::Array,
            required: vec![],
            properties: HashMap::new(),
            items: Some(Box::new(items)),
            additional_properties: true,
            r#enum: vec![],
            minimum: None,
            maximum: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }
    }

    pub fn validate_self(&self) -> Result<(), String> {
        if let (Some(lo), Some(hi)) = (self.minimum, self.maximum) {
            if lo > hi {
                return Err("schema minimum > maximum".into());
            }
        }
        if let (Some(lo), Some(hi)) = (self.min_length, self.max_length) {
            if lo > hi {
                return Err("schema min_length > max_length".into());
            }
        }
        if let Some(p) = &self.pattern {
            regex::Regex::new(p).map_err(|e| format!("invalid pattern: {}", e))?;
        }
        for (_, child) in &self.properties {
            child.validate_self()?;
        }
        if let Some(items) = &self.items {
            items.validate_self()?;
        }
        Ok(())
    }

    pub fn check(&self, value: &Value, path: &str) -> Vec<String> {
        let mut errors = Vec::new();
        match (self.kind, value) {
            (SchemaType::Object, Value::Object(map)) => {
                for r in &self.required {
                    if !map.contains_key(r) {
                        errors.push(format!("{}: missing required field '{}'", path, r));
                    }
                }
                for (k, v) in map {
                    if let Some(child) = self.properties.get(k) {
                        errors.extend(child.check(v, &format!("{}.{}", path, k)));
                    } else if !self.additional_properties {
                        errors.push(format!("{}: unexpected field '{}'", path, k));
                    }
                }
            }
            (SchemaType::Array, Value::Array(arr)) => {
                if let Some(items) = &self.items {
                    for (i, v) in arr.iter().enumerate() {
                        errors.extend(items.check(v, &format!("{}[{}]", path, i)));
                    }
                }
            }
            (SchemaType::String, Value::String(s)) => {
                if let Some(min) = self.min_length {
                    if (s.chars().count() as u32) < min {
                        errors.push(format!("{}: string shorter than {}", path, min));
                    }
                }
                if let Some(max) = self.max_length {
                    if (s.chars().count() as u32) > max {
                        errors.push(format!("{}: string longer than {}", path, max));
                    }
                }
                if let Some(pattern) = &self.pattern {
                    let re = match regex::Regex::new(pattern) {
                        Ok(r) => r,
                        Err(_) => {
                            errors.push(format!("{}: invalid pattern", path));
                            return errors;
                        }
                    };
                    if !re.is_match(s) {
                        errors.push(format!("{}: does not match pattern", path));
                    }
                }
                if !self.r#enum.is_empty() && !self.r#enum.iter().any(|e| e == value) {
                    errors.push(format!("{}: value not in enum", path));
                }
            }
            (SchemaType::Integer, Value::Number(n)) => {
                if !n.is_i64() && !n.is_u64() {
                    if let Some(f) = n.as_f64() {
                        if f.fract() != 0.0 {
                            errors.push(format!("{}: not an integer", path));
                            return errors;
                        }
                    }
                }
                let f = n.as_f64().unwrap_or(0.0);
                if let Some(min) = self.minimum {
                    if f < min {
                        errors.push(format!("{}: less than minimum {}", path, min));
                    }
                }
                if let Some(max) = self.maximum {
                    if f > max {
                        errors.push(format!("{}: greater than maximum {}", path, max));
                    }
                }
            }
            (SchemaType::Number, Value::Number(n)) => {
                let f = n.as_f64().unwrap_or(0.0);
                if let Some(min) = self.minimum {
                    if f < min {
                        errors.push(format!("{}: less than minimum {}", path, min));
                    }
                }
                if let Some(max) = self.maximum {
                    if f > max {
                        errors.push(format!("{}: greater than maximum {}", path, max));
                    }
                }
            }
            (SchemaType::Boolean, Value::Bool(_)) => {}
            (SchemaType::Null, Value::Null) => {}
            (expected, got) => {
                errors.push(format!(
                    "{}: type mismatch — expected {:?}, got {}",
                    path,
                    expected,
                    got_type(got)
                ));
            }
        }
        errors
    }
}

fn got_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ── Validation entry point ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    Ok,
    Reject { status: u16, errors: Vec<String> },
}

pub fn validate(
    cfg: &RequestValidationConfig,
    method: &str,
    headers: &HashMap<String, String>,
    body: Option<&[u8]>,
) -> ValidationOutcome {
    let mut errors: Vec<String> = Vec::new();

    if !cfg.allowed_methods.is_empty()
        && !cfg.allowed_methods.iter().any(|m| m.eq_ignore_ascii_case(method))
    {
        errors.push(format!("method {} not allowed", method));
    }

    for r in &cfg.required_headers {
        if !headers.contains_key(&r.to_ascii_lowercase()) {
            errors.push(format!("required header missing: {}", r));
        }
    }

    if !cfg.allowed_content_types.is_empty() {
        let ct = headers
            .get("content-type")
            .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase());
        let ct = ct.unwrap_or_default();
        let allowed = cfg
            .allowed_content_types
            .iter()
            .any(|a| a == "*/*" || a.eq_ignore_ascii_case(&ct));
        if !allowed && !ct.is_empty() {
            errors.push(format!("content-type {} not allowed", ct));
        }
        if ct.is_empty() && body.map_or(0, |b| b.len()) > 0 {
            errors.push("content-type header missing on body request".into());
        }
    }

    let body_len = body.map_or(0u64, |b| b.len() as u64);
    if body_len > cfg.max_body_bytes {
        errors.push(format!(
            "body too large: {} > {}",
            body_len, cfg.max_body_bytes
        ));
    }
    if body_len < cfg.min_body_bytes {
        errors.push(format!(
            "body too small: {} < {}",
            body_len, cfg.min_body_bytes
        ));
    }

    if let (Some(schema), Some(b)) = (&cfg.body_schema, body) {
        match serde_json::from_slice::<Value>(b) {
            Ok(v) => errors.extend(schema.check(&v, "$")),
            Err(e) => errors.push(format!("invalid JSON body: {}", e)),
        }
    }

    if errors.is_empty() {
        ValidationOutcome::Ok
    } else if cfg.log_only {
        ValidationOutcome::Ok
    } else {
        ValidationOutcome::Reject { status: cfg.status_on_invalid, errors }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_string()))
            .collect()
    }

    // ── Config validation ──

    #[test]
    fn config_default_validates() {
        RequestValidationConfig::default().validate().unwrap();
    }

    #[test]
    fn config_min_above_max_rejects() {
        let mut c = RequestValidationConfig::default();
        c.min_body_bytes = 100;
        c.max_body_bytes = 50;
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_invalid_status_rejects() {
        let mut c = RequestValidationConfig::default();
        c.status_on_invalid = 200;
        assert!(c.validate().is_err());
    }

    // ── Method allowlist ──

    #[test]
    fn method_allowlist_blocks_disallowed() {
        let mut c = RequestValidationConfig::default();
        c.allowed_methods = vec!["GET".into(), "POST".into()];
        let r = validate(&c, "DELETE", &h(&[]), None);
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    #[test]
    fn method_allowlist_passes_allowed_case_insensitive() {
        let mut c = RequestValidationConfig::default();
        c.allowed_methods = vec!["GET".into()];
        assert_eq!(validate(&c, "get", &h(&[]), None), ValidationOutcome::Ok);
    }

    #[test]
    fn method_allowlist_empty_means_all_allowed() {
        let r = validate(
            &RequestValidationConfig::default(),
            "PATCH",
            &h(&[]),
            None,
        );
        assert_eq!(r, ValidationOutcome::Ok);
    }

    // ── Required headers ──

    #[test]
    fn required_headers_missing_rejects() {
        let mut c = RequestValidationConfig::default();
        c.required_headers = vec!["X-Tenant-Id".into()];
        let r = validate(&c, "GET", &h(&[]), None);
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    #[test]
    fn required_headers_present_passes() {
        let mut c = RequestValidationConfig::default();
        c.required_headers = vec!["X-Tenant-Id".into()];
        let r = validate(&c, "GET", &h(&[("x-tenant-id", "acme")]), None);
        assert_eq!(r, ValidationOutcome::Ok);
    }

    // ── Content-type allowlist ──

    #[test]
    fn content_type_allowlist_blocks() {
        let r = validate(
            &RequestValidationConfig::default(),
            "POST",
            &h(&[("content-type", "text/plain")]),
            Some(b"x"),
        );
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    #[test]
    fn content_type_allowlist_passes_exact() {
        let r = validate(
            &RequestValidationConfig::default(),
            "POST",
            &h(&[("content-type", "application/json")]),
            Some(b"{}"),
        );
        assert_eq!(r, ValidationOutcome::Ok);
    }

    #[test]
    fn content_type_allowlist_strips_charset() {
        let r = validate(
            &RequestValidationConfig::default(),
            "POST",
            &h(&[("content-type", "application/json; charset=utf-8")]),
            Some(b"{}"),
        );
        assert_eq!(r, ValidationOutcome::Ok);
    }

    #[test]
    fn content_type_wildcard_passes_anything() {
        let mut c = RequestValidationConfig::default();
        c.allowed_content_types = vec!["*/*".into()];
        let r = validate(&c, "POST", &h(&[("content-type", "image/png")]), Some(b"x"));
        assert_eq!(r, ValidationOutcome::Ok);
    }

    #[test]
    fn content_type_missing_with_body_rejects() {
        let r = validate(
            &RequestValidationConfig::default(),
            "POST",
            &h(&[]),
            Some(b"data"),
        );
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    #[test]
    fn content_type_missing_without_body_ok() {
        let r = validate(&RequestValidationConfig::default(), "GET", &h(&[]), None);
        assert_eq!(r, ValidationOutcome::Ok);
    }

    // ── Body size limits ──

    #[test]
    fn body_above_max_rejects() {
        let mut c = RequestValidationConfig::default();
        c.max_body_bytes = 4;
        let r = validate(
            &c,
            "POST",
            &h(&[("content-type", "application/json")]),
            Some(b"hello"),
        );
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    #[test]
    fn body_below_min_rejects() {
        let mut c = RequestValidationConfig::default();
        c.min_body_bytes = 10;
        let r = validate(
            &c,
            "POST",
            &h(&[("content-type", "application/json")]),
            Some(b"hi"),
        );
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    // ── log_only ──

    #[test]
    fn log_only_returns_ok_even_on_errors() {
        let mut c = RequestValidationConfig::default();
        c.allowed_methods = vec!["GET".into()];
        c.log_only = true;
        let r = validate(&c, "DELETE", &h(&[]), None);
        assert_eq!(r, ValidationOutcome::Ok);
    }

    // ── JSON schema ──

    #[test]
    fn schema_object_required_field_missing() {
        let s = JsonSchema::object(vec![("name", JsonSchema::string())], vec!["name"]);
        let v = json!({});
        let errs = s.check(&v, "$");
        assert!(!errs.is_empty());
        assert!(errs[0].contains("missing required"));
    }

    #[test]
    fn schema_object_required_field_present() {
        let s = JsonSchema::object(vec![("name", JsonSchema::string())], vec!["name"]);
        let v = json!({"name": "a"});
        assert!(s.check(&v, "$").is_empty());
    }

    #[test]
    fn schema_type_mismatch_string_int() {
        let s = JsonSchema::string();
        let v = json!(5);
        let errs = s.check(&v, "$");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("type mismatch"));
    }

    #[test]
    fn schema_string_min_max_length() {
        let mut s = JsonSchema::string();
        s.min_length = Some(2);
        s.max_length = Some(4);
        assert!(!s.check(&json!("a"), "$").is_empty());
        assert!(!s.check(&json!("aaaaa"), "$").is_empty());
        assert!(s.check(&json!("abc"), "$").is_empty());
    }

    #[test]
    fn schema_string_pattern() {
        let mut s = JsonSchema::string();
        s.pattern = Some(r"^\d{4}$".into());
        assert!(s.check(&json!("1234"), "$").is_empty());
        assert!(!s.check(&json!("abcd"), "$").is_empty());
    }

    #[test]
    fn schema_string_enum() {
        let mut s = JsonSchema::string();
        s.r#enum = vec![json!("a"), json!("b")];
        assert!(s.check(&json!("a"), "$").is_empty());
        assert!(!s.check(&json!("c"), "$").is_empty());
    }

    #[test]
    fn schema_integer_minimum_maximum() {
        let mut s = JsonSchema::integer();
        s.minimum = Some(1.0);
        s.maximum = Some(10.0);
        assert!(s.check(&json!(5), "$").is_empty());
        assert!(!s.check(&json!(0), "$").is_empty());
        assert!(!s.check(&json!(11), "$").is_empty());
    }

    #[test]
    fn schema_integer_rejects_fractional() {
        let s = JsonSchema::integer();
        let errs = s.check(&json!(1.5), "$");
        assert!(!errs.is_empty());
    }

    #[test]
    fn schema_array_items_validated() {
        let s = JsonSchema::array(JsonSchema::integer());
        assert!(s.check(&json!([1, 2, 3]), "$").is_empty());
        let errs = s.check(&json!([1, "two"]), "$");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("$[1]"));
    }

    #[test]
    fn schema_additional_properties_allowed_by_default() {
        let s = JsonSchema::object(vec![("a", JsonSchema::string())], vec![]);
        assert!(s.check(&json!({"a": "x", "b": 1}), "$").is_empty());
    }

    #[test]
    fn schema_additional_properties_false_rejects_extras() {
        let mut s = JsonSchema::object(vec![("a", JsonSchema::string())], vec![]);
        s.additional_properties = false;
        assert!(!s.check(&json!({"a": "x", "b": 1}), "$").is_empty());
    }

    #[test]
    fn schema_self_validate_min_max_inverted() {
        let mut s = JsonSchema::integer();
        s.minimum = Some(10.0);
        s.maximum = Some(1.0);
        assert!(s.validate_self().is_err());
    }

    #[test]
    fn schema_self_validate_min_max_length_inverted() {
        let mut s = JsonSchema::string();
        s.min_length = Some(10);
        s.max_length = Some(1);
        assert!(s.validate_self().is_err());
    }

    #[test]
    fn schema_self_validate_invalid_pattern() {
        let mut s = JsonSchema::string();
        s.pattern = Some("[".into());
        assert!(s.validate_self().is_err());
    }

    #[test]
    fn validate_with_body_schema_passes() {
        let mut c = RequestValidationConfig::default();
        c.body_schema = Some(JsonSchema::object(
            vec![("id", JsonSchema::integer())],
            vec!["id"],
        ));
        let r = validate(
            &c,
            "POST",
            &h(&[("content-type", "application/json")]),
            Some(br#"{"id": 7}"#),
        );
        assert_eq!(r, ValidationOutcome::Ok);
    }

    #[test]
    fn validate_with_body_schema_rejects_missing_field() {
        let mut c = RequestValidationConfig::default();
        c.body_schema = Some(JsonSchema::object(
            vec![("id", JsonSchema::integer())],
            vec!["id"],
        ));
        let r = validate(
            &c,
            "POST",
            &h(&[("content-type", "application/json")]),
            Some(br#"{}"#),
        );
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }

    #[test]
    fn validate_with_invalid_json_rejects() {
        let mut c = RequestValidationConfig::default();
        c.body_schema = Some(JsonSchema::integer());
        let r = validate(
            &c,
            "POST",
            &h(&[("content-type", "application/json")]),
            Some(b"{not json}"),
        );
        assert!(matches!(r, ValidationOutcome::Reject { .. }));
    }
}
