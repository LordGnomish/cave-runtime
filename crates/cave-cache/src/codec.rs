//! Synchronous RESP3 codec on `BytesMut`, conforming to
//! `cave_kernel::codec::FrameCodec`.
//!
//! **Honest scope.** RESP3 is line-oriented (CRLF-delimited type-prefix
//! frames), not length-prefix framing. It does **not** reuse the
//! `cave_kernel::codec::length_prefix` helper that powers the
//! PostgreSQL and MongoDB codecs. This module exists to expose the
//! `FrameCodec` contract over RESP for the sake of API uniformity:
//!
//! - The same trait shape (`encode`, `decode`, `Ok(None)` for
//!   "incomplete") works regardless of framing strategy, which is the
//!   broadest part of the cross-crate pattern.
//! - Future consumers (e.g. an in-process Redis client, or a
//!   server-side rewrite that drops the BufReader path) can pick this
//!   up without re-implementing the parser.
//!
//! The async parser in [`super::resp`] is unchanged and still drives
//! the connection handler in [`super::server`]. Sweep-004 deliberately
//! did not migrate that path — see `docs/synergy/sweep-004-framecodec.md`
//! for why (no shared helper benefit, mature 116-test suite).

use bytes::BytesMut;
use cave_kernel::codec::{FrameCodec, FrameError};

use crate::resp::{Resp, encode_resp};

/// Default ceiling on a single RESP frame. Real Redis enforces no
/// hard cap, but we apply 64 MiB to keep a misbehaving peer from
/// announcing a huge bulk-string and OOMing the process.
pub const DEFAULT_MAX_FRAME_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct Resp3Codec {
    max_frame_size: usize,
}

impl Resp3Codec {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_FRAME_SIZE)
    }

    pub fn with_limit(max_frame_size: usize) -> Self {
        Self { max_frame_size }
    }
}

impl Default for Resp3Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCodec<Resp> for Resp3Codec {
    fn encode(&mut self, frame: Resp, buf: &mut BytesMut) -> Result<(), FrameError> {
        // `encode_resp` writes into a `Vec<u8>` because `super::resp` is
        // shared with the existing async write path. Bridge into the
        // BytesMut here. The intermediate allocation is cheap (one Vec
        // per frame) and avoids touching `super::resp` for sweep-004.
        let mut tmp = Vec::with_capacity(64);
        encode_resp(&mut tmp, &frame);
        if tmp.len() > self.max_frame_size {
            return Err(FrameError::Limit { actual: tmp.len(), max: self.max_frame_size });
        }
        buf.extend_from_slice(&tmp);
        Ok(())
    }

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Resp>, FrameError> {
        match decode_resp(buf, self.max_frame_size)? {
            Some((resp, consumed)) => {
                let _ = buf.split_to(consumed);
                Ok(Some(resp))
            }
            None => Ok(None),
        }
    }
}

/// `(parsed, bytes_consumed)` on success.
type DecodeStep<T> = Result<Option<(T, usize)>, FrameError>;

