//! Postgres frontend + backend protocol messages.

use bytes::{Bytes, BytesMut};
use std::collections::HashMap;

/// Frontend message from client.
#[derive(Debug, Clone)]
pub enum FrontendMessage {
    Query(String),
    Parse {
        name: String,
        query: String,
        param_types: Vec<u32>,
    },
    Bind {
        portal: String,
        statement: String,
        params: Vec<Option<Bytes>>,
        result_format: Vec<u16>,
    },
    Describe {
        kind: char, // 'S' = statement, 'P' = portal
        name: String,
    },
    Execute {
        portal: String,
        max_rows: i32,
    },
    Sync,
    Terminate,
}

impl FrontendMessage {
    pub fn parse_from_bytes(msg_type: u8, body: &[u8]) -> Result<Self, String> {
        match msg_type {
            b'Q' => {
                let s = String::from_utf8(body.to_vec())
                    .map_err(|_| "invalid utf8 in query".to_string())?;
                let query = s.trim_end_matches('\0').to_string();
                Ok(FrontendMessage::Query(query))
            }
            b'P' => parse_parse_message(body),
            b'B' => parse_bind_message(body),
            b'D' => parse_describe_message(body),
            b'E' => parse_execute_message(body),
            b'S' => Ok(FrontendMessage::Sync),
            b'X' => Ok(FrontendMessage::Terminate),
            _ => Err(format!("unknown frontend message type: {}", msg_type as char)),
        }
    }
}

fn parse_parse_message(body: &[u8]) -> Result<FrontendMessage, String> {
    let mut offset = 0;
    let name = read_cstring(body, &mut offset)?;
    let query = read_cstring(body, &mut offset)?;
    let param_count = read_i16(body, &mut offset)? as u32;
    let mut param_types = Vec::new();
    for _ in 0..param_count {
        param_types.push(read_u32(body, &mut offset)?);
    }
    Ok(FrontendMessage::Parse {
        name,
        query,
        param_types,
    })
}

fn parse_bind_message(body: &[u8]) -> Result<FrontendMessage, String> {
    let mut offset = 0;
    let portal = read_cstring(body, &mut offset)?;
    let statement = read_cstring(body, &mut offset)?;
    let param_format_count = read_i16(body, &mut offset)? as usize;
    let mut _param_formats = Vec::new();
    for _ in 0..param_format_count {
        _param_formats.push(read_i16(body, &mut offset)?);
    }
    let param_count = read_i16(body, &mut offset)? as usize;
    let mut params = Vec::new();
    for _ in 0..param_count {
        let len = read_i32(body, &mut offset)? as i32;
        if len == -1 {
            params.push(None);
        } else {
            let len = len as usize;
            if offset + len > body.len() {
                return Err("bind param out of bounds".to_string());
            }
            let bytes = Bytes::copy_from_slice(&body[offset..offset + len]);
            params.push(Some(bytes));
            offset += len;
        }
    }
    let result_format_count = read_i16(body, &mut offset)? as usize;
    let mut result_format = Vec::new();
    for _ in 0..result_format_count {
        result_format.push(read_i16(body, &mut offset)? as u16);
    }
    Ok(FrontendMessage::Bind {
        portal,
        statement,
        params,
        result_format,
    })
}

fn parse_describe_message(body: &[u8]) -> Result<FrontendMessage, String> {
    let mut offset = 0;
    let kind = read_u8(body, &mut offset)? as char;
    let name = read_cstring(body, &mut offset)?;
    Ok(FrontendMessage::Describe { kind, name })
}

fn parse_execute_message(body: &[u8]) -> Result<FrontendMessage, String> {
    let mut offset = 0;
    let portal = read_cstring(body, &mut offset)?;
    let max_rows = read_i32(body, &mut offset)?;
    Ok(FrontendMessage::Execute { portal, max_rows })
}

/// Backend message to client.
#[derive(Debug, Clone)]
pub enum BackendMessage {
    AuthenticationOk,
    BackendKeyData { pid: u32, secret: u32 },
    ParameterStatus { name: String, value: String },
    RowDescription { fields: Vec<FieldDescription> },
    DataRow { values: Vec<Option<Bytes>> },
    CommandComplete { tag: String },
    ReadyForQuery { status: char },
    ErrorResponse { fields: HashMap<char, String> },
    ParseComplete,
    BindComplete,
    PortalSuspended,
    EmptyQueryResponse,
}

