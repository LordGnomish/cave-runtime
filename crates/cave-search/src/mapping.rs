//! Index field mappings — types + analyzers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Text,
    Keyword,
    Long,
    Integer,
    Short,
    Byte,
    Double,
    Float,
    Date,
    Boolean,
    Object,
    Nested,
    Ip,
    GeoPoint,
}

impl FieldType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Keyword => "keyword",
            Self::Long => "long",
            Self::Integer => "integer",
            Self::Short => "short",
            Self::Byte => "byte",
            Self::Double => "double",
            Self::Float => "float",
            Self::Date => "date",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Nested => "nested",
            Self::Ip => "ip",
            Self::GeoPoint => "geo_point",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => Self::Text,
            "keyword" => Self::Keyword,
            "long" => Self::Long,
            "integer" => Self::Integer,
            "short" => Self::Short,
            "byte" => Self::Byte,
            "double" => Self::Double,
            "float" => Self::Float,
            "date" => Self::Date,
            "boolean" => Self::Boolean,
            "object" => Self::Object,
            "nested" => Self::Nested,
            "ip" => Self::Ip,
            "geo_point" => Self::GeoPoint,
            _ => return None,
        })
    }

    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::Long | Self::Integer | Self::Short | Self::Byte | Self::Double | Self::Float
        )
    }

    pub fn is_searchable_token(self) -> bool {
        matches!(self, Self::Keyword | Self::Text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMapping {
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub index: Option<bool>,
    #[serde(default)]
    pub doc_values: Option<bool>,
    #[serde(default)]
    pub fields: BTreeMap<String, Box<FieldMapping>>,
}

impl FieldMapping {
    pub fn new(t: FieldType) -> Self {
        Self {
            field_type: t,
            index: None,
            doc_values: None,
            fields: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexMapping {
    pub properties: BTreeMap<String, FieldMapping>,
}

impl IndexMapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_field(&mut self, name: &str, ty: FieldType) {
        self.properties.insert(name.into(), FieldMapping::new(ty));
    }

    pub fn field_type(&self, name: &str) -> Option<FieldType> {
        self.properties.get(name).map(|f| f.field_type)
    }

    pub fn field_count(&self) -> usize {
        self.properties.len()
    }

    pub fn validate_document(&self, doc: &serde_json::Value) -> Result<(), String> {
        let obj = doc.as_object().ok_or_else(|| "document must be object".to_string())?;
        for (k, v) in obj {
            if let Some(fm) = self.properties.get(k) {
                check_value(k, v, fm.field_type)?;
            }
        }
        Ok(())
    }
}

fn check_value(field: &str, value: &serde_json::Value, ty: FieldType) -> Result<(), String> {
    match ty {
        FieldType::Text | FieldType::Keyword | FieldType::Ip => {
            if !value.is_string() && !value.is_null() {
                return Err(format!("{} expected string, got {:?}", field, value));
            }
        }
        FieldType::Long | FieldType::Integer | FieldType::Short | FieldType::Byte => {
            if !value.is_i64() && !value.is_u64() && !value.is_null() {
                return Err(format!("{} expected integer, got {:?}", field, value));
            }
        }
        FieldType::Double | FieldType::Float => {
            if !value.is_number() && !value.is_null() {
                return Err(format!("{} expected number, got {:?}", field, value));
            }
        }
        FieldType::Boolean => {
            if !value.is_boolean() && !value.is_null() {
                return Err(format!("{} expected boolean, got {:?}", field, value));
            }
        }
        FieldType::Date => {
            if !value.is_string() && !value.is_i64() && !value.is_u64() && !value.is_null() {
                return Err(format!("{} expected date string or epoch, got {:?}", field, value));
            }
        }
        FieldType::Object | FieldType::Nested => {
            if !value.is_object() && !value.is_array() && !value.is_null() {
                return Err(format!("{} expected object/nested, got {:?}", field, value));
            }
        }
        FieldType::GeoPoint => {
            // Accept various forms; just ensure it's not a primitive.
            if value.is_null() {
                return Ok(());
            }
            if !value.is_object() && !value.is_string() && !value.is_array() {
                return Err(format!("{} expected geo_point form", field));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_round_trip_for_all_types() {
        for ty in [
            FieldType::Text,
            FieldType::Keyword,
            FieldType::Long,
            FieldType::Integer,
            FieldType::Short,
            FieldType::Byte,
            FieldType::Double,
            FieldType::Float,
            FieldType::Date,
            FieldType::Boolean,
            FieldType::Object,
            FieldType::Nested,
            FieldType::Ip,
            FieldType::GeoPoint,
        ] {
            assert_eq!(FieldType::parse(ty.as_str()), Some(ty));
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(FieldType::parse("xyz"), None);
    }

    #[test]
    fn is_numeric_true_for_number_types() {
        assert!(FieldType::Long.is_numeric());
        assert!(FieldType::Float.is_numeric());
        assert!(!FieldType::Text.is_numeric());
    }

    #[test]
    fn is_searchable_token_true_for_keyword_text() {
        assert!(FieldType::Keyword.is_searchable_token());
        assert!(FieldType::Text.is_searchable_token());
        assert!(!FieldType::Long.is_searchable_token());
    }

    #[test]
    fn add_field_then_lookup() {
        let mut m = IndexMapping::new();
        m.add_field("user_id", FieldType::Long);
        m.add_field("name", FieldType::Keyword);
        assert_eq!(m.field_type("user_id"), Some(FieldType::Long));
        assert_eq!(m.field_count(), 2);
    }

    #[test]
    fn validate_document_accepts_matching_types() {
        let mut m = IndexMapping::new();
        m.add_field("id", FieldType::Long);
        m.add_field("name", FieldType::Keyword);
        let doc = json!({"id": 42, "name": "alice"});
        assert!(m.validate_document(&doc).is_ok());
    }

    #[test]
    fn validate_document_rejects_type_mismatch() {
        let mut m = IndexMapping::new();
        m.add_field("id", FieldType::Long);
        let doc = json!({"id": "not a number"});
        assert!(m.validate_document(&doc).is_err());
    }

    #[test]
    fn validate_document_ignores_unmapped_fields() {
        let m = IndexMapping::new();
        let doc = json!({"anything": "goes"});
        assert!(m.validate_document(&doc).is_ok());
    }

    #[test]
    fn validate_document_accepts_null_for_any_type() {
        let mut m = IndexMapping::new();
        m.add_field("opt", FieldType::Long);
        let doc = json!({"opt": null});
        assert!(m.validate_document(&doc).is_ok());
    }

    #[test]
    fn field_mapping_serialize_deserialize() {
        let fm = FieldMapping::new(FieldType::Text);
        let s = serde_json::to_string(&fm).unwrap();
        assert!(s.contains("\"type\":\"text\""));
        let back: FieldMapping = serde_json::from_str(&s).unwrap();
        assert_eq!(back.field_type, FieldType::Text);
    }

    #[test]
    fn index_mapping_serialize() {
        let mut m = IndexMapping::new();
        m.add_field("a", FieldType::Long);
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"properties\""));
        assert!(s.contains("\"a\""));
    }

    #[test]
    fn boolean_field_validates_bool_only() {
        let mut m = IndexMapping::new();
        m.add_field("active", FieldType::Boolean);
        assert!(m.validate_document(&json!({"active": true})).is_ok());
        assert!(m.validate_document(&json!({"active": "yes"})).is_err());
    }

    #[test]
    fn date_field_accepts_string_or_epoch() {
        let mut m = IndexMapping::new();
        m.add_field("ts", FieldType::Date);
        assert!(m.validate_document(&json!({"ts": "2025-01-01"})).is_ok());
        assert!(m.validate_document(&json!({"ts": 1735689600})).is_ok());
        assert!(m.validate_document(&json!({"ts": true})).is_err());
    }
}
