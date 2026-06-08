// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! MongoDB OP_MSG wire protocol (opcode 2013).

use crate::bson::{self, Document};
use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum WireError {
    #[error("bson error: {0}")]
    BsonError(String),
    #[error("wire error: {0}")]
    WireError(String),
}

#[derive(Debug, Clone)]
pub enum Section {
    Body(Document),
    DocumentSequence(String, Vec<Document>),
}

#[derive(Debug, Clone)]
pub struct OpMsg {
    pub flag_bits: u32,
    pub sections: Vec<Section>,
}

impl OpMsg {
    pub fn new(doc: Document) -> Self {
        Self {
            flag_bits: 0,
            sections: vec![Section::Body(doc)],
        }
    }

    pub fn body(&self) -> Option<&Document> {
        for section in &self.sections {
            if let Section::Body(doc) = section {
                return Some(doc);
            }
        }
        None
    }
}

const OPCODE: i32 = 2013;

pub fn encode_op_msg(
    reply: &OpMsg,
    request_id: i32,
    response_to: i32,
) -> Result<Vec<u8>, WireError> {
    let mut payload = Vec::new();

    // Encode flag bits
    payload.extend_from_slice(&reply.flag_bits.to_le_bytes());

    // Encode sections
    for section in &reply.sections {
        match section {
            Section::Body(doc) => {
                payload.push(0x00); // Section kind: body
                let encoded =
                    bson::encode_doc(doc).map_err(|e| WireError::BsonError(e.to_string()))?;
                payload.extend_from_slice(&encoded);
            }
            Section::DocumentSequence(seq_name, docs) => {
                payload.push(0x01); // Document sequence marker
                let mut seq_data = Vec::new();
                seq_data.extend_from_slice(seq_name.as_bytes());
                seq_data.push(0);
                let mut docs_payload = Vec::new();
                for doc in docs {
                    let encoded =
                        bson::encode_doc(doc).map_err(|e| WireError::BsonError(e.to_string()))?;
                    docs_payload.extend_from_slice(&encoded);
                }
                let seq_len = (4 + seq_data.len() + docs_payload.len()) as u32;
                payload.extend_from_slice(&seq_len.to_le_bytes());
                payload.extend_from_slice(&seq_data);
                payload.extend_from_slice(&docs_payload);
            }
        }
    }

    // Build header (16 bytes)
    let message_length = (16 + payload.len()) as u32;
    let mut msg = Vec::new();
    msg.extend_from_slice(&message_length.to_le_bytes());
    msg.extend_from_slice(&request_id.to_le_bytes());
    msg.extend_from_slice(&response_to.to_le_bytes());
    msg.extend_from_slice(&OPCODE.to_le_bytes());

    msg.extend_from_slice(&payload);

    Ok(msg)
}

pub fn decode_op_msg(bytes: &[u8]) -> Result<(i32, OpMsg), WireError> {
    if bytes.len() < 16 {
        return Err(WireError::WireError("header too short".to_string()));
    }

    let message_length = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if bytes.len() < message_length {
        return Err(WireError::WireError("incomplete message".to_string()));
    }

    let request_id = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let _response_to = i32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let opcode = i32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);

    if opcode != OPCODE {
        return Err(WireError::WireError(format!("invalid opcode: {}", opcode)));
    }

    let flag_bits = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);

    let mut sections = Vec::new();
    let mut pos = 20;

    while pos < message_length {
        if pos + 1 > message_length {
            break;
        }

        let section_type = bytes[pos];
        pos += 1;

        if section_type == 0 {
            // Body section: document
            let doc =
                bson::decode_doc(&bytes[pos..]).map_err(|e| WireError::BsonError(e.to_string()))?;
            let encoded =
                bson::encode_doc(&doc).map_err(|e| WireError::BsonError(e.to_string()))?;
            let consumed = encoded.len();
            sections.push(Section::Body(doc));
            pos += consumed;
        } else if section_type == 1 {
            // Document sequence
            if pos + 4 > message_length {
                break;
            }
            let _seq_size =
                u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                    as usize;
            pos += 4;

            let (seq_name, consumed) = read_cstring(&bytes[pos..])?;
            pos += consumed;

            let mut docs = Vec::new();
            while pos < message_length {
                if bytes[pos] == 0 || bytes[pos] == 1 {
                    break;
                }
                let doc = bson::decode_doc(&bytes[pos..])
                    .map_err(|e| WireError::BsonError(e.to_string()))?;
                let encoded =
                    bson::encode_doc(&doc).map_err(|e| WireError::BsonError(e.to_string()))?;
                let consumed = encoded.len();
                docs.push(doc);
                pos += consumed;
            }

            sections.push(Section::DocumentSequence(seq_name, docs));
        } else {
            break;
        }
    }

    Ok((
        request_id,
        OpMsg {
            flag_bits,
            sections,
        },
    ))
}

