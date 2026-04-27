//! RESP2 + RESP3 sans-io codec.
//!
//! Wire types ([`RespValue`]):
//!
//! | prefix | RESP2 / RESP3 type             |
//! |--------|--------------------------------|
//! | `+`    | SimpleString                   |
//! | `-`    | Error                          |
//! | `:`    | Integer                        |
//! | `$`    | BulkString (`$-1\r\n` = nil)   |
//! | `*`    | Array     (`*-1\r\n` = nil)    |
//! | `_`    | Null (RESP3)                   |
//! | `#`    | Boolean (RESP3)                |
//! | `,`    | Double  (RESP3)                |
//! | `(`    | BigNumber (RESP3)              |
//! | `=`    | VerbatimString (RESP3)         |
//! | `%`    | Map (RESP3)                    |
//! | `~`    | Set (RESP3)                    |
//! | `>`    | Push (RESP3)                   |
//!
//! Reference: <https://redis.io/docs/reference/protocol-spec/>.
//!
//! Decoding is **partial**: when there are not enough bytes the function
//! returns `Err(RespCodecError::Incomplete)` and the input buffer is left
//! untouched (the byte cursor is restored on every Incomplete branch).

use bytes::{Buf, BufMut, BytesMut};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RespCodecError {
    #[error("incomplete frame")]
    Incomplete,
    #[error("invalid prefix: 0x{0:02x}")]
    InvalidPrefix(u8),
    #[error("invalid integer: {0}")]
    InvalidInteger(String),
    #[error("invalid double: {0}")]
    InvalidDouble(String),
    #[error("invalid utf-8: {0}")]
    InvalidUtf8(String),
    #[error("invalid length: {0}")]
    InvalidLength(i64),
    #[error("invalid map count: odd entries")]
    InvalidMapCount,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Option<Vec<u8>>),
    Array(Option<Vec<RespValue>>),
    Null,
    Boolean(bool),
    Double(f64),
    BigNumber(String),
    VerbatimString { encoding: String, data: Vec<u8> },
    Map(Vec<(RespValue, RespValue)>),
    Set(Vec<RespValue>),
    Push(Vec<RespValue>),
}

impl RespValue {
    pub fn ok() -> Self {
        RespValue::SimpleString("OK".into())
    }
    pub fn pong() -> Self {
        RespValue::SimpleString("PONG".into())
    }
    pub fn nil_bulk() -> Self {
        RespValue::BulkString(None)
    }
    pub fn nil_array() -> Self {
        RespValue::Array(None)
    }
    pub fn bulk(data: impl Into<Vec<u8>>) -> Self {
        RespValue::BulkString(Some(data.into()))
    }
    pub fn array(items: Vec<RespValue>) -> Self {
        RespValue::Array(Some(items))
    }
    pub fn empty_array() -> Self {
        RespValue::Array(Some(vec![]))
    }
}

// ── Encoder ─────────────────────────────────────────────────────────────────

