//! Minimal BSON encode/decode for MongoDB wire protocol.
//! Supports: double, string, embedded doc, array, binary, ObjectId, bool,
//! UTC datetime, null, regex, int32, timestamp, int64.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum BsonError {
    #[error("invalid bson type: {0}")]
    InvalidType(u8),
    #[error("decode error: {0}")]
    DecodeError(String),
    #[error("encode error: {0}")]
    EncodeError(String),
}

pub type Document = BTreeMap<String, Value>;

// BSON type codes
const TYPE_DOUBLE: u8 = 0x01;
const TYPE_STRING: u8 = 0x02;
const TYPE_DOCUMENT: u8 = 0x03;
const TYPE_ARRAY: u8 = 0x04;
const TYPE_BINARY: u8 = 0x05;
const TYPE_OBJECT_ID: u8 = 0x07;
const TYPE_BOOL: u8 = 0x08;
const TYPE_UTC_DATETIME: u8 = 0x09;
const TYPE_NULL: u8 = 0x0A;
const TYPE_REGEX: u8 = 0x0B;
const TYPE_INT32: u8 = 0x10;
const TYPE_TIMESTAMP: u8 = 0x11;
const TYPE_INT64: u8 = 0x12;

pub fn encode_doc(doc: &Document) -> Result<Vec<u8>, BsonError> {
    let mut buf = Vec::new();

    // Reserve space for length
    buf.extend_from_slice(&[0, 0, 0, 0]);

    for (key, value) in doc {
        encode_element(key, value, &mut buf)?;
    }

    // Document terminator
    buf.push(0x00);

    // Write document length
    let len = buf.len() as u32;
    buf[0..4].copy_from_slice(&len.to_le_bytes());

    Ok(buf)
}

pub fn decode_doc(bytes: &[u8]) -> Result<Document, BsonError> {
    if bytes.len() < 5 {
        return Err(BsonError::DecodeError("document too short".to_string()));
    }

    let mut doc = Document::new();
    let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;

    if bytes.len() < len {
        return Err(BsonError::DecodeError("incomplete document".to_string()));
    }

    let mut pos = 4;
    while pos < len - 1 {
        let (key, value, consumed) = decode_element(&bytes[pos..], len - pos)?;
        doc.insert(key, value);
        pos += consumed;
    }

    Ok(doc)
}

fn encode_element(key: &str, value: &Value, buf: &mut Vec<u8>) -> Result<(), BsonError> {
    match value {
        Value::Null => {
            buf.push(TYPE_NULL);
            write_cstring(buf, key)?;
        }
        Value::Bool(b) => {
            buf.push(TYPE_BOOL);
            write_cstring(buf, key)?;
            buf.push(if *b { 1 } else { 0 });
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    buf.push(TYPE_INT32);
                    write_cstring(buf, key)?;
                    buf.extend_from_slice(&(i as i32).to_le_bytes());
                } else {
                    buf.push(TYPE_INT64);
                    write_cstring(buf, key)?;
                    buf.extend_from_slice(&i.to_le_bytes());
                }
            } else if let Some(f) = n.as_f64() {
                buf.push(TYPE_DOUBLE);
                write_cstring(buf, key)?;
                buf.extend_from_slice(&f.to_le_bytes());
            }
        }
        Value::String(s) => {
            // Check if it's an ObjectId string (24 hex chars)
            if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                // Encode as ObjectId
                buf.push(TYPE_OBJECT_ID);
                write_cstring(buf, key)?;
                let bytes = hex::decode(s)
                    .map_err(|e| BsonError::EncodeError(format!("invalid objectid: {}", e)))?;
                if bytes.len() == 12 {
                    buf.extend_from_slice(&bytes);
                } else {
                    // Fallback to string
                    buf.clear();
                    buf.push(TYPE_STRING);
                    write_cstring(buf, key)?;
                    write_string(buf, s)?;
                }
            } else {
                buf.push(TYPE_STRING);
                write_cstring(buf, key)?;
                write_string(buf, s)?;
            }
        }
        Value::Array(arr) => {
            buf.push(TYPE_ARRAY);
            write_cstring(buf, key)?;
            let mut arr_doc = Document::new();
            for (i, item) in arr.iter().enumerate() {
                arr_doc.insert(i.to_string(), item.clone());
            }
            let arr_bytes = encode_doc(&arr_doc)?;
            buf.extend_from_slice(&arr_bytes);
        }
        Value::Object(obj) => {
            buf.push(TYPE_DOCUMENT);
            write_cstring(buf, key)?;
            let mut inner_doc = Document::new();
            for (k, v) in obj {
                inner_doc.insert(k.clone(), v.clone());
            }
            let doc_bytes = encode_doc(&inner_doc)?;
            buf.extend_from_slice(&doc_bytes);
        }
    }
    Ok(())
}

