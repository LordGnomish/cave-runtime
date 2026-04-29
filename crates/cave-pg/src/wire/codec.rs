//! Frontend ↔ backend message framing.
//!
//! Wire format (PostgreSQL protocol v3.0):
//!
//! ```text
//! ┌─────────┬─────────────┬────────────┐
//! │ type:1  │ length:i32  │ payload:.. │
//! └─────────┴─────────────┴────────────┘
//! ```
//!
//! `length` covers the i32 itself plus payload (but not the type byte). The
//! startup, SSL, GSS-ENC and Cancel requests are special — they have **no
//! type byte**; the first frame of a connection is always one of those four,
//! discriminated by the protocol-version int that follows the length.

use bytes::{Buf, BufMut, BytesMut};

/// Errors raised by the wire codec.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("incomplete frame: need {needed} more bytes")]
    Incomplete { needed: usize },
    #[error("unknown frontend message type: 0x{0:02x}")]
    UnknownFrontendType(u8),
    #[error("unknown backend message type: 0x{0:02x}")]
    UnknownBackendType(u8),
    #[error("invalid length: {0}")]
    InvalidLength(i32),
    #[error("invalid utf-8 in field: {0}")]
    Utf8(String),
    #[error("missing nul terminator in cstring")]
    MissingNul,
    #[error("unsupported protocol version: {major}.{minor}")]
    UnsupportedVersion { major: u16, minor: u16 },
    #[error("invalid value for {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
}

pub type ProtocolResult<T> = Result<T, ProtocolError>;

// ── Frontend messages ────────────────────────────────────────────────────────

/// Messages flowing client → server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontendMessage {
    /// Initial frame variants (no type byte).
    Startup(crate::wire::startup::StartupMessage),
    SslRequest,
    GssEncRequest,
    Cancel { process_id: i32, secret_key: i32 },
    /// 'Q' — simple query.
    Query(String),
    /// 'P' — Parse.
    Parse {
        statement_name: String,
        query: String,
        param_oids: Vec<u32>,
    },
    /// 'B' — Bind.
    Bind {
        portal_name: String,
        statement_name: String,
        param_formats: Vec<i16>,
        params: Vec<Option<Vec<u8>>>,
        result_formats: Vec<i16>,
    },
    /// 'D' — Describe (kind: 'S' = statement, 'P' = portal).
    Describe { kind: u8, name: String },
    /// 'E' — Execute.
    Execute { portal_name: String, max_rows: i32 },
    /// 'C' — Close (kind: 'S' or 'P').
    Close { kind: u8, name: String },
    /// 'S' — Sync.
    Sync,
    /// 'H' — Flush.
    Flush,
    /// 'X' — Terminate.
    Terminate,
    /// 'd' — CopyData (frontend → backend).
    CopyData(Vec<u8>),
    /// 'c' — CopyDone (frontend → backend).
    CopyDone,
    /// 'f' — CopyFail.
    CopyFail(String),
    /// 'p' — Password / SCRAM client-first / SCRAM client-final / SASL response.
    Password(Vec<u8>),
    /// 'F' — FunctionCall (deprecated fast-path).  We accept the on-wire form
    /// for protocol completeness; the dispatcher rejects calls with an
    /// ErrorResponse referring the client to a regular SELECT, since cave-pg
    /// does not expose an in-process function catalog.
    FunctionCall {
        function_oid: u32,
        arg_formats: Vec<i16>,
        args: Vec<Option<Vec<u8>>>,
        result_format: i16,
    },
}

// ── Backend messages ─────────────────────────────────────────────────────────

