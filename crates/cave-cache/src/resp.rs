// SPDX-License-Identifier: AGPL-3.0-or-later
//! RESP3 protocol serializer/deserializer.
//!
//! Supports both RESP2 (inline/array) and RESP3 (all types).
//! Reference: https://redis.io/docs/reference/protocol-spec/

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::error::{CacheError, CacheResult};

/// RESP value (covers both RESP2 and RESP3 types).
#[derive(Debug, Clone, PartialEq)]
pub enum Resp {
    // RESP2 types
    SimpleString(Vec<u8>),
    Error(String),
    Integer(i64),
    BulkString(Option<Vec<u8>>), // None = null bulk string
    Array(Option<Vec<Resp>>),    // None = null array

    // RESP3 additional types
    Null,
    Double(f64),
    Boolean(bool),
    BigNumber(Vec<u8>),
    BlobError(Vec<u8>, Vec<u8>),
    VerbatimString(Vec<u8>, Vec<u8>), // (encoding, data)
    Map(Vec<(Resp, Resp)>),
    Set(Vec<Resp>),
    Push(Vec<Resp>),
}

impl Resp {
    pub fn ok() -> Self {
        Resp::SimpleString(b"OK".to_vec())
    }

    pub fn queued() -> Self {
        Resp::SimpleString(b"QUEUED".to_vec())
    }

    pub fn pong() -> Self {
        Resp::SimpleString(b"PONG".to_vec())
    }

    pub fn nil() -> Self {
        Resp::BulkString(None)
    }

    pub fn nil_array() -> Self {
        Resp::Array(None)
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Resp::Error(msg.into())
    }

    pub fn from_error(e: &CacheError) -> Self {
        Resp::Error(e.to_resp_error())
    }

    pub fn bulk(b: Vec<u8>) -> Self {
        Resp::BulkString(Some(b))
    }

    pub fn int(n: i64) -> Self {
        Resp::Integer(n)
    }

    pub fn array(items: Vec<Resp>) -> Self {
        Resp::Array(Some(items))
    }

