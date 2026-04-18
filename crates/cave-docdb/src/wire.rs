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

pub fn encode_op_msg(reply: &OpMsg, request_id: i32, response_to: i32) -> Result<Vec<u8>, WireError> {
    let mut payload = Vec::new();

    // Encode flag bits
    payload.extend_from_slice(&reply.flag_bits.to_le_bytes());

    // Encode sections
    for section in &reply.sections {
        match section {
            Section::Body(doc) => {
                let encoded = bson::encode_doc(doc)
                    .map_err(|e| WireError::BsonError(e.to_string()))?;
                payload.extend_from_slice(&encoded);
            }
            Section::DocumentSequence(seq_name, docs) => {
                payload.push(0x01); // Document sequence marker
                let mut seq_data = Vec::new();
                seq_data.extend_from_slice(seq_name.as_bytes());
                seq_data.push(0);
                let mut docs_payload = Vec::new();
                for doc in docs {
                    let encoded = bson::encode_doc(doc)
                        .map_err(|e| WireError::BsonError(e.to_string()))?;
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

    let flag_bits = u32::from_le_bytes([
        bytes[16],
        bytes[17],
        bytes[18],
        bytes[19],
    ]);

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
            let doc = bson::decode_doc(&bytes[pos..])
                .map_err(|e| WireError::BsonError(e.to_string()))?;
            let encoded = bson::encode_doc(&doc)
                .map_err(|e| WireError::BsonError(e.to_string()))?;
            let consumed = encoded.len();
            sections.push(Section::Body(doc));
            pos += consumed;
        } else if section_type == 1 {
            // Document sequence
            if pos + 4 > message_length {
                break;
            }
            let _seq_size = u32::from_le_bytes([
                bytes[pos],
                bytes[pos + 1],
                bytes[pos + 2],
                bytes[pos + 3],
            ]) as usize;
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
                let encoded = bson::encode_doc(&doc)
                    .map_err(|e| WireError::BsonError(e.to_string()))?;
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
        let (req_id, _decoded) = decode_op_msg(&encoded).unwrap();

        assert_eq!(req_id, 42);
        assert!(encoded.len() > 16); // At least header + some payload
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
}