#[derive(Debug, Clone)]
pub struct FieldDescription {
    pub name: String,
    pub table_oid: u32,
    pub column_attr_num: i16,
    pub type_oid: u32,
    pub type_len: i16,
    pub type_mod: i32,
    pub format: i16, // 0 = text, 1 = binary
}

impl BackendMessage {
    pub fn serialize(&self) -> Result<BytesMut, String> {
        match self {
            BackendMessage::AuthenticationOk => serialize_authentication_ok(),
            BackendMessage::BackendKeyData { pid, secret } => {
                serialize_backend_key_data(*pid, *secret)
            }
            BackendMessage::ParameterStatus { name, value } => {
                serialize_parameter_status(name, value)
            }
            BackendMessage::RowDescription { fields } => serialize_row_description(fields),
            BackendMessage::DataRow { values } => serialize_data_row(values),
            BackendMessage::CommandComplete { tag } => serialize_command_complete(tag),
            BackendMessage::ReadyForQuery { status } => serialize_ready_for_query(*status),
            BackendMessage::ErrorResponse { fields } => serialize_error_response(fields),
            BackendMessage::ParseComplete => serialize_parse_complete(),
            BackendMessage::BindComplete => serialize_bind_complete(),
            BackendMessage::EmptyQueryResponse => serialize_empty_query_response(),
            _ => Err("unimplemented message type".to_string()),
        }
    }
}

fn serialize_authentication_ok() -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"R");
    buf.extend_from_slice(&4i32.to_be_bytes()); // length
    buf.extend_from_slice(&0i32.to_be_bytes()); // auth type 0 = ok
    Ok(buf)
}

fn serialize_backend_key_data(pid: u32, secret: u32) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"K");
    buf.extend_from_slice(&12i32.to_be_bytes()); // 4 + 4 + 4
    buf.extend_from_slice(&pid.to_be_bytes());
    buf.extend_from_slice(&secret.to_be_bytes());
    Ok(buf)
}

fn serialize_parameter_status(name: &str, value: &str) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"S");
    let name_bytes = name.as_bytes();
    let value_bytes = value.as_bytes();
    let len = 4 + name_bytes.len() + 1 + value_bytes.len() + 1;
    buf.extend_from_slice(&(len as i32).to_be_bytes());
    buf.extend_from_slice(name_bytes);
    buf.extend_from_slice(b"\0");
    buf.extend_from_slice(value_bytes);
    buf.extend_from_slice(b"\0");
    Ok(buf)
}

fn serialize_row_description(fields: &[FieldDescription]) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"T");
    let mut content = BytesMut::new();
    content.extend_from_slice(&(fields.len() as i16).to_be_bytes());
    for field in fields {
        let name_bytes = field.name.as_bytes();
        content.extend_from_slice(name_bytes);
        content.extend_from_slice(b"\0");
        content.extend_from_slice(&field.table_oid.to_be_bytes());
        content.extend_from_slice(&field.column_attr_num.to_be_bytes());
        content.extend_from_slice(&field.type_oid.to_be_bytes());
        content.extend_from_slice(&field.type_len.to_be_bytes());
        content.extend_from_slice(&field.type_mod.to_be_bytes());
        content.extend_from_slice(&field.format.to_be_bytes());
    }
    let len = 4 + content.len();
    buf.extend_from_slice(&(len as i32).to_be_bytes());
    buf.extend_from_slice(&content);
    Ok(buf)
}

fn serialize_data_row(values: &[Option<Bytes>]) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"D");
    let mut content = BytesMut::new();
    content.extend_from_slice(&(values.len() as i16).to_be_bytes());
    for val in values {
        match val {
            None => {
                content.extend_from_slice(&(-1i32).to_be_bytes());
            }
            Some(bytes) => {
                content.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
                content.extend_from_slice(bytes);
            }
        }
    }
    let len = 4 + content.len();
    buf.extend_from_slice(&(len as i32).to_be_bytes());
    buf.extend_from_slice(&content);
    Ok(buf)
}