/// Messages flowing server → client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendMessage {
    Authentication(crate::wire::auth::AuthRequest),
    /// 'S' — ParameterStatus (server_version, client_encoding, …).
    ParameterStatus { name: String, value: String },
    /// 'K' — BackendKeyData.
    BackendKeyData { process_id: i32, secret_key: i32 },
    /// 'Z' — ReadyForQuery (txn status: 'I'dle, 'T'xn, 'E'rror).
    ReadyForQuery(u8),
    /// 'T' — RowDescription.
    RowDescription(Vec<FieldDescription>),
    /// 'D' — DataRow.
    DataRow(Vec<Option<Vec<u8>>>),
    /// 'C' — CommandComplete tag (e.g. "SELECT 1", "INSERT 0 3").
    CommandComplete(String),
    /// 'I' — EmptyQueryResponse.
    EmptyQueryResponse,
    /// 'E' — ErrorResponse (severity, code, message, detail, …).
    ErrorResponse(Vec<(u8, String)>),
    /// 'N' — NoticeResponse.
    NoticeResponse(Vec<(u8, String)>),
    /// '1' — ParseComplete.
    ParseComplete,
    /// '2' — BindComplete.
    BindComplete,
    /// '3' — CloseComplete.
    CloseComplete,
    /// 'n' — NoData (response to Describe of a statement that returns no rows).
    NoData,
    /// 's' — PortalSuspended.
    PortalSuspended,
    /// 't' — ParameterDescription (response to Describe statement).
    ParameterDescription(Vec<u32>),
    /// 'A' — NotificationResponse (LISTEN/NOTIFY).
    NotificationResponse {
        process_id: i32,
        channel: String,
        payload: String,
    },
    /// 'G' — CopyInResponse.
    CopyInResponse {
        overall_format: u8,
        column_formats: Vec<i16>,
    },
    /// 'H' — CopyOutResponse.
    CopyOutResponse {
        overall_format: u8,
        column_formats: Vec<i16>,
    },
    /// 'W' — CopyBothResponse (replication).
    CopyBothResponse {
        overall_format: u8,
        column_formats: Vec<i16>,
    },
    /// 'd' — CopyData (backend → frontend).
    CopyData(Vec<u8>),
    /// 'c' — CopyDone (backend → frontend).
    CopyDone,
    /// 'V' — FunctionCallResponse.  Single-value reply to a `FunctionCall`.
    /// Length -1 means SQL NULL.
    FunctionCallResponse(Option<Vec<u8>>),
    /// 'v' — NegotiateProtocolVersion.  Sent by the server when the client
    /// requests a higher minor protocol version than the server supports
    /// (libpq names it `v3.NegotiateProtocolVersion`).  Carries the maximum
    /// minor version the server actually speaks plus the names of any
    /// `_pq_.*` startup options the server did not understand.
    NegotiateProtocolVersion {
        newest_minor_supported: i32,
        unrecognized_options: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDescription {
    pub name: String,
    pub table_oid: u32,
    pub column_attr: i16,
    pub type_oid: u32,
    pub type_size: i16,
    pub type_modifier: i32,
    /// 0 = text, 1 = binary.
    pub format_code: i16,
}

// ── Encoding ─────────────────────────────────────────────────────────────────

pub fn encode_frontend(msg: &FrontendMessage, buf: &mut BytesMut) {
    use FrontendMessage::*;
    match msg {
        Startup(s) => crate::wire::startup::encode_startup(s, buf),
        SslRequest => {
            buf.put_i32(8);
            buf.put_i32(80877103);
        }
        GssEncRequest => {
            buf.put_i32(8);
            buf.put_i32(80877104);
        }
        Cancel {
            process_id,
            secret_key,
        } => {
            buf.put_i32(16);
            buf.put_i32(80877102);
            buf.put_i32(*process_id);
            buf.put_i32(*secret_key);
        }
        Query(q) => write_typed(b'Q', buf, |b| put_cstr(b, q)),
        Parse {
            statement_name,
            query,
            param_oids,
        } => write_typed(b'P', buf, |b| {
            put_cstr(b, statement_name);
            put_cstr(b, query);
            b.put_i16(param_oids.len() as i16);
            for oid in param_oids {
                b.put_u32(*oid);
            }
        }),
        Bind {
            portal_name,
            statement_name,
            param_formats,
            params,
            result_formats,
        } => write_typed(b'B', buf, |b| {
            put_cstr(b, portal_name);
            put_cstr(b, statement_name);
            b.put_i16(param_formats.len() as i16);
            for f in param_formats {
                b.put_i16(*f);
            }
            b.put_i16(params.len() as i16);
            for p in params {
                match p {
                    Some(v) => {
                        b.put_i32(v.len() as i32);
                        b.put_slice(v);
                    }
                    None => b.put_i32(-1),
                }
            }
            b.put_i16(result_formats.len() as i16);
            for f in result_formats {
                b.put_i16(*f);
            }
        }),
        Describe { kind, name } => write_typed(b'D', buf, |b| {
            b.put_u8(*kind);
            put_cstr(b, name);
        }),
        Execute {
            portal_name,
            max_rows,
        } => write_typed(b'E', buf, |b| {
            put_cstr(b, portal_name);
            b.put_i32(*max_rows);
        }),
        Close { kind, name } => write_typed(b'C', buf, |b| {
            b.put_u8(*kind);
            put_cstr(b, name);
        }),
        Sync => write_typed(b'S', buf, |_| {}),
        Flush => write_typed(b'H', buf, |_| {}),
        Terminate => write_typed(b'X', buf, |_| {}),
        CopyData(d) => write_typed(b'd', buf, |b| b.put_slice(d)),
        CopyDone => write_typed(b'c', buf, |_| {}),
        CopyFail(msg) => write_typed(b'f', buf, |b| put_cstr(b, msg)),
        Password(p) => write_typed(b'p', buf, |b| b.put_slice(p)),
        FunctionCall {
            function_oid,
            arg_formats,
            args,
            result_format,
        } => write_typed(b'F', buf, |b| {
            b.put_u32(*function_oid);
            b.put_i16(arg_formats.len() as i16);
            for f in arg_formats {
                b.put_i16(*f);
            }
            b.put_i16(args.len() as i16);
            for a in args {
                match a {
                    Some(v) => {
                        b.put_i32(v.len() as i32);
                        b.put_slice(v);
                    }
                    None => b.put_i32(-1),
                }
            }
            b.put_i16(*result_format);
        }),
    }
}

pub fn encode_backend(msg: &BackendMessage, buf: &mut BytesMut) {
    use BackendMessage::*;
    match msg {
        Authentication(a) => crate::wire::auth::encode(a, buf),
        ParameterStatus { name, value } => write_typed(b'S', buf, |b| {
            put_cstr(b, name);
            put_cstr(b, value);
        }),
        BackendKeyData {
            process_id,
            secret_key,
        } => write_typed(b'K', buf, |b| {
            b.put_i32(*process_id);
            b.put_i32(*secret_key);
        }),
        ReadyForQuery(s) => write_typed(b'Z', buf, |b| b.put_u8(*s)),
        RowDescription(fields) => write_typed(b'T', buf, |b| {
            b.put_i16(fields.len() as i16);
            for f in fields {
                put_cstr(b, &f.name);
                b.put_u32(f.table_oid);
                b.put_i16(f.column_attr);
                b.put_u32(f.type_oid);
                b.put_i16(f.type_size);
                b.put_i32(f.type_modifier);
                b.put_i16(f.format_code);
            }
        }),
        DataRow(cells) => write_typed(b'D', buf, |b| {
            b.put_i16(cells.len() as i16);
            for c in cells {
                match c {
                    Some(v) => {
                        b.put_i32(v.len() as i32);
                        b.put_slice(v);
                    }
                    None => b.put_i32(-1),
                }
            }
        }),
        CommandComplete(tag) => write_typed(b'C', buf, |b| put_cstr(b, tag)),
        EmptyQueryResponse => write_typed(b'I', buf, |_| {}),
        ErrorResponse(fields) => write_typed(b'E', buf, |b| {
            for (k, v) in fields {
                b.put_u8(*k);
                put_cstr(b, v);
            }
            b.put_u8(0);
        }),
        NoticeResponse(fields) => write_typed(b'N', buf, |b| {
            for (k, v) in fields {
                b.put_u8(*k);
                put_cstr(b, v);
            }
            b.put_u8(0);
        }),
        ParseComplete => write_typed(b'1', buf, |_| {}),
        BindComplete => write_typed(b'2', buf, |_| {}),
        CloseComplete => write_typed(b'3', buf, |_| {}),
        NoData => write_typed(b'n', buf, |_| {}),
        PortalSuspended => write_typed(b's', buf, |_| {}),
        ParameterDescription(oids) => write_typed(b't', buf, |b| {
            b.put_i16(oids.len() as i16);
            for o in oids {
                b.put_u32(*o);
            }
        }),
        NotificationResponse {
            process_id,
            channel,
            payload,
        } => write_typed(b'A', buf, |b| {
            b.put_i32(*process_id);
            put_cstr(b, channel);
            put_cstr(b, payload);
        }),
        CopyInResponse {
            overall_format,
            column_formats,
        }
        | CopyOutResponse {
            overall_format,
            column_formats,
        }
        | CopyBothResponse {
            overall_format,
            column_formats,
        } => {
            let tag = match msg {
                CopyInResponse { .. } => b'G',
                CopyOutResponse { .. } => b'H',
                CopyBothResponse { .. } => b'W',
                _ => unreachable!(),
            };
            write_typed(tag, buf, |b| {
                b.put_u8(*overall_format);
                b.put_i16(column_formats.len() as i16);
                for f in column_formats {
                    b.put_i16(*f);
                }
            });
        }
        CopyData(d) => write_typed(b'd', buf, |b| b.put_slice(d)),
        CopyDone => write_typed(b'c', buf, |_| {}),
        FunctionCallResponse(value) => write_typed(b'V', buf, |b| match value {
            Some(v) => {
                b.put_i32(v.len() as i32);
                b.put_slice(v);
            }
            None => b.put_i32(-1),
        }),
        NegotiateProtocolVersion {
            newest_minor_supported,
            unrecognized_options,
        } => write_typed(b'v', buf, |b| {
            b.put_i32(*newest_minor_supported);
            b.put_i32(unrecognized_options.len() as i32);
            for opt in unrecognized_options {
                put_cstr(b, opt);
            }
        }),
    }
}

// ── Decoding ─────────────────────────────────────────────────────────────────

/// Decode one frontend message from `buf`. On `Ok`, the consumed bytes are
/// removed from `buf` (advance happens via `BytesMut::advance` semantics).
pub fn decode_frontend(buf: &mut BytesMut, awaiting_startup: bool) -> ProtocolResult<FrontendMessage> {
    if awaiting_startup {
        return crate::wire::startup::decode_startup_frame(buf);
    }
    if buf.len() < 5 {
        return Err(ProtocolError::Incomplete {
            needed: 5 - buf.len(),
        });
    }
    let tag = buf[0];
    let len = i32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
    if !(4..=i32::MAX).contains(&len) {
        return Err(ProtocolError::InvalidLength(len));
    }
    let total = 1 + len as usize;
    if buf.len() < total {
        return Err(ProtocolError::Incomplete {
            needed: total - buf.len(),
        });
    }
    buf.advance(5);
    let mut payload = buf.split_to(len as usize - 4);
    let msg = match tag {
        b'Q' => FrontendMessage::Query(read_cstr(&mut payload)?),
        b'P' => {
            let statement_name = read_cstr(&mut payload)?;
            let query = read_cstr(&mut payload)?;
            let n = read_i16(&mut payload)?;
            let mut oids = Vec::with_capacity(n as usize);
            for _ in 0..n {
                oids.push(read_u32(&mut payload)?);
            }
            FrontendMessage::Parse {
                statement_name,
                query,
                param_oids: oids,
            }
        }
        b'B' => {
            let portal_name = read_cstr(&mut payload)?;
            let statement_name = read_cstr(&mut payload)?;
            let nf = read_i16(&mut payload)?;
            let mut param_formats = Vec::with_capacity(nf as usize);
            for _ in 0..nf {
                param_formats.push(read_i16(&mut payload)?);
            }
            let np = read_i16(&mut payload)?;
            let mut params = Vec::with_capacity(np as usize);
            for _ in 0..np {
                let len = read_i32(&mut payload)?;
                if len == -1 {
                    params.push(None);
                } else {
                    if len < 0 {
                        return Err(ProtocolError::InvalidValue {
                            field: "Bind.param_len",
                            reason: format!("{}", len),
                        });
                    }
                    let v = read_bytes(&mut payload, len as usize)?;
                    params.push(Some(v));
                }
            }
            let nr = read_i16(&mut payload)?;
            let mut result_formats = Vec::with_capacity(nr as usize);
            for _ in 0..nr {
                result_formats.push(read_i16(&mut payload)?);
            }
            FrontendMessage::Bind {
                portal_name,
                statement_name,
                param_formats,
                params,
                result_formats,
            }
        }
        b'D' => {
            let kind = read_u8(&mut payload)?;
            let name = read_cstr(&mut payload)?;
            FrontendMessage::Describe { kind, name }
        }
        b'E' => {
            let portal_name = read_cstr(&mut payload)?;
            let max_rows = read_i32(&mut payload)?;
            FrontendMessage::Execute {
                portal_name,
                max_rows,
            }
        }
        b'C' => {
            let kind = read_u8(&mut payload)?;
            let name = read_cstr(&mut payload)?;
            FrontendMessage::Close { kind, name }
        }
        b'S' => FrontendMessage::Sync,
        b'H' => FrontendMessage::Flush,
        b'X' => FrontendMessage::Terminate,
        b'd' => FrontendMessage::CopyData(payload.to_vec()),
        b'c' => FrontendMessage::CopyDone,
        b'f' => FrontendMessage::CopyFail(read_cstr(&mut payload)?),
        b'p' => FrontendMessage::Password(payload.to_vec()),
        b'F' => {
            let function_oid = read_u32(&mut payload)?;
            let nf = read_i16(&mut payload)?;
            let mut arg_formats = Vec::with_capacity(nf.max(0) as usize);
            for _ in 0..nf {
                arg_formats.push(read_i16(&mut payload)?);
            }
            let na = read_i16(&mut payload)?;
            let mut args = Vec::with_capacity(na.max(0) as usize);
            for _ in 0..na {
                let len = read_i32(&mut payload)?;
                if len == -1 {
                    args.push(None);
                } else if len < 0 {
                    return Err(ProtocolError::InvalidValue {
                        field: "FunctionCall.arg_len",
                        reason: format!("{}", len),
                    });
                } else {
                    args.push(Some(read_bytes(&mut payload, len as usize)?));
                }
            }
            let result_format = read_i16(&mut payload)?;
            FrontendMessage::FunctionCall {
                function_oid,
                arg_formats,
                args,
                result_format,
            }
        }
        other => return Err(ProtocolError::UnknownFrontendType(other)),
    };
    Ok(msg)
}

pub fn decode_backend(buf: &mut BytesMut) -> ProtocolResult<BackendMessage> {
    if buf.len() < 5 {
        return Err(ProtocolError::Incomplete {
            needed: 5 - buf.len(),
        });
    }
    let tag = buf[0];
    let len = i32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
    if !(4..=i32::MAX).contains(&len) {
        return Err(ProtocolError::InvalidLength(len));
    }
    let total = 1 + len as usize;
    if buf.len() < total {
        return Err(ProtocolError::Incomplete {
            needed: total - buf.len(),
        });
    }
    buf.advance(5);
    let mut payload = buf.split_to(len as usize - 4);
    let msg = match tag {
        b'R' => BackendMessage::Authentication(crate::wire::auth::decode(&mut payload)?),
        b'S' => {
            let name = read_cstr(&mut payload)?;
            let value = read_cstr(&mut payload)?;
            BackendMessage::ParameterStatus { name, value }
        }
        b'K' => {
            let process_id = read_i32(&mut payload)?;
            let secret_key = read_i32(&mut payload)?;
            BackendMessage::BackendKeyData {
                process_id,
                secret_key,
            }
        }
        b'Z' => BackendMessage::ReadyForQuery(read_u8(&mut payload)?),
        b'T' => {
            let n = read_i16(&mut payload)?;
            let mut fields = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let name = read_cstr(&mut payload)?;
                let table_oid = read_u32(&mut payload)?;
                let column_attr = read_i16(&mut payload)?;
                let type_oid = read_u32(&mut payload)?;
                let type_size = read_i16(&mut payload)?;
                let type_modifier = read_i32(&mut payload)?;
                let format_code = read_i16(&mut payload)?;
                fields.push(FieldDescription {
                    name,
                    table_oid,
                    column_attr,
                    type_oid,
                    type_size,
                    type_modifier,
                    format_code,
                });
            }
            BackendMessage::RowDescription(fields)
        }
        b'D' => {
            let n = read_i16(&mut payload)?;
            let mut cells = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let len = read_i32(&mut payload)?;
                if len == -1 {
                    cells.push(None);
                } else if len < 0 {
                    return Err(ProtocolError::InvalidValue {
                        field: "DataRow.cell_len",
                        reason: format!("{}", len),
                    });
                } else {
                    cells.push(Some(read_bytes(&mut payload, len as usize)?));
                }
            }
            BackendMessage::DataRow(cells)
        }
        b'C' => BackendMessage::CommandComplete(read_cstr(&mut payload)?),
        b'I' => BackendMessage::EmptyQueryResponse,
        b'E' | b'N' => {
            let mut fields = Vec::new();
            loop {
                let k = read_u8(&mut payload)?;
                if k == 0 {
                    break;
                }
                let v = read_cstr(&mut payload)?;
                fields.push((k, v));
            }
            if tag == b'E' {
                BackendMessage::ErrorResponse(fields)
            } else {
                BackendMessage::NoticeResponse(fields)
            }
        }
        b'1' => BackendMessage::ParseComplete,
        b'2' => BackendMessage::BindComplete,
        b'3' => BackendMessage::CloseComplete,
        b'n' => BackendMessage::NoData,
        b's' => BackendMessage::PortalSuspended,
        b't' => {
            let n = read_i16(&mut payload)?;
            let mut oids = Vec::with_capacity(n as usize);
            for _ in 0..n {
                oids.push(read_u32(&mut payload)?);
            }
            BackendMessage::ParameterDescription(oids)
        }
        b'A' => {
            let process_id = read_i32(&mut payload)?;
            let channel = read_cstr(&mut payload)?;
            let payload_str = read_cstr(&mut payload)?;
            BackendMessage::NotificationResponse {
                process_id,
                channel,
                payload: payload_str,
            }
        }
        b'G' | b'H' | b'W' => {
            let overall_format = read_u8(&mut payload)?;
            let n = read_i16(&mut payload)?;
            let mut column_formats = Vec::with_capacity(n as usize);
            for _ in 0..n {
                column_formats.push(read_i16(&mut payload)?);
            }
            match tag {
                b'G' => BackendMessage::CopyInResponse {
                    overall_format,
                    column_formats,
                },
                b'H' => BackendMessage::CopyOutResponse {
                    overall_format,
                    column_formats,
                },
                _ => BackendMessage::CopyBothResponse {
                    overall_format,
                    column_formats,
                },
            }
        }
        b'd' => BackendMessage::CopyData(payload.to_vec()),
        b'c' => BackendMessage::CopyDone,
        b'V' => {
            let len = read_i32(&mut payload)?;
            if len == -1 {
                BackendMessage::FunctionCallResponse(None)
            } else if len < 0 {
                return Err(ProtocolError::InvalidValue {
                    field: "FunctionCallResponse.len",
                    reason: format!("{}", len),
                });
            } else {
                BackendMessage::FunctionCallResponse(Some(read_bytes(&mut payload, len as usize)?))
            }
        }
        b'v' => {
            let newest_minor_supported = read_i32(&mut payload)?;
            let n = read_i32(&mut payload)?;
            if n < 0 {
                return Err(ProtocolError::InvalidValue {
                    field: "NegotiateProtocolVersion.option_count",
                    reason: format!("{}", n),
                });
            }
            let mut opts = Vec::with_capacity(n as usize);
            for _ in 0..n {
                opts.push(read_cstr(&mut payload)?);
            }
            BackendMessage::NegotiateProtocolVersion {
                newest_minor_supported,
                unrecognized_options: opts,
            }
        }
        other => return Err(ProtocolError::UnknownBackendType(other)),
    };
    Ok(msg)
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn write_typed<F: FnOnce(&mut BytesMut)>(tag: u8, buf: &mut BytesMut, f: F) {
    buf.put_u8(tag);
    let len_pos = buf.len();
    buf.put_i32(0); // placeholder
    f(buf);
    let payload_len = buf.len() - len_pos;
    let bytes = (payload_len as i32).to_be_bytes();
    buf[len_pos..len_pos + 4].copy_from_slice(&bytes);
}

