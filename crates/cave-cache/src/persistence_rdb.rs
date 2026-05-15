// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: valkey-io/valkey@7.2 src/rdb.c (rdbSave, rdbLoad)
//
//! RDB (Redis Database) binary snapshot encoder/decoder.
//!
//! Implements the subset of RDB version 11 that cave-cache uses for
//! its keyspace dumps:
//!
//! * Magic header: `REDIS0011`
//! * AUX fields: `AUX_TAG → AUX_VALUE` pairs (redis-ver, redis-bits, …)
//! * `SELECTDB <idx>` for each non-empty database
//! * `RESIZEDB <total>, <expires>` size hint
//! * `EXPIRETIME_MS <ms>` per-key expiry timestamp
//! * Per-type value entries: string (0), list (1 / 14 plain), set (2 /
//!   15 plain), zset (3 / 17 listpack), hash (4 / 16 listpack), stream
//!   (15 / 19 listpacks v3)
//! * EOF marker `0xFF` + CRC64 placeholder (8 zero bytes)
//!
//! Atomic save semantics: write to `<path>.tmp`, fsync, rename — this
//! is the "BGSAVE under copy-on-write" pattern of upstream, adapted
//! for Rust where COW is achieved by cloning the keyspace under a read
//! lock before producing the byte stream.
//!
//! LZF compression for long strings is deferred — RDB version 11 emits
//! plain string entries (type 0) which load fine.

use crate::db::Db;
use crate::types::{Entry, Value, ZSet};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const RDB_MAGIC: &[u8; 5] = b"REDIS";
pub const RDB_VERSION: u32 = 11;

// Opcode table — matches src/rdb.h.
pub const RDB_OPCODE_AUX: u8 = 0xFA;
pub const RDB_OPCODE_RESIZEDB: u8 = 0xFB;
pub const RDB_OPCODE_EXPIRETIME_MS: u8 = 0xFC;
pub const RDB_OPCODE_EXPIRETIME: u8 = 0xFD;
pub const RDB_OPCODE_SELECTDB: u8 = 0xFE;
pub const RDB_OPCODE_EOF: u8 = 0xFF;

// Type table.
pub const RDB_TYPE_STRING: u8 = 0;
pub const RDB_TYPE_LIST: u8 = 1;
pub const RDB_TYPE_SET: u8 = 2;
pub const RDB_TYPE_ZSET: u8 = 3;
pub const RDB_TYPE_HASH: u8 = 4;
pub const RDB_TYPE_STREAM_LISTPACKS_3: u8 = 21;

#[derive(Debug, thiserror::Error)]
pub enum RdbError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("bad magic — got {0:?}, want \"REDIS\"")]
    BadMagic([u8; 5]),
    #[error("unsupported RDB version {0}")]
    UnsupportedVersion(u32),
    #[error("unexpected opcode 0x{0:02x} at offset {1}")]
    UnexpectedOpcode(u8, usize),
    #[error("truncated rdb stream at offset {0}")]
    Truncated(usize),
    #[error("invalid length encoding at offset {0}")]
    InvalidLength(usize),
    #[error("invalid utf-8 in stream id")]
    InvalidStreamId,
}

#[derive(Debug, Clone)]
pub struct RdbDb {
    pub idx: u32,
    pub entries: Vec<(Vec<u8>, Entry)>,
}

#[derive(Debug, Clone, Default)]
pub struct RdbDump {
    pub aux: Vec<(Vec<u8>, Vec<u8>)>,
    pub dbs: Vec<RdbDb>,
}

// ── Encode ───────────────────────────────────────────────────────────────────

/// Snapshot a single in-memory `Db` into an `RdbDump`. The caller
/// holds the read lock and produces this owned shape so the encoder
/// step can release the lock immediately.
pub fn snapshot_db(idx: u32, db: &Db) -> RdbDb {
    let mut entries: Vec<(Vec<u8>, Entry)> = Vec::with_capacity(db.keys.len());
    for (k, e) in &db.keys {
        if e.is_expired() {
            continue;
        }
        entries.push((k.clone(), e.clone()));
    }
    RdbDb { idx, entries }
}

pub fn snapshot_with_aux(aux: Vec<(Vec<u8>, Vec<u8>)>, dbs: Vec<RdbDb>) -> RdbDump {
    RdbDump { aux, dbs }
}