pub fn encode(value: &RespValue, buf: &mut BytesMut) {
    match value {
        RespValue::SimpleString(s) => {
            buf.put_u8(b'+');
            buf.put_slice(s.as_bytes());
            buf.put_slice(b"\r\n");
        }
        RespValue::Error(s) => {
            buf.put_u8(b'-');
            buf.put_slice(s.as_bytes());
            buf.put_slice(b"\r\n");
        }
        RespValue::Integer(n) => {
            buf.put_u8(b':');
            put_int(*n, buf);
            buf.put_slice(b"\r\n");
        }
        RespValue::BulkString(None) => buf.put_slice(b"$-1\r\n"),
        RespValue::BulkString(Some(data)) => {
            buf.put_u8(b'$');
            put_int(data.len() as i64, buf);
            buf.put_slice(b"\r\n");
            buf.put_slice(data);
            buf.put_slice(b"\r\n");
        }
        RespValue::Array(None) => buf.put_slice(b"*-1\r\n"),
        RespValue::Array(Some(items)) => {
            buf.put_u8(b'*');
            put_int(items.len() as i64, buf);
            buf.put_slice(b"\r\n");
            for it in items {
                encode(it, buf);
            }
        }
        RespValue::Null => buf.put_slice(b"_\r\n"),
        RespValue::Boolean(b) => {
            buf.put_u8(b'#');
            buf.put_u8(if *b { b't' } else { b'f' });
            buf.put_slice(b"\r\n");
        }
        RespValue::Double(d) => {
            buf.put_u8(b',');
            if d.is_infinite() {
                if *d > 0.0 {
                    buf.put_slice(b"inf");
                } else {
                    buf.put_slice(b"-inf");
                }
            } else if d.is_nan() {
                buf.put_slice(b"nan");
            } else {
                buf.put_slice(format!("{}", d).as_bytes());
            }
            buf.put_slice(b"\r\n");
        }
        RespValue::BigNumber(s) => {
            buf.put_u8(b'(');
            buf.put_slice(s.as_bytes());
            buf.put_slice(b"\r\n");
        }
        RespValue::VerbatimString { encoding, data } => {
            // = body = ENC:DATA — total bytes in length prefix include encoding+":"+data
            let body_len = encoding.len() + 1 + data.len();
            buf.put_u8(b'=');
            put_int(body_len as i64, buf);
            buf.put_slice(b"\r\n");
            buf.put_slice(encoding.as_bytes());
            buf.put_u8(b':');
            buf.put_slice(data);
            buf.put_slice(b"\r\n");
        }
        RespValue::Map(pairs) => {
            buf.put_u8(b'%');
            put_int(pairs.len() as i64, buf);
            buf.put_slice(b"\r\n");
            for (k, v) in pairs {
                encode(k, buf);
                encode(v, buf);
            }
        }
        RespValue::Set(items) => {
            buf.put_u8(b'~');
            put_int(items.len() as i64, buf);
            buf.put_slice(b"\r\n");
            for it in items {
                encode(it, buf);
            }
        }
        RespValue::Push(items) => {
            buf.put_u8(b'>');
            put_int(items.len() as i64, buf);
            buf.put_slice(b"\r\n");
            for it in items {
                encode(it, buf);
            }
        }
    }
}

fn put_int(n: i64, buf: &mut BytesMut) {
    let mut out = [0u8; 24];
    let s = i64_to_str(n, &mut out);
    buf.put_slice(s);
}

fn i64_to_str<'a>(n: i64, out: &'a mut [u8; 24]) -> &'a [u8] {
    let s = format!("{}", n);
    let bytes = s.as_bytes();
    out[..bytes.len()].copy_from_slice(bytes);
    &out[..bytes.len()]
}

// ── Decoder ─────────────────────────────────────────────────────────────────

pub fn decode(buf: &mut BytesMut) -> Result<RespValue, RespCodecError> {
    let snapshot = buf.clone();
    match decode_inner(buf) {
        Ok(v) => Ok(v),
        Err(e) => {
            *buf = snapshot;
            Err(e)
        }
    }
}