    pub fn empty_array() -> Self {
        Resp::Array(Some(vec![]))
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Resp::BulkString(None) | Resp::Array(None) | Resp::Null)
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

pub type Reader = BufReader<OwnedReadHalf>;

/// Parse a single RESP value from the reader.
pub async fn parse_resp(reader: &mut Reader) -> CacheResult<Resp> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).await.map_err(|_| CacheError::Io)?;
    if n == 0 {
        return Err(CacheError::Protocol("Connection closed".into()));
    }
    let line = line.trim_end_matches(|c| c == '\r' || c == '\n');
    if line.is_empty() {
        return Err(CacheError::Protocol("Empty line".into()));
    }

    let prefix = line.chars().next().unwrap();
    let rest = &line[1..];

    match prefix {
        '+' => Ok(Resp::SimpleString(rest.as_bytes().to_vec())),
        '-' => Ok(Resp::Error(rest.to_string())),
        ':' => {
            let n: i64 = rest.parse().map_err(|_| CacheError::Protocol("Invalid integer".into()))?;
            Ok(Resp::Integer(n))
        }
        '$' => {
            let len: i64 = rest.parse().map_err(|_| CacheError::Protocol("Invalid bulk len".into()))?;
            if len == -1 {
                return Ok(Resp::BulkString(None));
            }
            if len < 0 {
                return Err(CacheError::Protocol("Invalid bulk length".into()));
            }
            let len = len as usize;
            let mut buf = vec![0u8; len + 2]; // +2 for \r\n
            reader.read_exact(&mut buf).await.map_err(|_| CacheError::Io)?;
            buf.truncate(len);
            Ok(Resp::BulkString(Some(buf)))
        }
        '*' => {
            let count: i64 = rest.parse().map_err(|_| CacheError::Protocol("Invalid array len".into()))?;
            if count == -1 {
                return Ok(Resp::Array(None));
            }
            if count < 0 {
                return Err(CacheError::Protocol("Invalid array length".into()));
            }
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                items.push(Box::pin(parse_resp(reader)).await?);
            }
            Ok(Resp::Array(Some(items)))
        }
        // RESP3
        '_' => Ok(Resp::Null),
        '#' => match rest {
            "t" => Ok(Resp::Boolean(true)),
            "f" => Ok(Resp::Boolean(false)),
            _ => Err(CacheError::Protocol("Invalid boolean".into())),
        },
        ',' => {
            let f: f64 = match rest {
                "inf" | "+inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                _ => rest.parse().map_err(|_| CacheError::Protocol("Invalid double".into()))?,
            };
            Ok(Resp::Double(f))
        }
        '(' => Ok(Resp::BigNumber(rest.as_bytes().to_vec())),
        '!' => {
            let len: usize = rest.parse().map_err(|_| CacheError::Protocol("Invalid blob error len".into()))?;
            let mut buf = vec![0u8; len + 2];
            reader.read_exact(&mut buf).await.map_err(|_| CacheError::Io)?;
            buf.truncate(len);
            // format: "CODE message"
            let sep = buf.iter().position(|&b| b == b' ').unwrap_or(buf.len());
            Ok(Resp::BlobError(buf[..sep].to_vec(), buf[sep..].to_vec()))
        }
        '=' => {
            let len: usize = rest.parse().map_err(|_| CacheError::Protocol("Invalid verbatim len".into()))?;
            let mut buf = vec![0u8; len + 2];
            reader.read_exact(&mut buf).await.map_err(|_| CacheError::Io)?;
            buf.truncate(len);
            // first 3 bytes = encoding, byte 4 = ':', rest = data
            if buf.len() < 4 {
                return Err(CacheError::Protocol("Verbatim too short".into()));
            }
            Ok(Resp::VerbatimString(buf[..3].to_vec(), buf[4..].to_vec()))
        }
        '%' => {
            let count: usize = rest.parse().map_err(|_| CacheError::Protocol("Invalid map len".into()))?;
            let mut pairs = Vec::with_capacity(count);
            for _ in 0..count {
                let k = Box::pin(parse_resp(reader)).await?;
                let v = Box::pin(parse_resp(reader)).await?;
                pairs.push((k, v));
            }
            Ok(Resp::Map(pairs))
        }
        '~' => {
            let count: usize = rest.parse().map_err(|_| CacheError::Protocol("Invalid set len".into()))?;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(Box::pin(parse_resp(reader)).await?);
            }
            Ok(Resp::Set(items))
        }
        '>' => {
            let count: usize = rest.parse().map_err(|_| CacheError::Protocol("Invalid push len".into()))?;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(Box::pin(parse_resp(reader)).await?);
            }
            Ok(Resp::Push(items))
        }
        _ => Err(CacheError::Protocol(format!("Unknown RESP prefix: {:?}", prefix))),
    }
}

/// Parse a RESP command into a list of byte-slices.
/// Returns the raw args (first element is the command name).
pub async fn parse_command(reader: &mut Reader) -> CacheResult<Vec<Vec<u8>>> {
    let value = parse_resp(reader).await?;
    match value {
        Resp::Array(Some(items)) => {
            let mut args = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Resp::BulkString(Some(b)) => args.push(b),
                    Resp::SimpleString(b) => args.push(b),
                    _ => return Err(CacheError::Protocol("Expected bulk string in command array".into())),
                }
            }
            if args.is_empty() {
                return Err(CacheError::Protocol("Empty command".into()));
            }
            Ok(args)
        }
        // Inline commands (for redis-cli compatibility)
        Resp::SimpleString(s) => {
            parse_inline(&s)
        }
        _ => Err(CacheError::Protocol("Expected array for command".into())),
    }
}

