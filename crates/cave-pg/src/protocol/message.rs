//! PostgreSQL wire protocol v3 message types.
//!
//! Covers every frontend (client→server) and backend (server→client) message
//! defined in the PostgreSQL protocol specification.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use crate::error::{PgError, Result, Error};
use crate::types::{ColumnDesc, FormatCode, Oid};

// ─────────────────────────────────────────────────────────────────────────────
// Startup messages (before authentication)
// ─────────────────────────────────────────────────────────────────────────────

/// The initial startup message sent by the client before authentication.
/// Protocol version 3.0 = 196608 (0x00030000).
#[derive(Debug, Clone)]
pub struct StartupMessage {
    pub protocol_version: u32,
    pub parameters: HashMap<String, String>,
}

impl StartupMessage {
    pub const PROTOCOL_V3: u32 = 196608;
    pub const SSL_REQUEST: u32 = 80877103;   // 0x04D2162F
    pub const CANCEL_REQUEST: u32 = 80877102; // 0x04D2162E
    pub const GSSENC_REQUEST: u32 = 80877104;  // 0x04D21630

    pub fn is_ssl_request(&self) -> bool {
        self.protocol_version == Self::SSL_REQUEST
    }

    pub fn is_cancel_request(&self) -> bool {
        self.protocol_version == Self::CANCEL_REQUEST
    }

    pub fn is_gssenc_request(&self) -> bool {
        self.protocol_version == Self::GSSENC_REQUEST
    }

    pub fn database(&self) -> Option<&str> {
        self.parameters.get("database").map(String::as_str)
    }

    pub fn user(&self) -> Option<&str> {
        self.parameters.get("user").map(String::as_str)
    }

    pub fn application_name(&self) -> Option<&str> {
        self.parameters.get("application_name").map(String::as_str)
    }

    /// Parse from raw bytes (without the 4-byte length prefix already consumed).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            return Err(Error::Protocol("startup message too short".into()));
        }
        let mut cursor = std::io::Cursor::new(data);
        let protocol_version = cursor.get_u32();

        let mut parameters = HashMap::new();

        if protocol_version == Self::SSL_REQUEST
            || protocol_version == Self::CANCEL_REQUEST
            || protocol_version == Self::GSSENC_REQUEST
        {
            return Ok(Self { protocol_version, parameters });
        }

        // Read null-terminated key=value pairs
        let remaining = &data[4..];
        let mut i = 0;
        while i < remaining.len() {
            if remaining[i] == 0 {
                break; // terminator
            }
            let key = read_cstring(remaining, &mut i)?;
            if key.is_empty() {
                break;
            }
            let value = read_cstring(remaining, &mut i)?;
            parameters.insert(key, value);
        }

        Ok(Self { protocol_version, parameters })
    }
}

