//! PostgreSQL built-in type OIDs and binary encodings.
//!
//! OIDs sourced from `src/include/catalog/pg_type.dat` in the PostgreSQL
//! source tree (mirrored by `tokio-postgres::types::Type`). Binary encodings
//! follow the network-byte-order conventions documented at
//! <https://www.postgresql.org/docs/current/datatype-binary.html>.

#![allow(non_upper_case_globals)]

pub const BOOL: u32 = 16;
pub const BYTEA: u32 = 17;
pub const CHAR: u32 = 18;
pub const NAME: u32 = 19;
pub const INT8: u32 = 20;
pub const INT2: u32 = 21;
pub const INT4: u32 = 23;
pub const REGPROC: u32 = 24;
pub const TEXT: u32 = 25;
pub const OID: u32 = 26;
pub const TID: u32 = 27;
pub const XID: u32 = 28;
pub const CID: u32 = 29;
pub const JSON: u32 = 114;
pub const XML: u32 = 142;
pub const POINT: u32 = 600;
pub const PATH: u32 = 602;
pub const BOX: u32 = 603;
pub const POLYGON: u32 = 604;
pub const FLOAT4: u32 = 700;
pub const FLOAT8: u32 = 701;
pub const UNKNOWN: u32 = 705;
pub const VARCHAR: u32 = 1043;
pub const DATE: u32 = 1082;
pub const TIME: u32 = 1083;
pub const TIMESTAMP: u32 = 1114;
pub const TIMESTAMPTZ: u32 = 1184;
pub const INTERVAL: u32 = 1186;
pub const TIMETZ: u32 = 1266;
pub const NUMERIC: u32 = 1700;
pub const UUID: u32 = 2950;
pub const JSONB: u32 = 3802;

/// Format codes used in Bind / Describe / DataRow.
pub const FORMAT_TEXT: i16 = 0;
pub const FORMAT_BINARY: i16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatCode {
    Text,
    Binary,
}

impl FormatCode {
    pub fn from_i16(v: i16) -> Option<Self> {
        match v {
            0 => Some(Self::Text),
            1 => Some(Self::Binary),
            _ => None,
        }
    }
    pub fn to_i16(self) -> i16 {
        match self {
            Self::Text => 0,
            Self::Binary => 1,
        }
    }
}

// ── Binary codecs for the most common types ─────────────────────────────────

pub fn encode_bool(v: bool) -> Vec<u8> {
    vec![if v { 1 } else { 0 }]
}