fn serialize_command_complete(tag: &str) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"C");
    let tag_bytes = tag.as_bytes();
    let len = 4 + tag_bytes.len() + 1;
    buf.extend_from_slice(&(len as i32).to_be_bytes());
    buf.extend_from_slice(tag_bytes);
    buf.extend_from_slice(b"\0");
    Ok(buf)
}

fn serialize_ready_for_query(status: char) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"Z");
    buf.extend_from_slice(&5i32.to_be_bytes()); // 4 + 1
    buf.extend_from_slice(&[status as u8]);
    Ok(buf)
}

fn serialize_parse_complete() -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"1");
    buf.extend_from_slice(&4i32.to_be_bytes());
    Ok(buf)
}

fn serialize_bind_complete() -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"2");
    buf.extend_from_slice(&4i32.to_be_bytes());
    Ok(buf)
}

fn serialize_empty_query_response() -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"I");
    buf.extend_from_slice(&4i32.to_be_bytes());
    Ok(buf)
}

fn serialize_error_response(fields: &HashMap<char, String>) -> Result<BytesMut, String> {
    let mut buf = BytesMut::new();
    buf.extend_from_slice(b"E");
    let mut content = BytesMut::new();
    for (key, val) in fields {
        content.extend_from_slice(&[*key as u8]);
        let val_bytes = val.as_bytes();
        content.extend_from_slice(val_bytes);
        content.extend_from_slice(b"\0");
    }
    content.extend_from_slice(b"\0");
    let len = 4 + content.len();
    buf.extend_from_slice(&(len as i32).to_be_bytes());
    buf.extend_from_slice(&content);
    Ok(buf)
}

// Helper functions for parsing

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8, String> {
    if *offset >= data.len() {
        return Err("read past end".to_string());
    }
    let val = data[*offset];
    *offset += 1;
    Ok(val)
}

fn read_i16(data: &[u8], offset: &mut usize) -> Result<i16, String> {
    if *offset + 2 > data.len() {
        return Err("read past end".to_string());
    }
    let val = i16::from_be_bytes([data[*offset], data[*offset + 1]]);
    *offset += 2;
    Ok(val)
}

fn read_i32(data: &[u8], offset: &mut usize) -> Result<i32, String> {
    if *offset + 4 > data.len() {
        return Err("read past end".to_string());
    }
    let val = i32::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(val)
}

fn read_u32(data: &[u8], offset: &mut usize) -> Result<u32, String> {
    if *offset + 4 > data.len() {
        return Err("read past end".to_string());
    }
    let val = u32::from_be_bytes([
        data[*offset],
        data[*offset + 1],
        data[*offset + 2],
        data[*offset + 3],
    ]);
    *offset += 4;
    Ok(val)
}

fn read_cstring(data: &[u8], offset: &mut usize) -> Result<String, String> {
    let start = *offset;
    while *offset < data.len() && data[*offset] != 0 {
        *offset += 1;
    }
    if *offset >= data.len() {
        return Err("cstring not null-terminated".to_string());
    }
    let s = String::from_utf8(data[start..*offset].to_vec())
        .map_err(|_| "invalid utf8 in cstring".to_string())?;
    *offset += 1; // skip null terminator
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_query_message() {
        let mut body = b"SELECT * FROM users\0".to_vec();
        let msg = FrontendMessage::parse_from_bytes(b'Q', &body);
        assert!(matches!(msg, Ok(FrontendMessage::Query(ref q)) if q == "SELECT * FROM users"));
    }

    #[test]
    fn test_serialize_authentication_ok() {
        let msg = BackendMessage::AuthenticationOk;
        let buf = msg.serialize().unwrap();
        assert_eq!(buf[0], b'R');
        assert_eq!(buf.len(), 9); // 1 + 4 + 4
    }

    #[test]
    fn test_serialize_parameter_status() {
        let msg = BackendMessage::ParameterStatus {
            name: "server_version".to_string(),
            value: "14.0".to_string(),
        };
        let buf = msg.serialize().unwrap();
        assert_eq!(buf[0], b'S');
    }
}