fn decode_inner(buf: &mut BytesMut) -> Result<RespValue, RespCodecError> {
    if buf.is_empty() {
        return Err(RespCodecError::Incomplete);
    }
    let prefix = buf[0];
    buf.advance(1);
    match prefix {
        b'+' => {
            let line = read_line(buf)?;
            Ok(RespValue::SimpleString(
                String::from_utf8(line).map_err(|e| RespCodecError::InvalidUtf8(e.to_string()))?,
            ))
        }
        b'-' => {
            let line = read_line(buf)?;
            Ok(RespValue::Error(
                String::from_utf8(line).map_err(|e| RespCodecError::InvalidUtf8(e.to_string()))?,
            ))
        }
        b':' => Ok(RespValue::Integer(read_int(buf)?)),
        b'$' => {
            let len = read_int(buf)?;
            if len == -1 {
                return Ok(RespValue::BulkString(None));
            }
            if len < 0 {
                return Err(RespCodecError::InvalidLength(len));
            }
            let need = len as usize + 2;
            if buf.len() < need {
                return Err(RespCodecError::Incomplete);
            }
            let data = buf[..len as usize].to_vec();
            buf.advance(len as usize);
            expect_crlf(buf)?;
            Ok(RespValue::BulkString(Some(data)))
        }
        b'*' => {
            let n = read_int(buf)?;
            if n == -1 {
                return Ok(RespValue::Array(None));
            }
            if n < 0 {
                return Err(RespCodecError::InvalidLength(n));
            }
            let mut items = Vec::with_capacity(n as usize);
            for _ in 0..n {
                items.push(decode_inner(buf)?);
            }
            Ok(RespValue::Array(Some(items)))
        }
        b'_' => {
            expect_crlf(buf)?;
            Ok(RespValue::Null)
        }
        b'#' => {
            let line = read_line(buf)?;
            match &line[..] {
                b"t" => Ok(RespValue::Boolean(true)),
                b"f" => Ok(RespValue::Boolean(false)),
                _ => Err(RespCodecError::InvalidPrefix(b'#')),
            }
        }
        b',' => {
            let line = read_line(buf)?;
            let s = std::str::from_utf8(&line)
                .map_err(|e| RespCodecError::InvalidUtf8(e.to_string()))?;
            let f = match s {
                "inf" | "+inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                "nan" => f64::NAN,
                other => other
                    .parse::<f64>()
                    .map_err(|e| RespCodecError::InvalidDouble(e.to_string()))?,
            };
            Ok(RespValue::Double(f))
        }
        b'(' => {
            let line = read_line(buf)?;
            Ok(RespValue::BigNumber(
                String::from_utf8(line).map_err(|e| RespCodecError::InvalidUtf8(e.to_string()))?,
            ))
        }
        b'=' => {
            let len = read_int(buf)?;
            if len < 4 {
                return Err(RespCodecError::InvalidLength(len));
            }
            let need = len as usize + 2;
            if buf.len() < need {
                return Err(RespCodecError::Incomplete);
            }
            let payload = buf[..len as usize].to_vec();
            buf.advance(len as usize);
            expect_crlf(buf)?;
            let encoding = String::from_utf8(payload[..3].to_vec())
                .map_err(|e| RespCodecError::InvalidUtf8(e.to_string()))?;
            // payload[3] should be ':'; payload[4..] is the data
            if payload[3] != b':' {
                return Err(RespCodecError::InvalidPrefix(b'='));
            }
            Ok(RespValue::VerbatimString {
                encoding,
                data: payload[4..].to_vec(),
            })
        }
        b'%' => {
            let n = read_int(buf)?;
            if n < 0 {
                return Err(RespCodecError::InvalidLength(n));
            }
            let mut pairs = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let k = decode_inner(buf)?;
                let v = decode_inner(buf)?;
                pairs.push((k, v));
            }
            Ok(RespValue::Map(pairs))
        }
        b'~' => {
            let n = read_int(buf)?;
            if n < 0 {
                return Err(RespCodecError::InvalidLength(n));
            }
            let mut items = Vec::with_capacity(n as usize);
            for _ in 0..n {
                items.push(decode_inner(buf)?);
            }
            Ok(RespValue::Set(items))
        }
        b'>' => {
            let n = read_int(buf)?;
            if n < 0 {
                return Err(RespCodecError::InvalidLength(n));
            }
            let mut items = Vec::with_capacity(n as usize);
            for _ in 0..n {
                items.push(decode_inner(buf)?);
            }
            Ok(RespValue::Push(items))
        }
        other => Err(RespCodecError::InvalidPrefix(other)),
    }
}

fn read_line(buf: &mut BytesMut) -> Result<Vec<u8>, RespCodecError> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            let line = buf[..i].to_vec();
            buf.advance(i + 2);
            return Ok(line);
        }
        i += 1;
    }
    Err(RespCodecError::Incomplete)
}

fn read_int(buf: &mut BytesMut) -> Result<i64, RespCodecError> {
    let line = read_line(buf)?;
    let s = std::str::from_utf8(&line).map_err(|e| RespCodecError::InvalidUtf8(e.to_string()))?;
    s.parse::<i64>()
        .map_err(|e| RespCodecError::InvalidInteger(e.to_string()))
}

