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

    /// Build a legacy OP_QUERY frame (opcode 2004) the way a pre-OP_MSG driver
    /// sends its `isMaster` / `hello` handshake against `admin.$cmd`.
    fn build_op_query(req_id: i32, ns: &str, query: &Document) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0i32.to_le_bytes()); // flags
        payload.extend_from_slice(ns.as_bytes()); // fullCollectionName
        payload.push(0);
        payload.extend_from_slice(&0i32.to_le_bytes()); // numberToSkip
        payload.extend_from_slice(&(-1i32).to_le_bytes()); // numberToReturn
        payload.extend_from_slice(&bson::encode_doc(query).unwrap()); // query doc

        let mut msg = Vec::new();
        msg.extend_from_slice(&((16 + payload.len()) as u32).to_le_bytes());
        msg.extend_from_slice(&req_id.to_le_bytes());
        msg.extend_from_slice(&0i32.to_le_bytes()); // responseTo
        msg.extend_from_slice(&2004i32.to_le_bytes()); // OP_QUERY
        msg.extend_from_slice(&payload);
        msg
    }

    #[test]
    fn test_decode_op_query_handshake() {
        let mut q = Document::new();
        q.insert("isMaster".to_string(), Value::Number(1.into()));
        let frame = build_op_query(11, "admin.$cmd", &q);

        let (req_id, query) = decode_op_query(&frame).unwrap();
        assert_eq!(req_id, 11);
        assert_eq!(query.full_collection_name, "admin.$cmd");
        assert_eq!(query.number_to_skip, 0);
        assert_eq!(query.number_to_return, -1);
        assert_eq!(
            query.query.get("isMaster"),
            Some(&Value::Number(1.into()))
        );
    }

    #[test]
    fn test_decode_op_query_rejects_non_2004_opcode() {
        // Hand an OP_MSG frame to the OP_QUERY decoder — must be rejected.
        let mut doc = Document::new();
        doc.insert("ping".to_string(), Value::Number(1.into()));
        let frame = encode_op_msg(&OpMsg::new(doc), 1, 0).unwrap();
        assert!(decode_op_query(&frame).is_err());
    }

    #[test]
    fn test_encode_op_reply_header_and_body() {
        let mut doc = Document::new();
        doc.insert("ismaster".to_string(), Value::Bool(true));
        doc.insert("ok".to_string(), Value::Number(1.into()));

        let reply = encode_op_reply(&[doc], 99, 11).unwrap();
        // Header: messageLength, requestID, responseTo, opcode(=1, OP_REPLY).
        assert!(reply.len() >= 36); // 16 header + 20 reply prefix minimum
        let opcode = i32::from_le_bytes([reply[12], reply[13], reply[14], reply[15]]);
        assert_eq!(opcode, 1);
        let response_to = i32::from_le_bytes([reply[8], reply[9], reply[10], reply[11]]);
        assert_eq!(response_to, 11);
        // numberReturned sits at offset 16(header)+4(flags)+8(cursorID)+4(startingFrom)=32.
        let number_returned =
            i32::from_le_bytes([reply[32], reply[33], reply[34], reply[35]]);
        assert_eq!(number_returned, 1);
        // The single reply document round-trips back from its offset (36).
        let body = bson::decode_doc(&reply[36..]).unwrap();
        assert_eq!(body.get("ok"), Some(&Value::Number(1.into())));
        assert_eq!(body.get("ismaster"), Some(&Value::Bool(true)));
    }
}