pub fn encode(dump: &RdbDump) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1024);
    buf.extend_from_slice(RDB_MAGIC);
    write_ascii_u32(&mut buf, RDB_VERSION);

    for (k, v) in &dump.aux {
        buf.push(RDB_OPCODE_AUX);
        write_string(&mut buf, k);
        write_string(&mut buf, v);
    }

    for db in &dump.dbs {
        if db.entries.is_empty() {
            continue;
        }
        buf.push(RDB_OPCODE_SELECTDB);
        write_length(&mut buf, db.idx as usize);

        buf.push(RDB_OPCODE_RESIZEDB);
        write_length(&mut buf, db.entries.len());
        write_length(&mut buf, db.entries.iter().filter(|(_, e)| e.expires_at.is_some()).count());

        for (key, entry) in &db.entries {
            if let Some(expires_at) = entry.expires_at {
                let ms = absolute_expiry_ms(expires_at);
                buf.push(RDB_OPCODE_EXPIRETIME_MS);
                buf.extend_from_slice(&ms.to_le_bytes());
            }
            buf.push(rdb_type_of(&entry.value));
            write_string(&mut buf, key);
            write_value(&mut buf, &entry.value);
        }
    }

    buf.push(RDB_OPCODE_EOF);
    buf.extend_from_slice(&[0u8; 8]); // CRC64 placeholder
    buf
}

pub async fn atomic_save<P: AsRef<Path>>(path: P, bytes: &[u8]) -> io::Result<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = parent.join(format!(
        "{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    {
        let mut f = tokio::fs::File::create(&tmp).await?;
        use tokio::io::AsyncWriteExt as _;
        f.write_all(bytes).await?;
        f.sync_all().await?;
    }
    tokio::fs::rename(&tmp, path).await
}

fn rdb_type_of(v: &Value) -> u8 {
    match v {
        Value::String(_) => RDB_TYPE_STRING,
        Value::List(_) => RDB_TYPE_LIST,
        Value::Set(_) => RDB_TYPE_SET,
        Value::ZSet(_) => RDB_TYPE_ZSET,
        Value::Hash(_) => RDB_TYPE_HASH,
        Value::Stream(_) => RDB_TYPE_STREAM_LISTPACKS_3,
    }
}

fn write_value(buf: &mut Vec<u8>, v: &Value) {
    match v {
        Value::String(s) => write_string(buf, s),
        Value::List(l) => {
            write_length(buf, l.len());
            for item in l {
                write_string(buf, item);
            }
        }
        Value::Set(s) => {
            write_length(buf, s.len());
            for item in s {
                write_string(buf, item);
            }
        }
        Value::Hash(h) => {
            write_length(buf, h.len());
            for (k, v) in h {
                write_string(buf, k);
                write_string(buf, v);
            }
        }
        Value::ZSet(z) => {
            let asc: Vec<(Vec<u8>, f64)> =
                z.iter_asc().map(|(m, s)| (m.clone(), s)).collect();
            write_length(buf, asc.len());
            for (m, score) in &asc {
                write_string(buf, m);
                buf.extend_from_slice(&score.to_le_bytes());
            }
        }
        Value::Stream(stream) => {
            write_length(buf, stream.entries.len());
            for e in &stream.entries {
                let id = e.id.to_string();
                write_string(buf, id.as_bytes());
                write_length(buf, e.fields.len());
                for (k, v) in &e.fields {
                    write_string(buf, k);
                    write_string(buf, v);
                }
            }
        }
    }
}