/// Cancel request — contains the backend PID and secret key.
#[derive(Debug, Clone)]
pub struct CancelRequest {
    pub pid: u32,
    pub secret_key: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Frontend messages (client → server, after startup)
// ─────────────────────────────────────────────────────────────────────────────

/// All frontend message types.
#[derive(Debug, Clone)]
pub enum FrontendMessage {
    /// 'Q' — simple query
    Query(QueryMessage),
    /// 'P' — parse (extended query)
    Parse(ParseMessage),
    /// 'B' — bind (extended query)
    Bind(BindMessage),
    /// 'D' — describe (extended query)
    Describe(DescribeMessage),
    /// 'E' — execute (extended query)
    Execute(ExecuteMessage),
    /// 'C' — close (extended query)
    Close(CloseMessage),
    /// 'S' — sync
    Sync,
    /// 'H' — flush
    Flush,
    /// 'd' — copy data
    CopyData(Vec<u8>),
    /// 'c' — copy done
    CopyDone,
    /// 'f' — copy fail
    CopyFail(String),
    /// 'p' — password message (MD5 or SCRAM response)
    Password(Vec<u8>),
    /// 'X' — terminate
    Terminate,
    /// 'F' — function call (legacy, pre-v3 compatibility)
    FunctionCall(FunctionCallMessage),
}

/// 'Q' simple query message.
#[derive(Debug, Clone)]
pub struct QueryMessage {
    pub query: String,
}

/// 'P' parse — prepare a statement.
#[derive(Debug, Clone)]
pub struct ParseMessage {
    pub statement_name: String,
    pub query: String,
    pub param_types: Vec<Oid>,
}

/// 'B' bind — bind parameters to a prepared statement, creating a portal.
#[derive(Debug, Clone)]
pub struct BindMessage {
    pub portal_name: String,
    pub statement_name: String,
    pub param_formats: Vec<FormatCode>,
    pub params: Vec<Option<Vec<u8>>>,
    pub result_formats: Vec<FormatCode>,
}

/// 'D' describe — request metadata for a statement or portal.
#[derive(Debug, Clone)]
pub struct DescribeMessage {
    pub kind: DescribeKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescribeKind {
    Statement, // 'S'
    Portal,    // 'P'
}

/// 'E' execute — run a portal.
#[derive(Debug, Clone)]
pub struct ExecuteMessage {
    pub portal_name: String,
    pub max_rows: i32,  // 0 = unlimited
}

/// 'C' close — close a statement or portal.
#[derive(Debug, Clone)]
pub struct CloseMessage {
    pub kind: DescribeKind,
    pub name: String,
}

/// 'F' function call (legacy).
#[derive(Debug, Clone)]
pub struct FunctionCallMessage {
    pub function_oid: Oid,
    pub arg_formats: Vec<FormatCode>,
    pub args: Vec<Option<Vec<u8>>>,
    pub result_format: FormatCode,
}

impl FrontendMessage {
    /// Parse a frontend message from its type byte + body bytes.
    pub fn parse(msg_type: u8, body: &[u8]) -> Result<Self> {
        match msg_type {
            b'Q' => {
                let query = read_cstring_from_start(body)?;
                Ok(FrontendMessage::Query(QueryMessage { query }))
            }
            b'P' => {
                let mut i = 0;
                let statement_name = read_cstring(body, &mut i)?;
                let query = read_cstring(body, &mut i)?;
                let n_types = read_i16_be(body, &mut i)? as usize;
                let mut param_types = Vec::with_capacity(n_types);
                for _ in 0..n_types {
                    param_types.push(read_u32_be(body, &mut i)?);
                }
                Ok(FrontendMessage::Parse(ParseMessage { statement_name, query, param_types }))
            }
            b'B' => {
                let mut i = 0;
                let portal_name = read_cstring(body, &mut i)?;
                let statement_name = read_cstring(body, &mut i)?;
                let n_formats = read_i16_be(body, &mut i)? as usize;
                let mut param_formats = Vec::with_capacity(n_formats);
                for _ in 0..n_formats {
                    param_formats.push(FormatCode::from_i16(read_i16_be(body, &mut i)?));
                }
                let n_params = read_i16_be(body, &mut i)? as usize;
                let mut params = Vec::with_capacity(n_params);
                for _ in 0..n_params {
                    let len = read_i32_be(body, &mut i)?;
                    if len == -1 {
                        params.push(None);
                    } else {
                        let end = i + len as usize;
                        if end > body.len() {
                            return Err(Error::Protocol("bind param overflows body".into()));
                        }
                        params.push(Some(body[i..end].to_vec()));
                        i = end;
                    }
                }
                let n_result_formats = read_i16_be(body, &mut i)? as usize;
                let mut result_formats = Vec::with_capacity(n_result_formats);
                for _ in 0..n_result_formats {
                    result_formats.push(FormatCode::from_i16(read_i16_be(body, &mut i)?));
                }
                Ok(FrontendMessage::Bind(BindMessage {
                    portal_name, statement_name, param_formats, params, result_formats,
                }))
            }
            b'D' => {
                if body.is_empty() {
                    return Err(Error::Protocol("describe message empty".into()));
                }
                let kind = match body[0] {
                    b'S' => DescribeKind::Statement,
                    b'P' => DescribeKind::Portal,
                    other => return Err(Error::Protocol(format!("unknown describe kind: {other}"))),
                };
                let name = read_cstring_from_start(&body[1..])?;
                Ok(FrontendMessage::Describe(DescribeMessage { kind, name }))
            }
            b'E' => {
                let mut i = 0;
                let portal_name = read_cstring(body, &mut i)?;
                let max_rows = read_i32_be(body, &mut i)?;
                Ok(FrontendMessage::Execute(ExecuteMessage { portal_name, max_rows }))
            }
            b'C' => {
                if body.is_empty() {
                    return Err(Error::Protocol("close message empty".into()));
                }
                let kind = match body[0] {
                    b'S' => DescribeKind::Statement,
                    b'P' => DescribeKind::Portal,
                    other => return Err(Error::Protocol(format!("unknown close kind: {other}"))),
                };
                let name = read_cstring_from_start(&body[1..])?;
                Ok(FrontendMessage::Close(CloseMessage { kind, name }))
            }
            b'S' => Ok(FrontendMessage::Sync),
            b'H' => Ok(FrontendMessage::Flush),
            b'd' => Ok(FrontendMessage::CopyData(body.to_vec())),
            b'c' => Ok(FrontendMessage::CopyDone),
            b'f' => {
                let reason = read_cstring_from_start(body)?;
                Ok(FrontendMessage::CopyFail(reason))
            }
            b'p' => Ok(FrontendMessage::Password(body.to_vec())),
            b'X' => Ok(FrontendMessage::Terminate),
            b'F' => {
                let mut i = 0;
                let function_oid = read_u32_be(body, &mut i)?;
                let n_arg_formats = read_i16_be(body, &mut i)? as usize;
                let mut arg_formats = Vec::with_capacity(n_arg_formats);
                for _ in 0..n_arg_formats {
                    arg_formats.push(FormatCode::from_i16(read_i16_be(body, &mut i)?));
                }
                let n_args = read_i16_be(body, &mut i)? as usize;
                let mut args = Vec::with_capacity(n_args);
                for _ in 0..n_args {
                    let len = read_i32_be(body, &mut i)?;
                    if len == -1 { args.push(None); }
                    else {
                        let end = i + len as usize;
                        args.push(Some(body[i..end].to_vec()));
                        i = end;
                    }
                }
                let result_format = FormatCode::from_i16(read_i16_be(body, &mut i)?);
                Ok(FrontendMessage::FunctionCall(FunctionCallMessage {
                    function_oid, arg_formats, args, result_format,
                }))
            }
            other => Err(Error::Protocol(format!("unknown frontend message type: '{}'", other as char))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Backend messages (server → client)
// ─────────────────────────────────────────────────────────────────────────────

/// Transaction status indicator in ReadyForQuery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    Idle,           // 'I'
    InTransaction,  // 'T'
    Failed,         // 'E'
}

impl TransactionStatus {
    pub fn as_byte(self) -> u8 {
        match self {
            Self::Idle => b'I',
            Self::InTransaction => b'T',
            Self::Failed => b'E',
        }
    }
}

/// Authentication request types.
#[derive(Debug, Clone)]
pub enum AuthRequest {
    Ok,
    KerberosV5,
    CleartextPassword,
    MD5Password { salt: [u8; 4] },
    SCMCredential,
    GSS,
    GSSContinue(Vec<u8>),
    SSPI,
    SASL(Vec<String>),
    SASLContinue(Vec<u8>),
    SASLFinal(Vec<u8>),
}

/// All backend message types.
#[derive(Debug, Clone)]
pub enum BackendMessage {
    // ── Authentication
    Authentication(AuthRequest),
    // ── Connection setup
    BackendKeyData { pid: u32, secret_key: u32 },
    ParameterStatus { name: String, value: String },
    // ── Query response
    RowDescription(Vec<ColumnDesc>),
    DataRow(Vec<Option<Vec<u8>>>),
    CommandComplete(String),
    EmptyQueryResponse,
    // ── Extended query
    ParseComplete,
    BindComplete,
    CloseComplete,
    ParameterDescription(Vec<Oid>),
    NoData,
    PortalSuspended,
    // ── Copy protocol
    CopyInResponse { overall_format: FormatCode, column_formats: Vec<FormatCode> },
    CopyOutResponse { overall_format: FormatCode, column_formats: Vec<FormatCode> },
    CopyBothResponse { overall_format: FormatCode, column_formats: Vec<FormatCode> },
    CopyData(Vec<u8>),
    CopyDone,
    // ── Notifications / notices
    NotificationResponse { pid: u32, channel: String, payload: String },
    NoticeResponse(PgError),
    ErrorResponse(PgError),
    // ── Ready for query
    ReadyForQuery(TransactionStatus),
    // ── Function call result (legacy)
    FunctionCallResponse(Option<Vec<u8>>),
    // ── Negotiation
    NegotiateProtocolVersion { newest_minor: u32, unrecognized: Vec<String> },
}

impl BackendMessage {
    /// Encode this message into bytes suitable for sending over the wire.
    pub fn encode(&self) -> Bytes {
        let mut buf = BytesMut::new();
        match self {
            BackendMessage::Authentication(auth) => {
                buf.put_u8(b'R');
                match auth {
                    AuthRequest::Ok => {
                        buf.put_i32(8);
                        buf.put_i32(0);
                    }
                    AuthRequest::KerberosV5 => {
                        buf.put_i32(8);
                        buf.put_i32(2);
                    }
                    AuthRequest::CleartextPassword => {
                        buf.put_i32(8);
                        buf.put_i32(3);
                    }
                    AuthRequest::MD5Password { salt } => {
                        buf.put_i32(12);
                        buf.put_i32(5);
                        buf.put_slice(salt);
                    }
                    AuthRequest::SCMCredential => {
                        buf.put_i32(8);
                        buf.put_i32(6);
                    }
                    AuthRequest::GSS => {
                        buf.put_i32(8);
                        buf.put_i32(7);
                    }
                    AuthRequest::GSSContinue(data) => {
                        buf.put_i32(8 + data.len() as i32);
                        buf.put_i32(8);
                        buf.put_slice(data);
                    }
                    AuthRequest::SSPI => {
                        buf.put_i32(8);
                        buf.put_i32(9);
                    }
                    AuthRequest::SASL(mechanisms) => {
                        let mut body = BytesMut::new();
                        body.put_i32(10); // type
                        for mech in mechanisms {
                            body.put_slice(mech.as_bytes());
                            body.put_u8(0);
                        }
                        body.put_u8(0); // terminator
                        buf.put_i32(4 + body.len() as i32);
                        buf.put_slice(&body);
                    }
                    AuthRequest::SASLContinue(data) => {
                        buf.put_i32(8 + data.len() as i32);
                        buf.put_i32(11);
                        buf.put_slice(data);
                    }
                    AuthRequest::SASLFinal(data) => {
                        buf.put_i32(8 + data.len() as i32);
                        buf.put_i32(12);
                        buf.put_slice(data);
                    }
                }
            }
            BackendMessage::BackendKeyData { pid, secret_key } => {
                buf.put_u8(b'K');
                buf.put_i32(12);
                buf.put_u32(*pid);
                buf.put_u32(*secret_key);
            }
            BackendMessage::ParameterStatus { name, value } => {
                buf.put_u8(b'S');
                let body_len = 4 + name.len() + 1 + value.len() + 1;
                buf.put_i32(body_len as i32);
                buf.put_slice(name.as_bytes());
                buf.put_u8(0);
                buf.put_slice(value.as_bytes());
                buf.put_u8(0);
            }
            BackendMessage::RowDescription(cols) => {
                buf.put_u8(b'T');
                let mut body = BytesMut::new();
                body.put_i16(cols.len() as i16);
                for col in cols {
                    body.put_slice(col.name.as_bytes());
                    body.put_u8(0);
                    body.put_u32(col.table_oid);
                    body.put_i16(col.col_attr_num);
                    body.put_u32(col.type_oid);
                    body.put_i16(col.type_size);
                    body.put_i32(col.type_modifier);
                    body.put_i16(col.format as i16);
                }
                buf.put_i32(4 + body.len() as i32);
                buf.put_slice(&body);
            }
            BackendMessage::DataRow(values) => {
                buf.put_u8(b'D');
                let mut body = BytesMut::new();
                body.put_i16(values.len() as i16);
                for val in values {
                    match val {
                        None => body.put_i32(-1),
                        Some(data) => {
                            body.put_i32(data.len() as i32);
                            body.put_slice(data);
                        }
                    }
                }
                buf.put_i32(4 + body.len() as i32);
                buf.put_slice(&body);
            }
            BackendMessage::CommandComplete(tag) => {
                buf.put_u8(b'C');
                buf.put_i32(4 + tag.len() as i32 + 1);
                buf.put_slice(tag.as_bytes());
                buf.put_u8(0);
            }
            BackendMessage::EmptyQueryResponse => {
                buf.put_u8(b'I');
                buf.put_i32(4);
            }
            BackendMessage::ParseComplete => {
                buf.put_u8(b'1');
                buf.put_i32(4);
            }
            BackendMessage::BindComplete => {
                buf.put_u8(b'2');
                buf.put_i32(4);
            }
            BackendMessage::CloseComplete => {
                buf.put_u8(b'3');
                buf.put_i32(4);
            }
            BackendMessage::ParameterDescription(oids) => {
                buf.put_u8(b't');
                buf.put_i32(4 + 2 + (oids.len() * 4) as i32);
                buf.put_i16(oids.len() as i16);
                for oid in oids {
                    buf.put_u32(*oid);
                }
            }
            BackendMessage::NoData => {
                buf.put_u8(b'n');
                buf.put_i32(4);
            }
            BackendMessage::PortalSuspended => {
                buf.put_u8(b's');
                buf.put_i32(4);
            }
            BackendMessage::CopyInResponse { overall_format, column_formats } => {
                encode_copy_response(&mut buf, b'G', *overall_format, column_formats);
            }
            BackendMessage::CopyOutResponse { overall_format, column_formats } => {
                encode_copy_response(&mut buf, b'H', *overall_format, column_formats);
            }
            BackendMessage::CopyBothResponse { overall_format, column_formats } => {
                encode_copy_response(&mut buf, b'W', *overall_format, column_formats);
            }
            BackendMessage::CopyData(data) => {
                buf.put_u8(b'd');
                buf.put_i32(4 + data.len() as i32);
                buf.put_slice(data);
            }
            BackendMessage::CopyDone => {
                buf.put_u8(b'c');
                buf.put_i32(4);
            }
            BackendMessage::NotificationResponse { pid, channel, payload } => {
                buf.put_u8(b'A');
                let body_len = 4 + 4 + channel.len() + 1 + payload.len() + 1;
                buf.put_i32(body_len as i32);
                buf.put_u32(*pid);
                buf.put_slice(channel.as_bytes());
                buf.put_u8(0);
                buf.put_slice(payload.as_bytes());
                buf.put_u8(0);
            }
            BackendMessage::NoticeResponse(err) => {
                encode_error_or_notice(&mut buf, b'N', err);
            }
            BackendMessage::ErrorResponse(err) => {
                encode_error_or_notice(&mut buf, b'E', err);
            }
            BackendMessage::ReadyForQuery(status) => {
                buf.put_u8(b'Z');
                buf.put_i32(5);
                buf.put_u8(status.as_byte());
            }
            BackendMessage::FunctionCallResponse(result) => {
                buf.put_u8(b'V');
                match result {
                    None => {
                        buf.put_i32(8);
                        buf.put_i32(-1);
                    }
                    Some(data) => {
                        buf.put_i32(8 + data.len() as i32);
                        buf.put_i32(data.len() as i32);
                        buf.put_slice(data);
                    }
                }
            }
            BackendMessage::NegotiateProtocolVersion { newest_minor, unrecognized } => {
                buf.put_u8(b'v');
                let options_len: usize = unrecognized.iter().map(|s| s.len() + 1).sum();
                buf.put_i32(4 + 4 + 4 + options_len as i32);
                buf.put_u32(*newest_minor);
                buf.put_i32(unrecognized.len() as i32);
                for opt in unrecognized {
                    buf.put_slice(opt.as_bytes());
                    buf.put_u8(0);
                }
            }
        }
        buf.freeze()
    }
}

fn encode_copy_response(
    buf: &mut BytesMut,
    msg_type: u8,
    overall_format: FormatCode,
    column_formats: &[FormatCode],
) {
    buf.put_u8(msg_type);
    buf.put_i32(4 + 1 + 2 + (2 * column_formats.len()) as i32);
    buf.put_i8(overall_format as i8);
    buf.put_i16(column_formats.len() as i16);
    for fmt in column_formats {
        buf.put_i16(*fmt as i16);
    }
}

fn encode_error_or_notice(buf: &mut BytesMut, msg_type: u8, err: &PgError) {
    let mut fields = BytesMut::new();

    // S — Severity (localized)
    fields.put_u8(b'S');
    fields.put_slice(err.severity.as_str().as_bytes());
    fields.put_u8(0);

    // V — Severity (non-localized)
    fields.put_u8(b'V');
    fields.put_slice(err.severity.as_str().as_bytes());
    fields.put_u8(0);

    // C — SQLSTATE code
    fields.put_u8(b'C');
    fields.put_slice(err.sqlstate.0.as_bytes());
    fields.put_u8(0);

    // M — Message
    fields.put_u8(b'M');
    fields.put_slice(err.message.as_bytes());
    fields.put_u8(0);

    // D — Detail
    if let Some(detail) = &err.detail {
        fields.put_u8(b'D');
        fields.put_slice(detail.as_bytes());
        fields.put_u8(0);
    }

    // H — Hint
    if let Some(hint) = &err.hint {
        fields.put_u8(b'H');
        fields.put_slice(hint.as_bytes());
        fields.put_u8(0);
    }

    // P — Position
    if let Some(pos) = &err.position {
        fields.put_u8(b'P');
        fields.put_slice(pos.to_string().as_bytes());
        fields.put_u8(0);
    }

    // p — Internal position
    if let Some(pos) = &err.internal_position {
        fields.put_u8(b'p');
        fields.put_slice(pos.to_string().as_bytes());
        fields.put_u8(0);
    }

    // q — Internal query
    if let Some(q) = &err.internal_query {
        fields.put_u8(b'q');
        fields.put_slice(q.as_bytes());
        fields.put_u8(0);
    }

    // W — Where context
    if let Some(w) = &err.where_context {
        fields.put_u8(b'W');
        fields.put_slice(w.as_bytes());
        fields.put_u8(0);
    }

    // s — Schema name
    if let Some(s) = &err.schema_name {
        fields.put_u8(b's');
        fields.put_slice(s.as_bytes());
        fields.put_u8(0);
    }

    // t — Table name
    if let Some(t) = &err.table_name {
        fields.put_u8(b't');
        fields.put_slice(t.as_bytes());
        fields.put_u8(0);
    }

    // c — Column name
    if let Some(c) = &err.column_name {
        fields.put_u8(b'c');
        fields.put_slice(c.as_bytes());
        fields.put_u8(0);
    }

    // d — Data type name
    if let Some(d) = &err.data_type_name {
        fields.put_u8(b'd');
        fields.put_slice(d.as_bytes());
        fields.put_u8(0);
    }

    // n — Constraint name
    if let Some(n) = &err.constraint_name {
        fields.put_u8(b'n');
        fields.put_slice(n.as_bytes());
        fields.put_u8(0);
    }

    // F — File (source file name)
    if let Some(f) = &err.file {
        fields.put_u8(b'F');
        fields.put_slice(f.as_bytes());
        fields.put_u8(0);
    }

    // L — Line number
    if let Some(l) = &err.line {
        fields.put_u8(b'L');
        fields.put_slice(l.to_string().as_bytes());
        fields.put_u8(0);
    }

    // R — Routine name
    if let Some(r) = &err.routine {
        fields.put_u8(b'R');
        fields.put_slice(r.as_bytes());
        fields.put_u8(0);
    }

    // Null terminator
    fields.put_u8(0);

    buf.put_u8(msg_type);
    buf.put_i32(4 + fields.len() as i32);
    buf.put_slice(&fields);
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_cstring(buf: &[u8], pos: &mut usize) -> Result<String> {
    let start = *pos;
    let end = buf[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|i| start + i)
        .ok_or_else(|| Error::Protocol("missing null terminator in cstring".into()))?;
    let s = std::str::from_utf8(&buf[start..end])
        .map_err(|e| Error::Protocol(format!("invalid UTF-8 in cstring: {e}")))?
        .to_string();
    *pos = end + 1;
    Ok(s)
}

fn read_cstring_from_start(buf: &[u8]) -> Result<String> {
    let mut pos = 0;
    read_cstring(buf, &mut pos)
}

fn read_i16_be(buf: &[u8], pos: &mut usize) -> Result<i16> {
    if *pos + 2 > buf.len() {
        return Err(Error::Protocol("buffer underrun reading i16".into()));
    }
    let v = i16::from_be_bytes([buf[*pos], buf[*pos + 1]]);
    *pos += 2;
    Ok(v)
}

fn read_i32_be(buf: &[u8], pos: &mut usize) -> Result<i32> {
    if *pos + 4 > buf.len() {
        return Err(Error::Protocol("buffer underrun reading i32".into()));
    }
    let v = i32::from_be_bytes([buf[*pos], buf[*pos + 1], buf[*pos + 2], buf[*pos + 3]]);
    *pos += 4;
    Ok(v)
}

fn read_u32_be(buf: &[u8], pos: &mut usize) -> Result<u32> {
    read_i32_be(buf, pos).map(|v| v as u32)
}