// ── Legacy op-codes (pre-OP_MSG write protocol, removed in MongoDB 5.1) ──────
// Decoded for driver/tool compatibility. MongoDB 7.0 / FerretDB reject these
// for execution; cave-docdb decodes them into normalised structs so a legacy
// client's bytes can be understood and translated.

/// OP_UPDATE opcode.
pub const OP_UPDATE: i32 = 2001;
/// OP_INSERT opcode.
pub const OP_INSERT: i32 = 2002;
/// OP_QUERY opcode.
pub const OP_QUERY: i32 = 2004;
/// OP_DELETE opcode.
pub const OP_DELETE: i32 = 2006;

/// Decoded legacy `OP_INSERT` (2002).
#[derive(Debug, Clone, PartialEq)]
pub struct OpInsert {
    /// Insert flag bits (bit 0 = ContinueOnError).
    pub flags: i32,
    /// `db.collection` namespace.
    pub collection: String,
    /// Documents to insert.
    pub documents: Vec<Document>,
}

/// Decoded legacy `OP_UPDATE` (2001).
#[derive(Debug, Clone, PartialEq)]
pub struct OpUpdate {
    /// `db.collection` namespace.
    pub collection: String,
    /// Update flag bits (bit 0 = Upsert, bit 1 = MultiUpdate).
    pub flags: i32,
    /// Query selector.
    pub selector: Document,
    /// Update specification.
    pub update: Document,
}

/// Decoded legacy `OP_DELETE` (2006).
#[derive(Debug, Clone, PartialEq)]
pub struct OpDelete {
    /// `db.collection` namespace.
    pub collection: String,
    /// Delete flag bits (bit 0 = SingleRemove).
    pub flags: i32,
    /// Query selector.
    pub selector: Document,
}

/// Decoded legacy `OP_QUERY` (2004).
#[derive(Debug, Clone, PartialEq)]
pub struct OpQuery {
    /// Query flag bits.
    pub flags: i32,
    /// `db.collection` namespace.
    pub collection: String,
    /// Documents to skip.
    pub number_to_skip: i32,
    /// Batch size hint.
    pub number_to_return: i32,
    /// Query document.
    pub query: Document,
}

/// A decoded legacy write/query op.
#[derive(Debug, Clone, PartialEq)]
pub enum LegacyOp {
    /// OP_INSERT (2002).
    Insert(OpInsert),
    /// OP_UPDATE (2001).
    Update(OpUpdate),
    /// OP_DELETE (2006).
    Delete(OpDelete),
    /// OP_QUERY (2004).
    Query(OpQuery),
}

/// Read a `i32` little-endian at `pos`.
fn read_i32(bytes: &[u8], pos: usize) -> Result<i32, WireError> {
    if pos + 4 > bytes.len() {
        return Err(WireError::WireError("truncated i32".to_string()));
    }
    Ok(i32::from_le_bytes([
        bytes[pos],
        bytes[pos + 1],
        bytes[pos + 2],
        bytes[pos + 3],
    ]))
}

/// Read a BSON document at `pos`, returning it and the bytes consumed.
fn read_document(bytes: &[u8], pos: usize) -> Result<(Document, usize), WireError> {
    let doc = bson::decode_doc(&bytes[pos..]).map_err(|e| WireError::BsonError(e.to_string()))?;
    let consumed = bson::encode_doc(&doc)
        .map_err(|e| WireError::BsonError(e.to_string()))?
        .len();
    Ok((doc, consumed))
}