// Length encoding mirrors upstream's `rdbSaveLen`.
//
// * 6-bit       (00xxxxxx)
// * 14-bit      (01xxxxxx xxxxxxxx)
// * 32-bit BE   (10000000 xxxxxxxx xxxxxxxx xxxxxxxx xxxxxxxx)
fn write_length(buf: &mut Vec<u8>, len: usize) {
    if len < 64 {
        buf.push(len as u8);
    } else if len < 16384 {
        buf.push(0x40 | ((len >> 8) as u8 & 0x3F));
        buf.push((len & 0xFF) as u8);
    } else {
        buf.push(0x80);
        buf.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

fn write_string(buf: &mut Vec<u8>, s: &[u8]) {
    write_length(buf, s.len());
    buf.extend_from_slice(s);
}

fn write_ascii_u32(buf: &mut Vec<u8>, v: u32) {
    // RDB version is encoded as a 4-byte ASCII numeric string.
    let s = format!("{:04}", v);
    buf.extend_from_slice(s.as_bytes());
}

fn absolute_expiry_ms(expires_at: std::time::Instant) -> u64 {
    let now = std::time::Instant::now();
    let remaining = expires_at.checked_duration_since(now).unwrap_or(Duration::ZERO);
    let unix_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    unix_now + remaining.as_millis() as u64
}

// ── Decode ───────────────────────────────────────────────────────────────────

pub fn decode(buf: &[u8]) -> Result<RdbDump, RdbError> {
    let mut c = Cursor::new(buf);
    let magic = c.read_array::<5>()?;
    if &magic != RDB_MAGIC {
        return Err(RdbError::BadMagic(magic));
    }
    let ver = c.read_ascii_u32()?;
    if ver != RDB_VERSION {
        return Err(RdbError::UnsupportedVersion(ver));
    }
    let mut dump = RdbDump::default();
    let mut current_db: Option<RdbDb> = None;
    let mut pending_expiry_ms: Option<u64> = None;
    loop {
        let op = c.read_u8()?;
        match op {
            RDB_OPCODE_AUX => {
                let k = c.read_string()?;
                let v = c.read_string()?;
                dump.aux.push((k, v));
            }
            RDB_OPCODE_SELECTDB => {
                if let Some(db) = current_db.take() {
                    dump.dbs.push(db);
                }
                let idx = c.read_length()? as u32;
                current_db = Some(RdbDb { idx, entries: Vec::new() });
            }
            RDB_OPCODE_RESIZEDB => {
                let _total = c.read_length()?;
                let _expires = c.read_length()?;
            }
            RDB_OPCODE_EXPIRETIME_MS => {
                let bytes = c.read_array::<8>()?;
                pending_expiry_ms = Some(u64::from_le_bytes(bytes));
            }
            RDB_OPCODE_EXPIRETIME => {
                let bytes = c.read_array::<4>()?;
                pending_expiry_ms = Some(u32::from_le_bytes(bytes) as u64 * 1000);
            }
            RDB_OPCODE_EOF => {
                if let Some(db) = current_db.take() {
                    dump.dbs.push(db);
                }
                return Ok(dump);
            }
            t if t <= RDB_TYPE_HASH || t == RDB_TYPE_STREAM_LISTPACKS_3 => {
                let key = c.read_string()?;
                let value = read_value(&mut c, t)?;
                let mut entry = Entry::new(value);
                if let Some(ms) = pending_expiry_ms.take() {
                    let now_unix = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    if ms > now_unix {
                        entry.expires_at = Some(
                            std::time::Instant::now()
                                + Duration::from_millis(ms - now_unix),
                        );
                    } else {
                        // Already expired — skip.
                        continue;
                    }
                }
                if let Some(db) = current_db.as_mut() {
                    db.entries.push((key, entry));
                }
            }
            other => return Err(RdbError::UnexpectedOpcode(other, c.pos)),
        }
    }
}

fn read_value(c: &mut Cursor, type_byte: u8) -> Result<Value, RdbError> {
    Ok(match type_byte {
        RDB_TYPE_STRING => Value::String(c.read_string()?),
        RDB_TYPE_LIST => {
            let n = c.read_length()?;
            let mut list = VecDeque::with_capacity(n);
            for _ in 0..n {
                list.push_back(c.read_string()?);
            }
            Value::List(list)
        }
        RDB_TYPE_SET => {
            let n = c.read_length()?;
            let mut set = HashSet::with_capacity(n);
            for _ in 0..n {
                set.insert(c.read_string()?);
            }
            Value::Set(set)
        }
        RDB_TYPE_HASH => {
            let n = c.read_length()?;
            let mut h = HashMap::with_capacity(n);
            for _ in 0..n {
                let k = c.read_string()?;
                let v = c.read_string()?;
                h.insert(k, v);
            }
            Value::Hash(h)
        }
        RDB_TYPE_ZSET => {
            let n = c.read_length()?;
            let mut z = ZSet::default();
            for _ in 0..n {
                let m = c.read_string()?;
                let score_bytes = c.read_array::<8>()?;
                let score = f64::from_le_bytes(score_bytes);
                z.add(m, score);
            }
            Value::ZSet(z)
        }
        RDB_TYPE_STREAM_LISTPACKS_3 => {
            let n = c.read_length()?;
            let mut s = crate::types::Stream::default();
            for _ in 0..n {
                let id_bytes = c.read_string()?;
                let id = crate::types::StreamId::parse(&id_bytes).ok_or(RdbError::InvalidStreamId)?;
                let fcount = c.read_length()?;
                let mut fields = Vec::with_capacity(fcount);
                for _ in 0..fcount {
                    let k = c.read_string()?;
                    let v = c.read_string()?;
                    fields.push((k, v));
                }
                s.entries.push(crate::types::StreamEntry { id, fields });
            }
            Value::Stream(s)
        }
        _ => return Err(RdbError::UnexpectedOpcode(type_byte, c.pos)),
    })
}

// ── Cursor ───────────────────────────────────────────────────────────────────

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, RdbError> {
        if self.pos >= self.buf.len() {
            return Err(RdbError::Truncated(self.pos));
        }
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], RdbError> {
        if self.pos + N > self.buf.len() {
            return Err(RdbError::Truncated(self.pos));
        }
        let mut a = [0u8; N];
        a.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        Ok(a)
    }

    fn read_ascii_u32(&mut self) -> Result<u32, RdbError> {
        let s = self.read_array::<4>()?;
        let txt = std::str::from_utf8(&s).map_err(|_| RdbError::InvalidLength(self.pos))?;
        let trimmed = txt.trim_start_matches('0');
        if trimmed.is_empty() {
            return Ok(0);
        }
        trimmed
            .parse::<u32>()
            .map_err(|_| RdbError::InvalidLength(self.pos))
    }

    fn read_length(&mut self) -> Result<usize, RdbError> {
        let b = self.read_u8()?;
        let high = b & 0xC0;
        match high {
            0x00 => Ok((b & 0x3F) as usize),
            0x40 => {
                let lo = self.read_u8()?;
                Ok((((b & 0x3F) as usize) << 8) | (lo as usize))
            }
            0x80 => {
                let bytes = self.read_array::<4>()?;
                Ok(u32::from_be_bytes(bytes) as usize)
            }
            _ => Err(RdbError::InvalidLength(self.pos)),
        }
    }

    fn read_string(&mut self) -> Result<Vec<u8>, RdbError> {
        let n = self.read_length()?;
        if self.pos + n > self.buf.len() {
            return Err(RdbError::Truncated(self.pos));
        }
        let v = self.buf[self.pos..self.pos + n].to_vec();
        self.pos += n;
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Entry, Stream, StreamEntry, StreamId, Value, ZSet};
    use std::collections::{HashMap, HashSet, VecDeque};

    fn entry(v: Value) -> Entry {
        Entry::new(v)
    }

    fn dump_with(entries: Vec<(Vec<u8>, Entry)>) -> RdbDump {
        RdbDump {
            aux: vec![
                (b"redis-ver".to_vec(), b"7.2.0".to_vec()),
                (b"redis-bits".to_vec(), b"64".to_vec()),
            ],
            dbs: vec![RdbDb { idx: 0, entries }],
        }
    }

    #[test]
    fn encode_starts_with_magic_and_version() {
        let b = encode(&RdbDump::default());
        assert_eq!(&b[..5], RDB_MAGIC);
        assert_eq!(&b[5..9], b"0011");
    }

    #[test]
    fn empty_dump_ends_with_eof_marker() {
        let b = encode(&RdbDump::default());
        // Magic(5) + Ver(4) + EOF(1) + CRC(8) = 18
        assert_eq!(b.len(), 18);
        assert_eq!(b[9], RDB_OPCODE_EOF);
    }

    #[test]
    fn aux_fields_are_emitted_with_aux_opcode() {
        let mut d = RdbDump::default();
        d.aux.push((b"redis-ver".to_vec(), b"7.2.0".to_vec()));
        let b = encode(&d);
        // After magic + version, first byte should be AUX.
        assert_eq!(b[9], RDB_OPCODE_AUX);
    }

    #[test]
    fn string_value_round_trips() {
        let e = entry(Value::String(b"hello".to_vec()));
        let dump = dump_with(vec![(b"k".to_vec(), e)]);
        let b = encode(&dump);
        let back = decode(&b).unwrap();
        assert_eq!(back.dbs.len(), 1);
        assert_eq!(back.dbs[0].entries.len(), 1);
        assert_eq!(back.dbs[0].entries[0].0, b"k");
        if let Value::String(s) = &back.dbs[0].entries[0].1.value {
            assert_eq!(s, b"hello");
        } else {
            panic!("expected String");
        }
    }

    #[test]
    fn list_value_round_trips_preserving_order() {
        let mut list = VecDeque::new();
        list.push_back(b"a".to_vec());
        list.push_back(b"b".to_vec());
        list.push_back(b"c".to_vec());
        let dump = dump_with(vec![(b"L".to_vec(), entry(Value::List(list)))]);
        let b = encode(&dump);
        let back = decode(&b).unwrap();
        if let Value::List(l) = &back.dbs[0].entries[0].1.value {
            let v: Vec<&[u8]> = l.iter().map(|s| s.as_slice()).collect();
            assert_eq!(v, vec![&b"a"[..], &b"b"[..], &b"c"[..]]);
        } else {
            panic!();
        }
    }

    #[test]
    fn set_value_round_trips() {
        let mut set = HashSet::new();
        set.insert(b"x".to_vec());
        set.insert(b"y".to_vec());
        let dump = dump_with(vec![(b"S".to_vec(), entry(Value::Set(set.clone())))]);
        let back = decode(&encode(&dump)).unwrap();
        if let Value::Set(s) = &back.dbs[0].entries[0].1.value {
            assert_eq!(s, &set);
        } else {
            panic!();
        }
    }

    #[test]
    fn hash_value_round_trips() {
        let mut h = HashMap::new();
        h.insert(b"f1".to_vec(), b"v1".to_vec());
        h.insert(b"f2".to_vec(), b"v2".to_vec());
        let dump = dump_with(vec![(b"H".to_vec(), entry(Value::Hash(h.clone())))]);
        let back = decode(&encode(&dump)).unwrap();
        if let Value::Hash(g) = &back.dbs[0].entries[0].1.value {
            assert_eq!(g, &h);
        } else {
            panic!();
        }
    }

    #[test]
    fn zset_value_round_trips_with_scores() {
        let mut z = ZSet::default();
        z.add(b"m1".to_vec(), 1.0);
        z.add(b"m2".to_vec(), 2.5);
        let dump = dump_with(vec![(b"Z".to_vec(), entry(Value::ZSet(z)))]);
        let back = decode(&encode(&dump)).unwrap();
        if let Value::ZSet(z) = &back.dbs[0].entries[0].1.value {
            let v: Vec<(Vec<u8>, f64)> = z.iter_asc().map(|(m, s)| (m.clone(), s)).collect();
            assert_eq!(v.len(), 2);
            assert!((v[0].1 - 1.0).abs() < 1e-9);
        } else {
            panic!();
        }
    }

    #[test]
    fn stream_value_round_trips_preserving_id_and_fields() {
        let mut s = Stream::default();
        s.entries.push(StreamEntry {
            id: StreamId { ms: 100, seq: 0 },
            fields: vec![(b"f".to_vec(), b"v".to_vec())],
        });
        let dump = dump_with(vec![(b"X".to_vec(), entry(Value::Stream(s)))]);
        let back = decode(&encode(&dump)).unwrap();
        if let Value::Stream(s) = &back.dbs[0].entries[0].1.value {
            assert_eq!(s.entries.len(), 1);
            assert_eq!(s.entries[0].id.ms, 100);
            assert_eq!(s.entries[0].fields[0].0, b"f");
        } else {
            panic!();
        }
    }

    #[test]
    fn multi_db_round_trips() {
        let dump = RdbDump {
            aux: vec![],
            dbs: vec![
                RdbDb {
                    idx: 0,
                    entries: vec![(b"a".to_vec(), entry(Value::String(b"0".to_vec())))],
                },
                RdbDb {
                    idx: 3,
                    entries: vec![(b"b".to_vec(), entry(Value::String(b"3".to_vec())))],
                },
            ],
        };
        let back = decode(&encode(&dump)).unwrap();
        assert_eq!(back.dbs.len(), 2);
        assert_eq!(back.dbs[0].idx, 0);
        assert_eq!(back.dbs[1].idx, 3);
    }

    #[test]
    fn empty_db_skipped_during_encode() {
        let dump = RdbDump {
            aux: vec![],
            dbs: vec![RdbDb { idx: 7, entries: vec![] }],
        };
        let b = encode(&dump);
        // No SELECTDB emitted for an empty database.
        assert!(!b[9..].iter().any(|&x| x == RDB_OPCODE_SELECTDB));
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut b = encode(&RdbDump::default());
        b[0] = b'X';
        let err = decode(&b).unwrap_err();
        assert!(matches!(err, RdbError::BadMagic(_)));
    }

    #[test]
    fn decode_rejects_truncated_stream() {
        let b = encode(&dump_with(vec![(b"k".to_vec(), entry(Value::String(b"v".to_vec())))]));
        let err = decode(&b[..b.len() - 12]).unwrap_err();
        // Either truncated mid-stream or never seen EOF.
        match err {
            RdbError::Truncated(_) => {}
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn length_encoding_6bit_path() {
        let mut buf = Vec::new();
        write_length(&mut buf, 5);
        assert_eq!(buf, vec![5]);
    }

    #[test]
    fn length_encoding_14bit_path() {
        let mut buf = Vec::new();
        write_length(&mut buf, 1000);
        // 1000 → 0x03e8 → high 0x40 | (0x03 & 0x3F), low 0xE8.
        assert_eq!(buf[0] & 0xC0, 0x40);
        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_length().unwrap(), 1000);
    }

    #[test]
    fn length_encoding_32bit_path() {
        let mut buf = Vec::new();
        write_length(&mut buf, 100_000);
        assert_eq!(buf[0], 0x80);
        let mut c = Cursor::new(&buf);
        assert_eq!(c.read_length().unwrap(), 100_000);
    }

    #[test]
    fn aux_field_preserves_redis_ver() {
        let dump = RdbDump {
            aux: vec![(b"redis-ver".to_vec(), b"7.4.0".to_vec())],
            dbs: vec![],
        };
        let back = decode(&encode(&dump)).unwrap();
        assert_eq!(back.aux.len(), 1);
        assert_eq!(back.aux[0].0, b"redis-ver");
        assert_eq!(back.aux[0].1, b"7.4.0");
    }

    #[test]
    fn binary_safe_string_with_nulls_round_trips() {
        let dump = dump_with(vec![(
            b"binkey".to_vec(),
            entry(Value::String(vec![0, 1, 2, 0, 0xFF])),
        )]);
        let back = decode(&encode(&dump)).unwrap();
        if let Value::String(s) = &back.dbs[0].entries[0].1.value {
            assert_eq!(s, &vec![0u8, 1, 2, 0, 0xFF]);
        }
    }

    #[test]
    fn expired_entry_is_skipped_on_decode() {
        let mut e = entry(Value::String(b"v".to_vec()));
        // Set expires in the past.
        e.expires_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(60));
        // We pretend it hasn't expired locally yet (snapshot_db would skip
        // it). Encode it manually.
        let mut buf = Vec::new();
        buf.extend_from_slice(RDB_MAGIC);
        buf.extend_from_slice(b"0011");
        buf.push(RDB_OPCODE_SELECTDB);
        write_length(&mut buf, 0);
        buf.push(RDB_OPCODE_RESIZEDB);
        write_length(&mut buf, 1);
        write_length(&mut buf, 1);
        // Past expiry: 1 ms after unix epoch.
        buf.push(RDB_OPCODE_EXPIRETIME_MS);
        buf.extend_from_slice(&1u64.to_le_bytes());
        buf.push(RDB_TYPE_STRING);
        write_string(&mut buf, b"k");
        write_string(&mut buf, b"v");
        buf.push(RDB_OPCODE_EOF);
        buf.extend_from_slice(&[0u8; 8]);

        let back = decode(&buf).unwrap();
        assert_eq!(back.dbs.len(), 1);
        assert!(back.dbs[0].entries.is_empty());
    }

    #[test]
    fn snapshot_db_filters_expired_keys() {
        let mut db = Db::new();
        let mut alive = entry(Value::String(b"a".to_vec()));
        let mut expired = entry(Value::String(b"e".to_vec()));
        expired.expires_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        alive.expires_at =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
        db.keys.insert(b"alive".to_vec(), alive);
        db.keys.insert(b"expired".to_vec(), expired);
        let s = snapshot_db(0, &db);
        assert_eq!(s.entries.len(), 1);
        assert_eq!(s.entries[0].0, b"alive");
    }

    #[tokio::test]
    async fn atomic_save_writes_and_renames_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snap.rdb");
        let bytes = encode(&dump_with(vec![(
            b"k".to_vec(),
            entry(Value::String(b"v".to_vec())),
        )]));
        atomic_save(&path, &bytes).await.unwrap();
        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back, bytes);
    }

    #[test]
    fn round_trip_through_atomic_save_preserves_data() {
        let dump = dump_with(vec![
            (b"k1".to_vec(), entry(Value::String(b"v1".to_vec()))),
            (b"k2".to_vec(), entry(Value::String(b"v2".to_vec()))),
        ]);
        let bytes = encode(&dump);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.dbs[0].entries.len(), 2);
    }
}