fn decode_element(
    bytes: &[u8],
    _remaining: usize,
) -> Result<(String, Value, usize), BsonError> {
    if bytes.is_empty() {
        return Err(BsonError::DecodeError("unexpected eof".to_string()));
    }

    let type_code = bytes[0];
    let (key, key_len) = read_cstring(&bytes[1..])?;
    let mut pos = 1 + key_len;

    let value = match type_code {
        TYPE_NULL => Value::Null,
        TYPE_BOOL => {
            let b = bytes[pos] != 0;
            pos += 1;
            Value::Bool(b)
        }
        TYPE_INT32 => {
            if pos + 4 > bytes.len() {
                return Err(BsonError::DecodeError("truncated int32".to_string()));
            }
            let val = i32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
            pos += 4;
            Value::Number(val.into())
        }
        TYPE_INT64 => {
            if pos + 8 > bytes.len() {
                return Err(BsonError::DecodeError("truncated int64".to_string()));
            }
            let val = i64::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
                bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7],
            ]);
            pos += 8;
            Value::Number(val.into())
        }
        TYPE_DOUBLE => {
            if pos + 8 > bytes.len() {
                return Err(BsonError::DecodeError("truncated double".to_string()));
            }
            let val = f64::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
                bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7],
            ]);
            pos += 8;
            Value::Number(
                serde_json::Number::from_f64(val)
                    .unwrap_or_else(|| serde_json::Number::from(0))
            )
        }
        TYPE_STRING => {
            let (s, new_pos) = read_string(&bytes[pos..])?;
            pos += new_pos;
            Value::String(s)
        }
        TYPE_DOCUMENT => {
            if pos + 4 > bytes.len() {
                return Err(BsonError::DecodeError("truncated document".to_string()));
            }
            let doc_len = u32::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
            ]) as usize;
            if pos + doc_len > bytes.len() {
                return Err(BsonError::DecodeError("incomplete document".to_string()));
            }
            let doc = decode_doc(&bytes[pos..pos + doc_len])?;
            pos += doc_len;
            Value::Object(doc.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        TYPE_ARRAY => {
            if pos + 4 > bytes.len() {
                return Err(BsonError::DecodeError("truncated array".to_string()));
            }
            let arr_len = u32::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
            ]) as usize;
            if pos + arr_len > bytes.len() {
                return Err(BsonError::DecodeError("incomplete array".to_string()));
            }
            let arr_doc = decode_doc(&bytes[pos..pos + arr_len])?;
            let mut arr = Vec::new();
            for i in 0.. {
                match arr_doc.get(&i.to_string()) {
                    Some(v) => arr.push(v.clone()),
                    None => break,
                }
            }
            pos += arr_len;
            Value::Array(arr)
        }
        TYPE_OBJECT_ID => {
            if pos + 12 > bytes.len() {
                return Err(BsonError::DecodeError("truncated objectid".to_string()));
            }
            let oid = &bytes[pos..pos + 12];
            pos += 12;
            Value::String(hex::encode(oid))
        }
        TYPE_UTC_DATETIME => {
            if pos + 8 > bytes.len() {
                return Err(BsonError::DecodeError("truncated datetime".to_string()));
            }
            let ms = i64::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
                bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7],
            ]);
            pos += 8;
            if let Some(dt) = DateTime::<Utc>::from_timestamp_millis(ms) {
                Value::String(dt.to_rfc3339())
            } else {
                Value::Null
            }
        }
        TYPE_BINARY => {
            if pos + 4 > bytes.len() {
                return Err(BsonError::DecodeError("truncated binary".to_string()));
            }
            let len = u32::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
            ]) as usize;
            pos += 4;
            if pos + 1 > bytes.len() {
                return Err(BsonError::DecodeError("truncated binary subtype".to_string()));
            }
            let _subtype = bytes[pos];
            pos += 1;
            if pos + len > bytes.len() {
                return Err(BsonError::DecodeError("incomplete binary data".to_string()));
            }
            let data = bytes[pos..pos + len].to_vec();
            pos += len;
            Value::String(format!("Binary({})", hex::encode(&data)))
        }
        TYPE_REGEX => {
            let (pattern, new_pos) = read_cstring(&bytes[pos..])?;
            let (flags, new_pos2) = read_cstring(&bytes[pos + new_pos + 1..])?;
            pos += new_pos + 1 + new_pos2 + 1;
            Value::Object(
                vec![
                    ("pattern".to_string(), Value::String(pattern)),
                    ("flags".to_string(), Value::String(flags)),
                ]
                .into_iter()
                .collect(),
            )
        }
        TYPE_TIMESTAMP => {
            if pos + 8 > bytes.len() {
                return Err(BsonError::DecodeError("truncated timestamp".to_string()));
            }
            let val = u64::from_le_bytes([
                bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3],
                bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7],
            ]);
            pos += 8;
            Value::Number(val.into())
        }
        _ => {
            tracing::warn!(target: "cave_docdb::bson", "unknown bson type: {}", type_code);
            Value::Null
        }
    };

    Ok((key, value, pos))
}