/// Decode a legacy wire message (OP_INSERT/OP_UPDATE/OP_DELETE/OP_QUERY).
///
/// These pre-OP_MSG codes were removed from the server in MongoDB 5.1;
/// cave-docdb decodes them so a legacy driver's bytes are understood and can be
/// translated onto the modern command path.
pub fn decode_legacy(bytes: &[u8]) -> Result<LegacyOp, WireError> {
    if bytes.len() < 16 {
        return Err(WireError::WireError("header too short".to_string()));
    }
    let opcode = read_i32(bytes, 12)?;
    let mut pos = 16;
    match opcode {
        OP_INSERT => {
            let flags = read_i32(bytes, pos)?;
            pos += 4;
            let (collection, consumed) = read_cstring(&bytes[pos..])?;
            pos += consumed;
            let mut documents = Vec::new();
            while pos < bytes.len() {
                let (d, c) = read_document(bytes, pos)?;
                pos += c;
                documents.push(d);
            }
            Ok(LegacyOp::Insert(OpInsert {
                flags,
                collection,
                documents,
            }))
        }
        OP_UPDATE => {
            pos += 4; // reserved ZERO
            let (collection, consumed) = read_cstring(&bytes[pos..])?;
            pos += consumed;
            let flags = read_i32(bytes, pos)?;
            pos += 4;
            let (selector, c1) = read_document(bytes, pos)?;
            pos += c1;
            let (update, _c2) = read_document(bytes, pos)?;
            Ok(LegacyOp::Update(OpUpdate {
                collection,
                flags,
                selector,
                update,
            }))
        }
        OP_DELETE => {
            pos += 4; // reserved ZERO
            let (collection, consumed) = read_cstring(&bytes[pos..])?;
            pos += consumed;
            let flags = read_i32(bytes, pos)?;
            pos += 4;
            let (selector, _c) = read_document(bytes, pos)?;
            Ok(LegacyOp::Delete(OpDelete {
                collection,
                flags,
                selector,
            }))
        }
        OP_QUERY => {
            let flags = read_i32(bytes, pos)?;
            pos += 4;
            let (collection, consumed) = read_cstring(&bytes[pos..])?;
            pos += consumed;
            let number_to_skip = read_i32(bytes, pos)?;
            pos += 4;
            let number_to_return = read_i32(bytes, pos)?;
            pos += 4;
            let (query, _c) = read_document(bytes, pos)?;
            Ok(LegacyOp::Query(OpQuery {
                flags,
                collection,
                number_to_skip,
                number_to_return,
                query,
            }))
        }
        other => Err(WireError::WireError(format!(
            "not a legacy write/query opcode: {}",
            other
        ))),
    }
}