fn expect_crlf(buf: &mut BytesMut) -> Result<(), RespCodecError> {
    if buf.len() < 2 {
        return Err(RespCodecError::Incomplete);
    }
    if buf[0] != b'\r' || buf[1] != b'\n' {
        return Err(RespCodecError::InvalidPrefix(buf[0]));
    }
    buf.advance(2);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(v: RespValue) {
        let mut buf = BytesMut::new();
        encode(&v, &mut buf);
        let snapshot = buf.clone();
        let decoded = decode(&mut buf).unwrap_or_else(|e| {
            panic!("decode {:?} of {:?}: bytes={:?}", e, v, &snapshot[..])
        });
        assert_eq!(decoded, v);
        assert!(buf.is_empty(), "trailing bytes: {:?}", buf);
    }

    #[test]
    fn simple_string_round_trip() {
        rt(RespValue::SimpleString("OK".into()));
    }
    #[test]
    fn error_round_trip() {
        rt(RespValue::Error("ERR boom".into()));
    }
    #[test]
    fn integer_zero() {
        rt(RespValue::Integer(0));
    }
    #[test]
    fn integer_negative() {
        rt(RespValue::Integer(-9_223_372_036_854_775_807));
    }
    #[test]
    fn integer_max() {
        rt(RespValue::Integer(i64::MAX));
    }
    #[test]
    fn bulk_string_simple() {
        rt(RespValue::bulk(b"hello".to_vec()));
    }
    #[test]
    fn bulk_string_empty() {
        rt(RespValue::bulk(Vec::new()));
    }
    #[test]
    fn bulk_string_with_crlf() {
        rt(RespValue::bulk(b"line1\r\nline2".to_vec()));
    }
    #[test]
    fn bulk_string_binary() {
        rt(RespValue::bulk(vec![0u8, 1, 2, 0xFF, 0xFE]));
    }
    #[test]
    fn bulk_string_nil() {
        rt(RespValue::nil_bulk());
    }
    #[test]
    fn array_empty() {
        rt(RespValue::empty_array());
    }
    #[test]
    fn array_nil() {
        rt(RespValue::nil_array());
    }
    #[test]
    fn array_mixed() {
        rt(RespValue::array(vec![
            RespValue::Integer(1),
            RespValue::bulk(b"two".to_vec()),
            RespValue::SimpleString("three".into()),
            RespValue::nil_bulk(),
        ]));
    }
    #[test]
    fn array_nested() {
        rt(RespValue::array(vec![
            RespValue::array(vec![RespValue::Integer(1), RespValue::Integer(2)]),
            RespValue::array(vec![RespValue::Integer(3)]),
            RespValue::empty_array(),
        ]));
    }
    #[test]
    fn null_resp3() {
        rt(RespValue::Null);
    }
    #[test]
    fn boolean_true() {
        rt(RespValue::Boolean(true));
    }
    #[test]
    fn boolean_false() {
        rt(RespValue::Boolean(false));
    }
    #[test]
    fn double_finite() {
        rt(RespValue::Double(3.14));
    }
    #[test]
    fn double_negative() {
        rt(RespValue::Double(-1.5));
    }
    #[test]
    fn double_inf() {
        let mut buf = BytesMut::new();
        encode(&RespValue::Double(f64::INFINITY), &mut buf);
        let v = decode(&mut buf).unwrap();
        match v {
            RespValue::Double(d) => assert!(d.is_infinite() && d > 0.0),
            other => panic!("expected Double, got {:?}", other),
        }
    }
    #[test]
    fn double_neg_inf() {
        let mut buf = BytesMut::new();
        encode(&RespValue::Double(f64::NEG_INFINITY), &mut buf);
        let v = decode(&mut buf).unwrap();
        match v {
            RespValue::Double(d) => assert!(d.is_infinite() && d < 0.0),
            other => panic!("expected Double, got {:?}", other),
        }
    }
    #[test]
    fn big_number_round_trip() {
        rt(RespValue::BigNumber(
            "1234567890123456789012345678901234567890".into(),
        ));
    }
    #[test]
    fn verbatim_string_round_trip() {
        rt(RespValue::VerbatimString {
            encoding: "txt".into(),
            data: b"hello\nworld".to_vec(),
        });
    }
    #[test]
    fn map_round_trip() {
        rt(RespValue::Map(vec![
            (RespValue::bulk(b"k1".to_vec()), RespValue::Integer(1)),
            (RespValue::bulk(b"k2".to_vec()), RespValue::Integer(2)),
        ]));
    }
    #[test]
    fn set_round_trip() {
        rt(RespValue::Set(vec![
            RespValue::bulk(b"a".to_vec()),
            RespValue::bulk(b"b".to_vec()),
        ]));
    }
    #[test]
    fn push_round_trip() {
        rt(RespValue::Push(vec![
            RespValue::SimpleString("message".into()),
            RespValue::bulk(b"chan".to_vec()),
            RespValue::bulk(b"hello".to_vec()),
        ]));
    }
    #[test]
    fn bulk_string_byte_layout_matches_spec() {
        let mut buf = BytesMut::new();
        encode(&RespValue::bulk(b"hello".to_vec()), &mut buf);
        assert_eq!(&buf[..], b"$5\r\nhello\r\n");
    }
    #[test]
    fn nil_bulk_byte_layout() {
        let mut buf = BytesMut::new();
        encode(&RespValue::nil_bulk(), &mut buf);
        assert_eq!(&buf[..], b"$-1\r\n");
    }
    #[test]
    fn array_byte_layout() {
        let mut buf = BytesMut::new();
        encode(
            &RespValue::array(vec![RespValue::Integer(1), RespValue::Integer(2)]),
            &mut buf,
        );
        assert_eq!(&buf[..], b"*2\r\n:1\r\n:2\r\n");
    }
    #[test]
    fn incomplete_returns_buffer_intact() {
        let mut buf = BytesMut::from(&b"$5\r\nhel"[..]);
        let snapshot = buf.clone();
        let err = decode(&mut buf).unwrap_err();
        assert_eq!(err, RespCodecError::Incomplete);
        assert_eq!(buf, snapshot);
    }
    #[test]
    fn incomplete_array_returns_buffer_intact() {
        let mut buf = BytesMut::from(&b"*3\r\n:1\r\n"[..]);
        let snapshot = buf.clone();
        let err = decode(&mut buf).unwrap_err();
        assert_eq!(err, RespCodecError::Incomplete);
        assert_eq!(buf, snapshot);
    }
    #[test]
    fn unknown_prefix_errors() {
        let mut buf = BytesMut::from(&b"&abc\r\n"[..]);
        let err = decode(&mut buf).unwrap_err();
        assert_eq!(err, RespCodecError::InvalidPrefix(b'&'));
    }
    #[test]
    fn invalid_integer_errors() {
        let mut buf = BytesMut::from(&b":abc\r\n"[..]);
        let err = decode(&mut buf).unwrap_err();
        match err {
            RespCodecError::InvalidInteger(_) => {}
            other => panic!("expected InvalidInteger, got {:?}", other),
        }
    }
    #[test]
    fn negative_bulk_length_errors() {
        let mut buf = BytesMut::from(&b"$-2\r\n"[..]);
        let err = decode(&mut buf).unwrap_err();
        assert_eq!(err, RespCodecError::InvalidLength(-2));
    }
    #[test]
    fn back_to_back_replies() {
        let mut buf = BytesMut::new();
        encode(&RespValue::SimpleString("OK".into()), &mut buf);
        encode(&RespValue::Integer(42), &mut buf);
        encode(&RespValue::bulk(b"x".to_vec()), &mut buf);
        let v1 = decode(&mut buf).unwrap();
        let v2 = decode(&mut buf).unwrap();
        let v3 = decode(&mut buf).unwrap();
        assert_eq!(v1, RespValue::SimpleString("OK".into()));
        assert_eq!(v2, RespValue::Integer(42));
        assert_eq!(v3, RespValue::bulk(b"x".to_vec()));
        assert!(buf.is_empty());
    }
    #[test]
    fn map_with_three_entries_round_trip() {
        let pairs = vec![
            (RespValue::bulk(b"a".to_vec()), RespValue::Integer(1)),
            (RespValue::bulk(b"b".to_vec()), RespValue::Integer(2)),
            (RespValue::bulk(b"c".to_vec()), RespValue::Integer(3)),
        ];
        rt(RespValue::Map(pairs));
    }
    #[test]
    fn deeply_nested_array() {
        let inner = RespValue::array(vec![RespValue::Integer(1)]);
        let mid = RespValue::array(vec![inner.clone(), inner.clone()]);
        let outer = RespValue::array(vec![mid.clone(), mid]);
        rt(outer);
    }
    #[test]
    fn ok_helper_matches_simple_string() {
        assert_eq!(RespValue::ok(), RespValue::SimpleString("OK".into()));
    }
    #[test]
    fn pong_helper_matches_simple_string() {
        assert_eq!(RespValue::pong(), RespValue::SimpleString("PONG".into()));
    }
    #[test]
    fn nil_helpers_match() {
        assert_eq!(RespValue::nil_bulk(), RespValue::BulkString(None));
        assert_eq!(RespValue::nil_array(), RespValue::Array(None));
    }
    #[test]
    fn error_text_includes_kind() {
        let mut buf = BytesMut::new();
        encode(&RespValue::Error("WRONGTYPE wrong kind".into()), &mut buf);
        assert!(buf.starts_with(b"-WRONGTYPE wrong kind\r\n"));
    }
}