/// Try to decode one RESP value from a slice. Returns
/// `Ok(Some((value, consumed)))`, `Ok(None)` if more bytes are needed,
/// or `Err(_)` on protocol error.
fn decode_resp(src: &[u8], max_frame_size: usize) -> DecodeStep<Resp> {
    if src.is_empty() {
        return Ok(None);
    }
    if src.len() > max_frame_size {
        // The peer has already pushed more bytes than we'll accept in
        // one frame — bail before parsing rather than hold the bytes.
        return Err(FrameError::Limit { actual: src.len(), max: max_frame_size });
    }

    let prefix = src[0];
    let rest = &src[1..];

    match prefix {
        b'+' => decode_simple_string(rest).map(|opt| opt.map(|(s, c)| (Resp::SimpleString(s), 1 + c))),
        b'-' => decode_simple_error(rest).map(|opt| opt.map(|(s, c)| (Resp::Error(s), 1 + c))),
        b':' => decode_integer(rest).map(|opt| opt.map(|(n, c)| (Resp::Integer(n), 1 + c))),
        b'$' => decode_bulk_string(rest).map(|opt| opt.map(|(b, c)| (Resp::BulkString(b), 1 + c))),
        b'*' => match decode_array(rest, max_frame_size)? {
            Some((arr, c)) => Ok(Some((Resp::Array(arr), 1 + c))),
            None => Ok(None),
        },
        b'_' => decode_terminator(rest).map(|opt| opt.map(|c| (Resp::Null, 1 + c))),
        b'#' => decode_boolean(rest).map(|opt| opt.map(|(b, c)| (Resp::Boolean(b), 1 + c))),
        b',' => decode_double(rest).map(|opt| opt.map(|(f, c)| (Resp::Double(f), 1 + c))),
        b'(' => decode_simple_string(rest).map(|opt| opt.map(|(s, c)| (Resp::BigNumber(s), 1 + c))),
        b'%' => match decode_map(rest, max_frame_size)? {
            Some((pairs, c)) => Ok(Some((Resp::Map(pairs), 1 + c))),
            None => Ok(None),
        },
        b'~' => match decode_array_payload(rest, max_frame_size)? {
            Some((items, c)) => Ok(Some((Resp::Set(items), 1 + c))),
            None => Ok(None),
        },
        b'>' => match decode_array_payload(rest, max_frame_size)? {
            Some((items, c)) => Ok(Some((Resp::Push(items), 1 + c))),
            None => Ok(None),
        },
        other => Err(FrameError::invalid(format!("unknown RESP prefix: 0x{:02x}", other))),
    }
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn decode_simple_string(src: &[u8]) -> Result<Option<(Vec<u8>, usize)>, FrameError> {
    match find_crlf(src) {
        Some(idx) => Ok(Some((src[..idx].to_vec(), idx + 2))),
        None => Ok(None),
    }
}

fn decode_simple_error(src: &[u8]) -> Result<Option<(String, usize)>, FrameError> {
    match find_crlf(src) {
        Some(idx) => {
            let s = std::str::from_utf8(&src[..idx])
                .map_err(|_| FrameError::invalid("non-UTF-8 in simple error"))?
                .to_string();
            Ok(Some((s, idx + 2)))
        }
        None => Ok(None),
    }
}

fn decode_integer(src: &[u8]) -> Result<Option<(i64, usize)>, FrameError> {
    match find_crlf(src) {
        Some(idx) => {
            let s = std::str::from_utf8(&src[..idx])
                .map_err(|_| FrameError::invalid("non-UTF-8 in integer"))?;
            let n: i64 = s
                .parse()
                .map_err(|_| FrameError::invalid(format!("invalid integer: {s:?}")))?;
            Ok(Some((n, idx + 2)))
        }
        None => Ok(None),
    }
}

fn decode_terminator(src: &[u8]) -> Result<Option<usize>, FrameError> {
    if src.len() < 2 {
        return Ok(None);
    }
    if &src[..2] != b"\r\n" {
        return Err(FrameError::invalid("RESP3 null missing CRLF"));
    }
    Ok(Some(2))
}

fn decode_boolean(src: &[u8]) -> Result<Option<(bool, usize)>, FrameError> {
    match find_crlf(src) {
        Some(idx) => match &src[..idx] {
            b"t" => Ok(Some((true, idx + 2))),
            b"f" => Ok(Some((false, idx + 2))),
            other => Err(FrameError::invalid(format!(
                "invalid boolean: {:?}",
                String::from_utf8_lossy(other)
            ))),
        },
        None => Ok(None),
    }
}

fn decode_double(src: &[u8]) -> Result<Option<(f64, usize)>, FrameError> {
    match find_crlf(src) {
        Some(idx) => {
            let s = std::str::from_utf8(&src[..idx])
                .map_err(|_| FrameError::invalid("non-UTF-8 in double"))?;
            let f = match s {
                "inf" | "+inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                _ => s
                    .parse::<f64>()
                    .map_err(|_| FrameError::invalid(format!("invalid double: {s:?}")))?,
            };
            Ok(Some((f, idx + 2)))
        }
        None => Ok(None),
    }
}

fn decode_bulk_string(src: &[u8]) -> DecodeStep<Option<Vec<u8>>> {
    let len_idx = match find_crlf(src) {
        Some(i) => i,
        None => return Ok(None),
    };
    let len_str = std::str::from_utf8(&src[..len_idx])
        .map_err(|_| FrameError::invalid("non-UTF-8 in bulk length"))?;
    let len: i64 = len_str
        .parse()
        .map_err(|_| FrameError::invalid(format!("invalid bulk length: {len_str:?}")))?;
    if len == -1 {
        return Ok(Some((None, len_idx + 2)));
    }
    if len < 0 {
        return Err(FrameError::invalid(format!("negative bulk length: {len}")));
    }
    let len = len as usize;
    let payload_start = len_idx + 2;
    let payload_end = payload_start + len;
    let total = payload_end + 2;
    if src.len() < total {
        return Ok(None);
    }
    if &src[payload_end..payload_end + 2] != b"\r\n" {
        return Err(FrameError::invalid("bulk string missing trailing CRLF"));
    }
    let payload = src[payload_start..payload_end].to_vec();
    Ok(Some((Some(payload), total)))
}

fn decode_array(src: &[u8], max_frame_size: usize) -> DecodeStep<Option<Vec<Resp>>> {
    let len_idx = match find_crlf(src) {
        Some(i) => i,
        None => return Ok(None),
    };
    let count_str = std::str::from_utf8(&src[..len_idx])
        .map_err(|_| FrameError::invalid("non-UTF-8 in array count"))?;
    let count: i64 = count_str
        .parse()
        .map_err(|_| FrameError::invalid(format!("invalid array count: {count_str:?}")))?;
    if count == -1 {
        return Ok(Some((None, len_idx + 2)));
    }
    if count < 0 {
        return Err(FrameError::invalid(format!("negative array count: {count}")));
    }
    let count = count as usize;
    let mut consumed = len_idx + 2;
    let mut items = Vec::with_capacity(count);
    for _ in 0..count {
        match decode_resp(&src[consumed..], max_frame_size)? {
            Some((item, c)) => {
                items.push(item);
                consumed += c;
            }
            None => return Ok(None),
        }
    }
    Ok(Some((Some(items), consumed)))
}

fn decode_array_payload(src: &[u8], max_frame_size: usize) -> DecodeStep<Vec<Resp>> {
    match decode_array(src, max_frame_size)? {
        Some((Some(items), c)) => Ok(Some((items, c))),
        Some((None, _)) => Err(FrameError::invalid("Set/Push cannot be null")),
        None => Ok(None),
    }
}

fn decode_map(src: &[u8], max_frame_size: usize) -> DecodeStep<Vec<(Resp, Resp)>> {
    let len_idx = match find_crlf(src) {
        Some(i) => i,
        None => return Ok(None),
    };
    let count_str = std::str::from_utf8(&src[..len_idx])
        .map_err(|_| FrameError::invalid("non-UTF-8 in map count"))?;
    let count: usize = count_str
        .parse()
        .map_err(|_| FrameError::invalid(format!("invalid map count: {count_str:?}")))?;
    let mut consumed = len_idx + 2;
    let mut pairs = Vec::with_capacity(count);
    for _ in 0..count {
        let key = match decode_resp(&src[consumed..], max_frame_size)? {
            Some((k, c)) => {
                consumed += c;
                k
            }
            None => return Ok(None),
        };
        let value = match decode_resp(&src[consumed..], max_frame_size)? {
            Some((v, c)) => {
                consumed += c;
                v
            }
            None => return Ok(None),
        };
        pairs.push((key, value));
    }
    Ok(Some((pairs, consumed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    fn enc(resp: &Resp) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_resp(&mut buf, resp);
        buf
    }

    #[test]
    fn decode_simple_string_roundtrip() {
        let mut buf = BytesMut::from(&b"+OK\r\n"[..]);
        let mut codec = Resp3Codec::new();
        let r = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(r, Resp::SimpleString(b"OK".to_vec()));
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_integer_roundtrip() {
        let mut buf = BytesMut::from(&b":42\r\n"[..]);
        let mut codec = Resp3Codec::new();
        let r = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(r, Resp::Integer(42));
    }

    #[test]
    fn decode_bulk_string_roundtrip() {
        let mut buf = BytesMut::from(&b"$5\r\nhello\r\n"[..]);
        let mut codec = Resp3Codec::new();
        let r = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(r, Resp::BulkString(Some(b"hello".to_vec())));
    }

    #[test]
    fn decode_null_bulk_string() {
        let mut buf = BytesMut::from(&b"$-1\r\n"[..]);
        let mut codec = Resp3Codec::new();
        let r = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(r, Resp::BulkString(None));
    }

    #[test]
    fn decode_array_of_bulks() {
        let mut buf = BytesMut::from(&b"*2\r\n$3\r\nGET\r\n$3\r\nfoo\r\n"[..]);
        let mut codec = Resp3Codec::new();
        let r = codec.decode(&mut buf).unwrap().unwrap();
        match r {
            Resp::Array(Some(items)) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], Resp::BulkString(Some(b"GET".to_vec())));
                assert_eq!(items[1], Resp::BulkString(Some(b"foo".to_vec())));
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn decode_returns_none_when_payload_partial() {
        let mut buf = BytesMut::from(&b"$5\r\nhel"[..]); // 3 of 5 payload bytes
        let snapshot = buf.clone();
        let mut codec = Resp3Codec::new();
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!(buf, snapshot, "buffer must be untouched on Ok(None)");
    }

    #[test]
    fn decode_returns_none_when_array_partial() {
        // Array announces 3 items but only delivers 2.
        let mut buf = BytesMut::from(&b"*3\r\n:1\r\n:2\r\n"[..]);
        let snapshot = buf.clone();
        let mut codec = Resp3Codec::new();
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!(buf, snapshot, "partial array must not advance buffer");
    }

    #[test]
    fn decode_two_pipelined_frames() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&enc(&Resp::SimpleString(b"PONG".to_vec())));
        buf.extend_from_slice(&enc(&Resp::Integer(7)));
        let mut codec = Resp3Codec::new();
        let f1 = codec.decode(&mut buf).unwrap().unwrap();
        let f2 = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(f1, Resp::SimpleString(b"PONG".to_vec()));
        assert_eq!(f2, Resp::Integer(7));
        assert!(buf.is_empty());
    }

    #[test]
    fn decode_resp3_boolean_and_null() {
        let mut buf = BytesMut::from(&b"#t\r\n_\r\n#f\r\n"[..]);
        let mut codec = Resp3Codec::new();
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), Resp::Boolean(true));
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), Resp::Null);
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), Resp::Boolean(false));
    }

    #[test]
    fn decode_resp3_double() {
        let mut buf = BytesMut::from(&b",3.14\r\n,inf\r\n,-inf\r\n"[..]);
        let mut codec = Resp3Codec::new();
        match codec.decode(&mut buf).unwrap().unwrap() {
            Resp::Double(f) => assert!((f - 3.14).abs() < 1e-9),
            other => panic!("expected Double, got {other:?}"),
        }
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), Resp::Double(f64::INFINITY));
        assert_eq!(codec.decode(&mut buf).unwrap().unwrap(), Resp::Double(f64::NEG_INFINITY));
    }

    #[test]
    fn decode_resp3_map() {
        // %2\r\n+a\r\n:1\r\n+b\r\n:2\r\n
        let mut buf = BytesMut::from(&b"%2\r\n+a\r\n:1\r\n+b\r\n:2\r\n"[..]);
        let mut codec = Resp3Codec::new();
        match codec.decode(&mut buf).unwrap().unwrap() {
            Resp::Map(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert_eq!(pairs[0].0, Resp::SimpleString(b"a".to_vec()));
                assert_eq!(pairs[0].1, Resp::Integer(1));
                assert_eq!(pairs[1].0, Resp::SimpleString(b"b".to_vec()));
                assert_eq!(pairs[1].1, Resp::Integer(2));
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn decode_unknown_prefix_is_invalid() {
        let mut buf = BytesMut::from(&b"?garbage\r\n"[..]);
        let mut codec = Resp3Codec::new();
        match codec.decode(&mut buf) {
            Err(FrameError::Invalid(_)) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn encode_then_decode_array_roundtrip_via_legacy_encoder() {
        // Cross-check: the new sync codec must accept whatever the
        // legacy `encode_resp` produces — that's the whole point of
        // exposing both interfaces.
        let frame = Resp::Array(Some(vec![
            Resp::BulkString(Some(b"SET".to_vec())),
            Resp::BulkString(Some(b"key".to_vec())),
            Resp::BulkString(Some(b"value".to_vec())),
        ]));
        let mut codec = Resp3Codec::new();
        let mut buf = BytesMut::new();
        codec.encode(frame.clone(), &mut buf).unwrap();
        let decoded = codec.decode(&mut buf).unwrap().unwrap();
        assert_eq!(decoded, frame);
    }

    #[test]
    fn split_reads_assemble_into_one_frame() {
        // Simulate three TCP segments delivering one bulk string.
        let frame_bytes = b"$11\r\nhello world\r\n";
        let mut buf = BytesMut::new();
        let mut codec = Resp3Codec::new();
        for chunk in &[&frame_bytes[..3], &frame_bytes[3..10], &frame_bytes[10..]] {
            buf.extend_from_slice(chunk);
            // Either decode succeeds on the last chunk or returns None.
            let _ = codec.decode(&mut buf);
        }
        // Final state: buffer should be empty (last decode succeeded).
        assert!(buf.is_empty(), "expected buffer drained, got {} bytes", buf.len());
    }
}
