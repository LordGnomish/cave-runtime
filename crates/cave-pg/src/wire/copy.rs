//! `COPY` sub-protocol — bulk text/binary streaming.
//!
//! After the server acknowledges a `COPY ... FROM STDIN` with a
//! [`BackendMessage::CopyInResponse`], the client streams [`CopyData`] frames
//! until it sends `CopyDone` (success) or `CopyFail` (abort). For
//! `COPY ... TO STDOUT` the directions are reversed.
//!
//! The PostgreSQL binary copy format prefixes the stream with the 11-byte
//! signature `PGCOPY\n\xff\r\n\0`, then a 4-byte flags field and a 4-byte
//! header extension length (zero in practice). Each row is `i16(field-count)
//! || (i32(len) || bytes)*`. The trailer is a single `i16(-1)`.

use bytes::{Buf, BufMut, BytesMut};

use super::codec::{ProtocolError, ProtocolResult};

pub const BINARY_SIGNATURE: &[u8] = b"PGCOPY\n\xff\r\n\0";
pub const ROW_TRAILER: i16 = -1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyFormat {
    Text,
    Binary,
    Csv,
}

impl CopyFormat {
    pub fn overall_byte(&self) -> u8 {
        match self {
            CopyFormat::Text | CopyFormat::Csv => 0,
            CopyFormat::Binary => 1,
        }
    }
}

/// Encode a single COPY-text row from string fields, applying PostgreSQL's
/// text-mode escaping (`\\`, `\b`, `\f`, `\n`, `\r`, `\t`, `\v`, `\N` for null).
pub fn encode_text_row(fields: &[Option<&str>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(b'\t');
        }
        match f {
            None => out.extend_from_slice(b"\\N"),
            Some(s) => {
                for ch in s.chars() {
                    match ch {
                        '\\' => out.extend_from_slice(b"\\\\"),
                        '\n' => out.extend_from_slice(b"\\n"),
                        '\r' => out.extend_from_slice(b"\\r"),
                        '\t' => out.extend_from_slice(b"\\t"),
                        '\x08' => out.extend_from_slice(b"\\b"),
                        '\x0c' => out.extend_from_slice(b"\\f"),
                        '\x0b' => out.extend_from_slice(b"\\v"),
                        other => {
                            let mut buf = [0u8; 4];
                            let s = other.encode_utf8(&mut buf);
                            out.extend_from_slice(s.as_bytes());
                        }
                    }
                }
            }
        }
    }
    out.push(b'\n');
    out
}

/// Decode a COPY-text row. Returns `None` for the empty trailer line `\.\n`
/// that the legacy clients sometimes emit.
pub fn decode_text_row(line: &[u8]) -> Option<Vec<Option<String>>> {
    let line = match line.strip_suffix(b"\n") {
        Some(s) => s,
        None => line,
    };
    if line == b"\\." {
        return None;
    }
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.iter().peekable();
    let mut field_was_null = false;
    let mut emitted = 0;
    while let Some(&b) = chars.next() {
        if b == b'\t' {
            if field_was_null && current.is_empty() {
                fields.push(None);
            } else {
                fields.push(Some(std::mem::take(&mut current)));
            }
            field_was_null = false;
            emitted += 1;
            continue;
        }
        if b == b'\\' {
            match chars.next() {
                Some(b'N') => {
                    field_was_null = true;
                }
                Some(b'\\') => current.push('\\'),
                Some(b'n') => current.push('\n'),
                Some(b'r') => current.push('\r'),
                Some(b't') => current.push('\t'),
                Some(b'b') => current.push('\x08'),
                Some(b'f') => current.push('\x0c'),
                Some(b'v') => current.push('\x0b'),
                Some(&other) => current.push(other as char),
                None => {}
            }
        } else {
            current.push(b as char);
        }
    }
    if !current.is_empty() || field_was_null || emitted > 0 || !fields.is_empty() {
        if field_was_null && current.is_empty() {
            fields.push(None);
        } else {
            fields.push(Some(current));
        }
    }
    Some(fields)
}