fn write_cstring(buf: &mut Vec<u8>, s: &str) -> Result<(), BsonError> {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
    Ok(())
}

fn read_cstring(bytes: &[u8]) -> Result<(String, usize), BsonError> {
    for (i, &b) in bytes.iter().enumerate() {
        if b == 0 {
            let s = String::from_utf8(bytes[..i].to_vec())
                .map_err(|e| BsonError::DecodeError(format!("invalid utf8: {}", e)))?;
            return Ok((s, i));
        }
    }
    Err(BsonError::DecodeError("unterminated cstring".to_string()))
}

fn write_string(buf: &mut Vec<u8>, s: &str) -> Result<(), BsonError> {
    let bytes = s.as_bytes();
    let len = (bytes.len() + 1) as u32;
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(bytes);
    buf.push(0);
    Ok(())
}

fn read_string(bytes: &[u8]) -> Result<(String, usize), BsonError> {
    if bytes.len() < 4 {
        return Err(BsonError::DecodeError("string too short".to_string()));
    }
    let len = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if bytes.len() < 4 + len {
        return Err(BsonError::DecodeError("incomplete string".to_string()));
    }
    let s = String::from_utf8(bytes[4..4 + len - 1].to_vec())
        .map_err(|e| BsonError::DecodeError(format!("invalid utf8: {}", e)))?;
    Ok((s, 4 + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bson_encode_null() {
        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::Null);
        let encoded = encode_doc(&doc).unwrap();
        assert!(encoded.len() >= 5);
    }

    #[test]
    fn test_bson_encode_bool() {
        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::Bool(true));
        let encoded = encode_doc(&doc).unwrap();
        assert!(encoded.len() >= 5);
    }

    #[test]
    fn test_bson_encode_number() {
        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::Number(42.into()));
        let encoded = encode_doc(&doc).unwrap();
        assert!(encoded.len() >= 5);
    }

    #[test]
    fn test_bson_encode_string() {
        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::String("test".to_string()));
        let encoded = encode_doc(&doc).unwrap();
        assert!(encoded.len() >= 5);
    }

    #[test]
    fn test_bson_encode_array() {
        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::Array(vec![Value::Number(1.into())]));
        let encoded = encode_doc(&doc).unwrap();
        assert!(encoded.len() >= 5);
    }

    #[test]
    fn test_bson_encode_object() {
        let mut doc = Document::new();
        doc.insert("field".to_string(), Value::Object(serde_json::Map::new()));
        let encoded = encode_doc(&doc).unwrap();
        assert!(encoded.len() >= 5);
    }
}