pub fn decode_bool(b: &[u8]) -> Option<bool> {
    if b.len() != 1 {
        return None;
    }
    match b[0] {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

pub fn encode_int2(v: i16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

pub fn decode_int2(b: &[u8]) -> Option<i16> {
    if b.len() != 2 {
        return None;
    }
    Some(i16::from_be_bytes([b[0], b[1]]))
}

pub fn encode_int4(v: i32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

pub fn decode_int4(b: &[u8]) -> Option<i32> {
    if b.len() != 4 {
        return None;
    }
    let mut a = [0; 4];
    a.copy_from_slice(b);
    Some(i32::from_be_bytes(a))
}

pub fn encode_int8(v: i64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

pub fn decode_int8(b: &[u8]) -> Option<i64> {
    if b.len() != 8 {
        return None;
    }
    let mut a = [0; 8];
    a.copy_from_slice(b);
    Some(i64::from_be_bytes(a))
}

pub fn encode_float4(v: f32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

pub fn decode_float4(b: &[u8]) -> Option<f32> {
    if b.len() != 4 {
        return None;
    }
    let mut a = [0; 4];
    a.copy_from_slice(b);
    Some(f32::from_be_bytes(a))
}

pub fn encode_float8(v: f64) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

pub fn decode_float8(b: &[u8]) -> Option<f64> {
    if b.len() != 8 {
        return None;
    }
    let mut a = [0; 8];
    a.copy_from_slice(b);
    Some(f64::from_be_bytes(a))
}

pub fn encode_text(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

pub fn decode_text(b: &[u8]) -> Option<String> {
    std::str::from_utf8(b).ok().map(|s| s.to_string())
}

/// PostgreSQL UUID binary format = 16 bytes big-endian.
pub fn encode_uuid(bytes: [u8; 16]) -> Vec<u8> {
    bytes.to_vec()
}

pub fn decode_uuid(b: &[u8]) -> Option<[u8; 16]> {
    if b.len() != 16 {
        return None;
    }
    let mut a = [0u8; 16];
    a.copy_from_slice(b);
    Some(a)
}

/// JSONB binary header byte = 1 (version 1).
pub fn encode_jsonb(json_text: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + json_text.len());
    v.push(1);
    v.extend_from_slice(json_text.as_bytes());
    v
}

pub fn decode_jsonb(b: &[u8]) -> Option<String> {
    if b.is_empty() || b[0] != 1 {
        return None;
    }
    std::str::from_utf8(&b[1..]).ok().map(|s| s.to_string())
}

/// Returns the human-readable name for a built-in OID.
pub fn type_name(oid: u32) -> Option<&'static str> {
    Some(match oid {
        BOOL => "bool",
        BYTEA => "bytea",
        CHAR => "char",
        NAME => "name",
        INT8 => "int8",
        INT2 => "int2",
        INT4 => "int4",
        REGPROC => "regproc",
        TEXT => "text",
        OID => "oid",
        TID => "tid",
        XID => "xid",
        CID => "cid",
        JSON => "json",
        XML => "xml",
        POINT => "point",
        PATH => "path",
        BOX => "box",
        POLYGON => "polygon",
        FLOAT4 => "float4",
        FLOAT8 => "float8",
        UNKNOWN => "unknown",
        VARCHAR => "varchar",
        DATE => "date",
        TIME => "time",
        TIMESTAMP => "timestamp",
        TIMESTAMPTZ => "timestamptz",
        INTERVAL => "interval",
        TIMETZ => "timetz",
        NUMERIC => "numeric",
        UUID => "uuid",
        JSONB => "jsonb",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_constants_match_postgres() {
        // citation: src/include/catalog/pg_type.dat
        assert_eq!(BOOL, 16);
        assert_eq!(INT4, 23);
        assert_eq!(TEXT, 25);
        assert_eq!(FLOAT8, 701);
        assert_eq!(UUID, 2950);
        assert_eq!(JSONB, 3802);
    }

    #[test]
    fn format_code_round_trip() {
        assert_eq!(FormatCode::from_i16(0), Some(FormatCode::Text));
        assert_eq!(FormatCode::from_i16(1), Some(FormatCode::Binary));
        assert_eq!(FormatCode::from_i16(2), None);
        assert_eq!(FormatCode::Text.to_i16(), 0);
        assert_eq!(FormatCode::Binary.to_i16(), 1);
    }

    #[test]
    fn bool_round_trip_true() {
        assert_eq!(decode_bool(&encode_bool(true)), Some(true));
    }
    #[test]
    fn bool_round_trip_false() {
        assert_eq!(decode_bool(&encode_bool(false)), Some(false));
    }
    #[test]
    fn bool_invalid_byte_returns_none() {
        assert_eq!(decode_bool(&[2]), None);
        assert_eq!(decode_bool(&[]), None);
        assert_eq!(decode_bool(&[0, 0]), None);
    }

    #[test]
    fn int4_round_trip() {
        for v in [0, 1, -1, i32::MAX, i32::MIN, 12345] {
            assert_eq!(decode_int4(&encode_int4(v)), Some(v));
        }
    }

    #[test]
    fn int4_wrong_length_none() {
        assert_eq!(decode_int4(&[0, 0]), None);
    }

    #[test]
    fn int8_round_trip_max() {
        assert_eq!(decode_int8(&encode_int8(i64::MAX)), Some(i64::MAX));
    }

    #[test]
    fn int2_round_trip_negative() {
        assert_eq!(decode_int2(&encode_int2(-32768)), Some(-32768));
    }

    #[test]
    fn float4_round_trip() {
        assert_eq!(decode_float4(&encode_float4(3.14_f32)), Some(3.14_f32));
    }

    #[test]
    fn float8_round_trip_pi() {
        let v = std::f64::consts::PI;
        assert_eq!(decode_float8(&encode_float8(v)), Some(v));
    }

    #[test]
    fn float8_wrong_length_none() {
        assert_eq!(decode_float8(&[0; 4]), None);
    }

    #[test]
    fn text_round_trip() {
        assert_eq!(decode_text(&encode_text("héllo, 🌍")).unwrap(), "héllo, 🌍");
    }

    #[test]
    fn text_invalid_utf8_returns_none() {
        assert!(decode_text(&[0xFF, 0xFE]).is_none());
    }

    #[test]
    fn uuid_round_trip() {
        let id = [
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ];
        assert_eq!(decode_uuid(&encode_uuid(id)), Some(id));
    }

    #[test]
    fn jsonb_version_byte_required() {
        let blob = encode_jsonb(r#"{"k":1}"#);
        assert_eq!(blob[0], 1);
        assert_eq!(decode_jsonb(&blob).unwrap(), r#"{"k":1}"#);
    }

    #[test]
    fn jsonb_wrong_version_none() {
        let blob = vec![2, b'{', b'}'];
        assert_eq!(decode_jsonb(&blob), None);
    }

    #[test]
    fn type_name_known_oids() {
        assert_eq!(type_name(INT4), Some("int4"));
        assert_eq!(type_name(JSONB), Some("jsonb"));
        assert_eq!(type_name(99999), None);
    }
}