/// Build a binary COPY stream from rows of optional byte slices.
pub fn encode_binary_stream(rows: &[Vec<Option<&[u8]>>]) -> BytesMut {
    let mut buf = BytesMut::new();
    buf.put_slice(BINARY_SIGNATURE);
    buf.put_u32(0); // flags
    buf.put_u32(0); // header extension length
    for row in rows {
        buf.put_i16(row.len() as i16);
        for cell in row {
            match cell {
                None => buf.put_i32(-1),
                Some(v) => {
                    buf.put_i32(v.len() as i32);
                    buf.put_slice(v);
                }
            }
        }
    }
    buf.put_i16(ROW_TRAILER);
    buf
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryStream {
    pub flags: u32,
    pub rows: Vec<Vec<Option<Vec<u8>>>>,
}

pub fn decode_binary_stream(buf: &mut BytesMut) -> ProtocolResult<BinaryStream> {
    if buf.len() < BINARY_SIGNATURE.len() + 8 {
        return Err(ProtocolError::Incomplete {
            needed: BINARY_SIGNATURE.len() + 8 - buf.len(),
        });
    }
    if &buf[..BINARY_SIGNATURE.len()] != BINARY_SIGNATURE {
        return Err(ProtocolError::InvalidValue {
            field: "copy.binary_signature",
            reason: "mismatch".into(),
        });
    }
    buf.advance(BINARY_SIGNATURE.len());
    let flags = buf.get_u32();
    let ext_len = buf.get_u32() as usize;
    if buf.len() < ext_len {
        return Err(ProtocolError::Incomplete {
            needed: ext_len - buf.len(),
        });
    }
    buf.advance(ext_len);
    let mut rows = Vec::new();
    loop {
        if buf.len() < 2 {
            return Err(ProtocolError::Incomplete {
                needed: 2 - buf.len(),
            });
        }
        let n = buf.get_i16();
        if n == ROW_TRAILER {
            break;
        }
        if n < 0 {
            return Err(ProtocolError::InvalidValue {
                field: "copy.row_field_count",
                reason: format!("{}", n),
            });
        }
        let mut row = Vec::with_capacity(n as usize);
        for _ in 0..n {
            if buf.len() < 4 {
                return Err(ProtocolError::Incomplete {
                    needed: 4 - buf.len(),
                });
            }
            let len = buf.get_i32();
            if len == -1 {
                row.push(None);
            } else if len < 0 {
                return Err(ProtocolError::InvalidValue {
                    field: "copy.cell_len",
                    reason: format!("{}", len),
                });
            } else {
                let len = len as usize;
                if buf.len() < len {
                    return Err(ProtocolError::Incomplete {
                        needed: len - buf.len(),
                    });
                }
                let v = buf[..len].to_vec();
                buf.advance(len);
                row.push(Some(v));
            }
        }
        rows.push(row);
    }
    Ok(BinaryStream { flags, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_signature_matches_spec() {
        assert_eq!(BINARY_SIGNATURE, b"PGCOPY\n\xff\r\n\0");
        assert_eq!(BINARY_SIGNATURE.len(), 11);
    }

    #[test]
    fn text_row_simple() {
        assert_eq!(
            encode_text_row(&[Some("a"), Some("b")]),
            b"a\tb\n".to_vec()
        );
    }

    #[test]
    fn text_row_with_null() {
        assert_eq!(
            encode_text_row(&[Some("a"), None, Some("c")]),
            b"a\t\\N\tc\n".to_vec()
        );
    }

    #[test]
    fn text_row_escapes_special_chars() {
        let row = encode_text_row(&[Some("tab\there\nand\\backslash")]);
        assert_eq!(row, b"tab\\there\\nand\\\\backslash\n".to_vec());
    }

    #[test]
    fn text_row_round_trip_simple() {
        let encoded = encode_text_row(&[Some("hello"), Some("world")]);
        let decoded = decode_text_row(&encoded).unwrap();
        assert_eq!(
            decoded,
            vec![Some("hello".to_string()), Some("world".to_string())]
        );
    }

    #[test]
    fn text_row_round_trip_with_nulls() {
        let encoded = encode_text_row(&[None, Some("x"), None]);
        let decoded = decode_text_row(&encoded).unwrap();
        assert_eq!(decoded, vec![None, Some("x".to_string()), None]);
    }

    #[test]
    fn text_row_round_trip_with_escapes() {
        let encoded = encode_text_row(&[Some("a\tb"), Some("c\nd"), Some("e\\f")]);
        let decoded = decode_text_row(&encoded).unwrap();
        assert_eq!(
            decoded,
            vec![
                Some("a\tb".to_string()),
                Some("c\nd".to_string()),
                Some("e\\f".to_string())
            ]
        );
    }

    #[test]
    fn text_trailer_dot_returns_none() {
        assert!(decode_text_row(b"\\.\n").is_none());
        assert!(decode_text_row(b"\\.").is_none());
    }

    #[test]
    fn binary_stream_round_trip_simple() {
        let rows = vec![vec![Some(&b"\x00\x00\x00\x2a"[..])]];
        let mut buf = encode_binary_stream(&rows);
        let parsed = decode_binary_stream(&mut buf).unwrap();
        assert_eq!(parsed.flags, 0);
        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.rows[0], vec![Some(b"\x00\x00\x00\x2a".to_vec())]);
    }

    #[test]
    fn binary_stream_round_trip_multi_row() {
        let rows = vec![
            vec![Some(&b"a"[..]), None],
            vec![None, Some(&b"bb"[..])],
            vec![Some(&b"cc"[..]), Some(&b"d"[..])],
        ];
        let mut buf = encode_binary_stream(&rows);
        let parsed = decode_binary_stream(&mut buf).unwrap();
        assert_eq!(parsed.rows.len(), 3);
        assert_eq!(parsed.rows[1], vec![None, Some(b"bb".to_vec())]);
    }

    #[test]
    fn binary_stream_empty_rows_round_trip() {
        let rows: Vec<Vec<Option<&[u8]>>> = vec![];
        let mut buf = encode_binary_stream(&rows);
        let parsed = decode_binary_stream(&mut buf).unwrap();
        assert!(parsed.rows.is_empty());
    }

    #[test]
    fn binary_stream_signature_mismatch_errors() {
        let mut buf = BytesMut::from(&b"NOTPGCOPY!!\x00\x00\x00\x00\x00\x00\x00\x00"[..]);
        let err = decode_binary_stream(&mut buf).unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => {
                assert_eq!(field, "copy.binary_signature")
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn binary_stream_truncated_errors() {
        let mut buf = BytesMut::from(&b"PGCOPY\n\xff\r\n\0"[..]);
        let err = decode_binary_stream(&mut buf).unwrap_err();
        match err {
            ProtocolError::Incomplete { .. } => {}
            other => panic!("expected Incomplete, got {:?}", other),
        }
    }

    #[test]
    fn copy_format_overall_byte() {
        assert_eq!(CopyFormat::Text.overall_byte(), 0);
        assert_eq!(CopyFormat::Csv.overall_byte(), 0);
        assert_eq!(CopyFormat::Binary.overall_byte(), 1);
    }

    #[test]
    fn binary_stream_with_extension_header_skipped() {
        let mut buf = BytesMut::new();
        buf.extend_from_slice(BINARY_SIGNATURE);
        buf.put_u32(0); // flags
        buf.put_u32(4); // ext header len
        buf.put_slice(b"junk");
        buf.put_i16(1);
        buf.put_i32(2);
        buf.put_slice(b"hi");
        buf.put_i16(ROW_TRAILER);
        let parsed = decode_binary_stream(&mut buf).unwrap();
        assert_eq!(parsed.rows[0][0].as_deref(), Some(&b"hi"[..]));
    }
}