fn parse_inline(line: &[u8]) -> CacheResult<Vec<Vec<u8>>> {
    let s = std::str::from_utf8(line).map_err(|_| CacheError::Protocol("Non-UTF8 inline".into()))?;
    let args: Vec<Vec<u8>> = s.split_whitespace().map(|t| t.as_bytes().to_vec()).collect();
    if args.is_empty() {
        return Err(CacheError::Protocol("Empty inline command".into()));
    }
    Ok(args)
}

// ── Serializer ────────────────────────────────────────────────────────────────

pub async fn write_resp(writer: &mut OwnedWriteHalf, resp: &Resp) -> CacheResult<()> {
    let mut buf = Vec::new();
    encode_resp(&mut buf, resp);
    writer.write_all(&buf).await.map_err(|_| CacheError::Io)?;
    Ok(())
}

pub fn encode_resp(buf: &mut Vec<u8>, resp: &Resp) {
    match resp {
        Resp::SimpleString(s) => {
            buf.push(b'+');
            buf.extend_from_slice(s);
            buf.extend_from_slice(b"\r\n");
        }
        Resp::Error(e) => {
            buf.push(b'-');
            buf.extend_from_slice(e.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        Resp::Integer(n) => {
            buf.push(b':');
            buf.extend_from_slice(n.to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        Resp::BulkString(None) => {
            buf.extend_from_slice(b"$-1\r\n");
        }
        Resp::BulkString(Some(s)) => {
            buf.push(b'$');
            buf.extend_from_slice(s.len().to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            buf.extend_from_slice(s);
            buf.extend_from_slice(b"\r\n");
        }
        Resp::Array(None) => {
            buf.extend_from_slice(b"*-1\r\n");
        }
        Resp::Array(Some(items)) => {
            buf.push(b'*');
            buf.extend_from_slice(items.len().to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            for item in items {
                encode_resp(buf, item);
            }
        }
        Resp::Null => {
            buf.extend_from_slice(b"_\r\n");
        }
        Resp::Boolean(b) => {
            buf.extend_from_slice(if *b { b"#t\r\n" } else { b"#f\r\n" });
        }
        Resp::Double(f) => {
            buf.push(b',');
            let s = if f.is_infinite() && *f > 0.0 { "inf".to_string() }
                    else if f.is_infinite() { "-inf".to_string() }
                    else { format!("{}", f) };
            buf.extend_from_slice(s.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        Resp::BigNumber(n) => {
            buf.push(b'(');
            buf.extend_from_slice(n);
            buf.extend_from_slice(b"\r\n");
        }
        Resp::BlobError(code, msg) => {
            let content: Vec<u8> = code.iter().chain(b" ".iter()).chain(msg.iter()).copied().collect();
            buf.push(b'!');
            buf.extend_from_slice(content.len().to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            buf.extend_from_slice(&content);
            buf.extend_from_slice(b"\r\n");
        }
        Resp::VerbatimString(enc, data) => {
            let total = 3 + 1 + data.len(); // enc:data
            buf.push(b'=');
            buf.extend_from_slice(total.to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            buf.extend_from_slice(enc);
            buf.push(b':');
            buf.extend_from_slice(data);
            buf.extend_from_slice(b"\r\n");
        }
        Resp::Map(pairs) => {
            buf.push(b'%');
            buf.extend_from_slice(pairs.len().to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            for (k, v) in pairs {
                encode_resp(buf, k);
                encode_resp(buf, v);
            }
        }
        Resp::Set(items) => {
            buf.push(b'~');
            buf.extend_from_slice(items.len().to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            for item in items {
                encode_resp(buf, item);
            }
        }
        Resp::Push(items) => {
            buf.push(b'>');
            buf.extend_from_slice(items.len().to_string().as_bytes());
            buf.extend_from_slice(b"\r\n");
            for item in items {
                encode_resp(buf, item);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Encode a RESP2 response as bytes (for use in tests / non-async contexts).
pub fn encode_resp2(resp: &Resp) -> Vec<u8> {
    let mut buf = Vec::new();
    encode_resp(&mut buf, resp);
    buf
}