fn read_cstring(bytes: &[u8]) -> Result<(String, usize), WireError> {
    for (i, &b) in bytes.iter().enumerate() {
        if b == 0 {
            let s = String::from_utf8(bytes[..i].to_vec())
                .map_err(|e| WireError::WireError(format!("invalid utf8: {}", e)))?;
            return Ok((s, i + 1));
        }
    }
    Err(WireError::WireError("unterminated cstring".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_encode_msg() {
        let mut doc = Document::new();
        doc.insert("ok".to_string(), Value::Number(1.into()));
        let msg = OpMsg::new(doc);

        let encoded = encode_op_msg(&msg, 1, 0).unwrap();
        assert!(encoded.len() >= 20); // At least header
    }

    #[test]
    fn test_encode_decode_complex_msg() {
        let mut inner = Document::new();
        inner.insert("field".to_string(), Value::String("value".to_string()));
        let mut doc = Document::new();
        doc.insert("command".to_string(), Value::String("find".to_string()));
        doc.insert(
            "nested".to_string(),
            Value::Object(inner.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        );

        let msg = OpMsg::new(doc);
        let encoded = encode_op_msg(&msg, 42, 0).unwrap();
        let (req_id, decoded) = decode_op_msg(&encoded).unwrap();

        assert_eq!(req_id, 42);
        assert!(encoded.len() > 16);
        // Verify body is decoded correctly
        let body = decoded.body().expect("should have body section");
        assert_eq!(
            body.get("command"),
            Some(&Value::String("find".to_string()))
        );
    }

    #[test]
    fn test_encode_decode_roundtrip_body() {
        let mut doc = Document::new();
        doc.insert("ok".to_string(), Value::Number(1.into()));
        doc.insert("n".to_string(), Value::Number(42.into()));
        doc.insert("msg".to_string(), Value::String("hello".to_string()));

        let msg = OpMsg::new(doc);
        let encoded = encode_op_msg(&msg, 7, 0).unwrap();
        let (req_id, decoded) = decode_op_msg(&encoded).unwrap();

        assert_eq!(req_id, 7);
        let body = decoded.body().expect("must have body");
        assert_eq!(body.get("ok"), Some(&Value::Number(1.into())));
        assert_eq!(body.get("n"), Some(&Value::Number(42.into())));
        assert_eq!(body.get("msg"), Some(&Value::String("hello".to_string())));
    }

    #[test]
    fn test_wire_header_format() {
        let mut doc = Document::new();
        doc.insert("test".to_string(), Value::Number(1.into()));
        let msg = OpMsg::new(doc);

        let encoded = encode_op_msg(&msg, 100, 50).unwrap();

        // Check header
        assert!(encoded.len() >= 16);
        let opcode = i32::from_le_bytes([encoded[12], encoded[13], encoded[14], encoded[15]]);
        assert_eq!(opcode, 2013);
    }

    // ── legacy op decoders ───────────────────────────────────────────────

    /// Build a legacy message: 16-byte header + body, with length filled in.
    fn legacy_frame(opcode: i32, body: Vec<u8>) -> Vec<u8> {
        let mut msg = Vec::new();
        let len = (16 + body.len()) as u32;
        msg.extend_from_slice(&len.to_le_bytes());
        msg.extend_from_slice(&1i32.to_le_bytes()); // request_id
        msg.extend_from_slice(&0i32.to_le_bytes()); // response_to
        msg.extend_from_slice(&opcode.to_le_bytes());
        msg.extend_from_slice(&body);
        msg
    }

    fn doc(pairs: &[(&str, Value)]) -> Document {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn cstr(s: &str) -> Vec<u8> {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        b
    }

    #[test]
    fn decode_op_insert() {
        let d1 = bson::encode_doc(&doc(&[("_id", Value::String("1".into()))])).unwrap();
        let d2 = bson::encode_doc(&doc(&[("_id", Value::String("2".into()))])).unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes()); // flags
        body.extend_from_slice(&cstr("testdb.users"));
        body.extend_from_slice(&d1);
        body.extend_from_slice(&d2);

        let op = decode_legacy(&legacy_frame(OP_INSERT, body)).unwrap();
        match op {
            LegacyOp::Insert(ins) => {
                assert_eq!(ins.collection, "testdb.users");
                assert_eq!(ins.documents.len(), 2);
                assert_eq!(ins.documents[1].get("_id"), Some(&Value::String("2".into())));
            }
            other => panic!("expected Insert, got {:?}", other),
        }
    }

    #[test]
    fn decode_op_update() {
        let selector = bson::encode_doc(&doc(&[("_id", Value::String("1".into()))])).unwrap();
        let update = bson::encode_doc(&doc(&[("x", Value::Number(5.into()))])).unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes()); // reserved ZERO
        body.extend_from_slice(&cstr("testdb.c"));
        body.extend_from_slice(&3i32.to_le_bytes()); // flags: upsert|multi
        body.extend_from_slice(&selector);
        body.extend_from_slice(&update);

        match decode_legacy(&legacy_frame(OP_UPDATE, body)).unwrap() {
            LegacyOp::Update(u) => {
                assert_eq!(u.collection, "testdb.c");
                assert_eq!(u.flags, 3);
                assert_eq!(u.selector.get("_id"), Some(&Value::String("1".into())));
                assert_eq!(u.update.get("x"), Some(&Value::Number(5.into())));
            }
            other => panic!("expected Update, got {:?}", other),
        }
    }

    #[test]
    fn decode_op_delete() {
        let selector = bson::encode_doc(&doc(&[("_id", Value::String("9".into()))])).unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes()); // reserved ZERO
        body.extend_from_slice(&cstr("testdb.c"));
        body.extend_from_slice(&1i32.to_le_bytes()); // flags: single
        body.extend_from_slice(&selector);

        match decode_legacy(&legacy_frame(OP_DELETE, body)).unwrap() {
            LegacyOp::Delete(d) => {
                assert_eq!(d.collection, "testdb.c");
                assert_eq!(d.flags, 1);
                assert_eq!(d.selector.get("_id"), Some(&Value::String("9".into())));
            }
            other => panic!("expected Delete, got {:?}", other),
        }
    }

    #[test]
    fn decode_op_query() {
        let query = bson::encode_doc(&doc(&[("status", Value::String("active".into()))])).unwrap();
        let mut body = Vec::new();
        body.extend_from_slice(&0i32.to_le_bytes()); // flags
        body.extend_from_slice(&cstr("testdb.c"));
        body.extend_from_slice(&5i32.to_le_bytes()); // numberToSkip
        body.extend_from_slice(&10i32.to_le_bytes()); // numberToReturn
        body.extend_from_slice(&query);

        match decode_legacy(&legacy_frame(OP_QUERY, body)).unwrap() {
            LegacyOp::Query(q) => {
                assert_eq!(q.collection, "testdb.c");
                assert_eq!(q.number_to_skip, 5);
                assert_eq!(q.number_to_return, 10);
                assert_eq!(q.query.get("status"), Some(&Value::String("active".into())));
            }
            other => panic!("expected Query, got {:?}", other),
        }
    }

    #[test]
    fn decode_legacy_rejects_unknown_opcode() {
        assert!(decode_legacy(&legacy_frame(2013, vec![0, 0, 0, 0])).is_err());
    }
}