pub(crate) fn put_cstr(buf: &mut BytesMut, s: &str) {
    buf.put_slice(s.as_bytes());
    buf.put_u8(0);
}

pub(crate) fn read_cstr(buf: &mut BytesMut) -> ProtocolResult<String> {
    let nul = buf
        .iter()
        .position(|&b| b == 0)
        .ok_or(ProtocolError::MissingNul)?;
    let s = std::str::from_utf8(&buf[..nul])
        .map_err(|e| ProtocolError::Utf8(e.to_string()))?
        .to_owned();
    buf.advance(nul + 1);
    Ok(s)
}

pub(crate) fn read_u8(buf: &mut BytesMut) -> ProtocolResult<u8> {
    if buf.is_empty() {
        return Err(ProtocolError::Incomplete { needed: 1 });
    }
    Ok(buf.get_u8())
}

pub(crate) fn read_i16(buf: &mut BytesMut) -> ProtocolResult<i16> {
    if buf.len() < 2 {
        return Err(ProtocolError::Incomplete {
            needed: 2 - buf.len(),
        });
    }
    Ok(buf.get_i16())
}

pub(crate) fn read_i32(buf: &mut BytesMut) -> ProtocolResult<i32> {
    if buf.len() < 4 {
        return Err(ProtocolError::Incomplete {
            needed: 4 - buf.len(),
        });
    }
    Ok(buf.get_i32())
}

pub(crate) fn read_u32(buf: &mut BytesMut) -> ProtocolResult<u32> {
    if buf.len() < 4 {
        return Err(ProtocolError::Incomplete {
            needed: 4 - buf.len(),
        });
    }
    Ok(buf.get_u32())
}

pub(crate) fn read_bytes(buf: &mut BytesMut, n: usize) -> ProtocolResult<Vec<u8>> {
    if buf.len() < n {
        return Err(ProtocolError::Incomplete {
            needed: n - buf.len(),
        });
    }
    let v = buf[..n].to_vec();
    buf.advance(n);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::auth::AuthRequest;

    fn rt_frontend(msg: FrontendMessage) {
        let mut buf = BytesMut::new();
        encode_frontend(&msg, &mut buf);
        let decoded = decode_frontend(&mut buf, false).unwrap();
        assert_eq!(msg, decoded);
        assert!(buf.is_empty());
    }
    fn rt_backend(msg: BackendMessage) {
        let mut buf = BytesMut::new();
        encode_backend(&msg, &mut buf);
        let decoded = decode_backend(&mut buf).unwrap();
        assert_eq!(msg, decoded);
        assert!(buf.is_empty());
    }

    #[test]
    fn query_round_trip() {
        rt_frontend(FrontendMessage::Query("SELECT 1".into()));
    }
    #[test]
    fn query_empty() {
        rt_frontend(FrontendMessage::Query(String::new()));
    }
    #[test]
    fn query_unicode() {
        rt_frontend(FrontendMessage::Query("SELECT 'héllo'".into()));
    }
    #[test]
    fn parse_no_params() {
        rt_frontend(FrontendMessage::Parse {
            statement_name: "s1".into(),
            query: "SELECT 1".into(),
            param_oids: vec![],
        });
    }
    #[test]
    fn parse_with_params() {
        rt_frontend(FrontendMessage::Parse {
            statement_name: String::new(),
            query: "SELECT $1::int4, $2::text".into(),
            param_oids: vec![23, 25],
        });
    }
    #[test]
    fn bind_text_format() {
        rt_frontend(FrontendMessage::Bind {
            portal_name: "p1".into(),
            statement_name: "s1".into(),
            param_formats: vec![0, 0],
            params: vec![Some(b"42".to_vec()), Some(b"hello".to_vec())],
            result_formats: vec![0],
        });
    }
    #[test]
    fn bind_with_null() {
        rt_frontend(FrontendMessage::Bind {
            portal_name: String::new(),
            statement_name: String::new(),
            param_formats: vec![],
            params: vec![None, Some(b"x".to_vec())],
            result_formats: vec![],
        });
    }
    #[test]
    fn describe_statement() {
        rt_frontend(FrontendMessage::Describe {
            kind: b'S',
            name: "s1".into(),
        });
    }
    #[test]
    fn describe_portal() {
        rt_frontend(FrontendMessage::Describe {
            kind: b'P',
            name: "p1".into(),
        });
    }
    #[test]
    fn execute_unlimited() {
        rt_frontend(FrontendMessage::Execute {
            portal_name: "p1".into(),
            max_rows: 0,
        });
    }
    #[test]
    fn execute_limited() {
        rt_frontend(FrontendMessage::Execute {
            portal_name: String::new(),
            max_rows: 100,
        });
    }
    #[test]
    fn close_statement() {
        rt_frontend(FrontendMessage::Close {
            kind: b'S',
            name: "s1".into(),
        });
    }
    #[test]
    fn sync_msg() {
        rt_frontend(FrontendMessage::Sync);
    }
    #[test]
    fn flush_msg() {
        rt_frontend(FrontendMessage::Flush);
    }
    #[test]
    fn terminate_msg() {
        rt_frontend(FrontendMessage::Terminate);
    }
    #[test]
    fn copy_data_round_trip() {
        rt_frontend(FrontendMessage::CopyData(b"row\tdata\n".to_vec()));
    }
    #[test]
    fn copy_done_round_trip() {
        rt_frontend(FrontendMessage::CopyDone);
    }
    #[test]
    fn copy_fail_round_trip() {
        rt_frontend(FrontendMessage::CopyFail("client gave up".into()));
    }
    #[test]
    fn password_round_trip() {
        rt_frontend(FrontendMessage::Password(b"hunter2\0".to_vec()));
    }
    #[test]
    fn ssl_request_no_type_byte() {
        let mut buf = BytesMut::new();
        encode_frontend(&FrontendMessage::SslRequest, &mut buf);
        // 8 byte length + 80877103
        assert_eq!(buf.len(), 8);
        let decoded = decode_frontend(&mut buf, true).unwrap();
        assert_eq!(decoded, FrontendMessage::SslRequest);
    }
    #[test]
    fn cancel_request_round_trip() {
        let msg = FrontendMessage::Cancel {
            process_id: 12345,
            secret_key: 67890,
        };
        let mut buf = BytesMut::new();
        encode_frontend(&msg, &mut buf);
        assert_eq!(buf.len(), 16);
        let decoded = decode_frontend(&mut buf, true).unwrap();
        assert_eq!(decoded, msg);
    }
    #[test]
    fn gss_enc_request_round_trip() {
        let mut buf = BytesMut::new();
        encode_frontend(&FrontendMessage::GssEncRequest, &mut buf);
        assert_eq!(buf.len(), 8);
        let decoded = decode_frontend(&mut buf, true).unwrap();
        assert_eq!(decoded, FrontendMessage::GssEncRequest);
    }
    #[test]
    fn incomplete_frame_signals_need() {
        let mut buf = BytesMut::from(&b"Q\0\0"[..]);
        let err = decode_frontend(&mut buf, false).unwrap_err();
        match err {
            ProtocolError::Incomplete { needed } => assert!(needed >= 1),
            other => panic!("expected Incomplete, got {:?}", other),
        }
    }
    #[test]
    fn unknown_frontend_type_errors() {
        let mut buf = BytesMut::new();
        buf.put_u8(b'Z');
        buf.put_i32(4);
        let err = decode_frontend(&mut buf, false).unwrap_err();
        assert_eq!(err, ProtocolError::UnknownFrontendType(b'Z'));
    }
    #[test]
    fn invalid_length_errors() {
        let mut buf = BytesMut::new();
        buf.put_u8(b'Q');
        buf.put_i32(-1);
        let err = decode_frontend(&mut buf, false).unwrap_err();
        assert_eq!(err, ProtocolError::InvalidLength(-1));
    }
    #[test]
    fn cstring_missing_nul_errors() {
        let mut payload = BytesMut::from(&b"no nul here"[..]);
        let err = read_cstr(&mut payload).unwrap_err();
        assert_eq!(err, ProtocolError::MissingNul);
    }
    #[test]
    fn cstring_invalid_utf8_errors() {
        let mut payload = BytesMut::from(&[0xFF, 0xFE, 0][..]);
        let err = read_cstr(&mut payload).unwrap_err();
        matches!(err, ProtocolError::Utf8(_));
    }
    // backend round-trips
    #[test]
    fn parameter_status_round_trip() {
        rt_backend(BackendMessage::ParameterStatus {
            name: "server_version".into(),
            value: "16.2 (cave-pg)".into(),
        });
    }
    #[test]
    fn backend_key_data_round_trip() {
        rt_backend(BackendMessage::BackendKeyData {
            process_id: 4711,
            secret_key: -1,
        });
    }
    #[test]
    fn ready_for_query_idle() {
        rt_backend(BackendMessage::ReadyForQuery(b'I'));
    }
    #[test]
    fn ready_for_query_in_txn() {
        rt_backend(BackendMessage::ReadyForQuery(b'T'));
    }
    #[test]
    fn ready_for_query_failed_txn() {
        rt_backend(BackendMessage::ReadyForQuery(b'E'));
    }
    #[test]
    fn row_description_one_field() {
        rt_backend(BackendMessage::RowDescription(vec![FieldDescription {
            name: "id".into(),
            table_oid: 16384,
            column_attr: 1,
            type_oid: 23,
            type_size: 4,
            type_modifier: -1,
            format_code: 0,
        }]));
    }
    #[test]
    fn row_description_multi_field() {
        rt_backend(BackendMessage::RowDescription(vec![
            FieldDescription {
                name: "id".into(),
                table_oid: 16384,
                column_attr: 1,
                type_oid: 23,
                type_size: 4,
                type_modifier: -1,
                format_code: 0,
            },
            FieldDescription {
                name: "name".into(),
                table_oid: 16384,
                column_attr: 2,
                type_oid: 25,
                type_size: -1,
                type_modifier: -1,
                format_code: 0,
            },
        ]));
    }
    #[test]
    fn data_row_with_null() {
        rt_backend(BackendMessage::DataRow(vec![
            Some(b"42".to_vec()),
            None,
            Some(b"hello".to_vec()),
        ]));
    }
    #[test]
    fn data_row_empty() {
        rt_backend(BackendMessage::DataRow(vec![]));
    }
    #[test]
    fn command_complete_round_trip() {
        rt_backend(BackendMessage::CommandComplete("SELECT 1".into()));
    }
    #[test]
    fn empty_query_response_round_trip() {
        rt_backend(BackendMessage::EmptyQueryResponse);
    }
    #[test]
    fn error_response_round_trip() {
        rt_backend(BackendMessage::ErrorResponse(vec![
            (b'S', "ERROR".into()),
            (b'C', "42P01".into()),
            (b'M', "relation \"x\" does not exist".into()),
        ]));
    }
    #[test]
    fn notice_response_round_trip() {
        rt_backend(BackendMessage::NoticeResponse(vec![
            (b'S', "NOTICE".into()),
            (b'M', "auto-vacuum starting".into()),
        ]));
    }
    #[test]
    fn parse_complete_round_trip() {
        rt_backend(BackendMessage::ParseComplete);
    }
    #[test]
    fn bind_complete_round_trip() {
        rt_backend(BackendMessage::BindComplete);
    }
    #[test]
    fn close_complete_round_trip() {
        rt_backend(BackendMessage::CloseComplete);
    }
    #[test]
    fn no_data_round_trip() {
        rt_backend(BackendMessage::NoData);
    }
    #[test]
    fn portal_suspended_round_trip() {
        rt_backend(BackendMessage::PortalSuspended);
    }
    #[test]
    fn parameter_description_round_trip() {
        rt_backend(BackendMessage::ParameterDescription(vec![23, 25, 1700]));
    }
    #[test]
    fn notification_response_round_trip() {
        rt_backend(BackendMessage::NotificationResponse {
            process_id: 4711,
            channel: "ch".into(),
            payload: "hi".into(),
        });
    }
    #[test]
    fn copy_in_response_round_trip() {
        rt_backend(BackendMessage::CopyInResponse {
            overall_format: 0,
            column_formats: vec![0, 0],
        });
    }
    #[test]
    fn copy_out_response_round_trip() {
        rt_backend(BackendMessage::CopyOutResponse {
            overall_format: 1,
            column_formats: vec![1, 1, 1],
        });
    }
    #[test]
    fn copy_both_response_round_trip() {
        rt_backend(BackendMessage::CopyBothResponse {
            overall_format: 0,
            column_formats: vec![0],
        });
    }
    #[test]
    fn copy_data_backend_round_trip() {
        rt_backend(BackendMessage::CopyData(vec![1, 2, 3, 4, 5]));
    }
    #[test]
    fn auth_ok_round_trip() {
        rt_backend(BackendMessage::Authentication(AuthRequest::Ok));
    }
    #[test]
    fn auth_md5_round_trip() {
        rt_backend(BackendMessage::Authentication(AuthRequest::Md5Password {
            salt: [1, 2, 3, 4],
        }));
    }
    #[test]
    fn auth_sasl_round_trip() {
        rt_backend(BackendMessage::Authentication(AuthRequest::Sasl(vec![
            "SCRAM-SHA-256".into(),
            "SCRAM-SHA-256-PLUS".into(),
        ])));
    }
    #[test]
    fn frame_length_matches_payload_query() {
        let mut buf = BytesMut::new();
        encode_frontend(&FrontendMessage::Query("hi".into()), &mut buf);
        // Q + i32 + "hi\0" = 1 + 4 + 3 = 8; length field = 4 + 3 = 7.
        assert_eq!(buf[0], b'Q');
        let len = i32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
        assert_eq!(len, 7);
        assert_eq!(buf.len(), 8);
    }
    #[test]
    fn unknown_backend_type_errors() {
        let mut buf = BytesMut::new();
        buf.put_u8(b'?');
        buf.put_i32(4);
        let err = decode_backend(&mut buf).unwrap_err();
        assert_eq!(err, ProtocolError::UnknownBackendType(b'?'));
    }
    #[test]
    fn back_to_back_frames() {
        let mut buf = BytesMut::new();
        encode_frontend(&FrontendMessage::Query("a".into()), &mut buf);
        encode_frontend(&FrontendMessage::Sync, &mut buf);
        encode_frontend(&FrontendMessage::Terminate, &mut buf);
        let m1 = decode_frontend(&mut buf, false).unwrap();
        let m2 = decode_frontend(&mut buf, false).unwrap();
        let m3 = decode_frontend(&mut buf, false).unwrap();
        assert_eq!(m1, FrontendMessage::Query("a".into()));
        assert_eq!(m2, FrontendMessage::Sync);
        assert_eq!(m3, FrontendMessage::Terminate);
        assert!(buf.is_empty());
    }

    #[test]
    fn function_call_round_trip() {
        rt_frontend(FrontendMessage::FunctionCall {
            function_oid: 1574, // pg_proc.oid for nextval
            arg_formats: vec![0],
            args: vec![Some(b"my_seq".to_vec())],
            result_format: 0,
        });
    }
    #[test]
    fn function_call_with_null_arg() {
        rt_frontend(FrontendMessage::FunctionCall {
            function_oid: 42,
            arg_formats: vec![1, 1],
            args: vec![None, Some(vec![0xde, 0xad, 0xbe, 0xef])],
            result_format: 1,
        });
    }
    #[test]
    fn function_call_response_some() {
        rt_backend(BackendMessage::FunctionCallResponse(Some(b"42".to_vec())));
    }
    #[test]
    fn function_call_response_null() {
        rt_backend(BackendMessage::FunctionCallResponse(None));
    }
    #[test]
    fn negotiate_protocol_version_no_options() {
        rt_backend(BackendMessage::NegotiateProtocolVersion {
            newest_minor_supported: 0,
            unrecognized_options: vec![],
        });
    }
    #[test]
    fn negotiate_protocol_version_with_options() {
        rt_backend(BackendMessage::NegotiateProtocolVersion {
            newest_minor_supported: 2,
            unrecognized_options: vec!["_pq_.future_feature".into(), "_pq_.unknown".into()],
        });
    }
}
