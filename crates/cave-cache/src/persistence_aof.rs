// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Source: valkey-io/valkey@7.2 src/aof.c (feedAppendOnlyFile, loadAppendOnlyFile, rewriteAppendOnlyFile)
//
//! Append-Only File (AOF) persistence.
//!
//! Every write command is appended to a log file in RESP2 array form
//! (the same wire format clients speak). On boot the cache replays the
//! log to rebuild the keyspace. AOF rewrite (BGREWRITEAOF) shrinks the
//! file by re-deriving a minimal command stream from the current
//! keyspace.
//!
//! ## `appendfsync` modes
//!
//! * [`FsyncPolicy::Always`] — `fsync()` after every command (slow,
//!   strongest durability).
//! * [`FsyncPolicy::EverySec`] — fsync once per second from a
//!   background task (default — bounded data loss = 1s of writes).
//! * [`FsyncPolicy::No`] — let the kernel decide.
//!
//! The AOF file is text-safe (RESP2 is ASCII for the framing) which
//! makes recovery debuggable. Binary key/value payloads are length-
//! framed as `$<len>\r\n<bytes>\r\n`, never escaped.

use crate::db::Db;
use crate::types::{Entry, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsyncPolicy {
    Always,
    EverySec,
    No,
}

impl FsyncPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "always" => Self::Always,
            "everysec" => Self::EverySec,
            "no" => Self::No,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::EverySec => "everysec",
            Self::No => "no",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AofError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("malformed RESP at byte {0}")]
    MalformedResp(usize),
    #[error("AOF rewrite already in progress")]
    RewriteInFlight,
}

/// Encode a single command (already broken into argv) as RESP2.
pub fn encode_command(argv: &[&[u8]]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(argv.iter().map(|a| a.len()).sum::<usize>() + 32);
    buf.extend_from_slice(format!("*{}\r\n", argv.len()).as_bytes());
    for a in argv {
        buf.extend_from_slice(format!("${}\r\n", a.len()).as_bytes());
        buf.extend_from_slice(a);
        buf.extend_from_slice(b"\r\n");
    }
    buf
}

/// Decode a stream of RESP2 commands. Returns the parsed argv lists.
pub fn decode_commands(buf: &[u8]) -> Result<Vec<Vec<Vec<u8>>>, AofError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let (cmd, advance) = decode_one(buf, pos)?;
        out.push(cmd);
        pos += advance;
    }
    Ok(out)
}

fn decode_one(buf: &[u8], mut pos: usize) -> Result<(Vec<Vec<u8>>, usize), AofError> {
    let start = pos;
    if buf.get(pos).copied() != Some(b'*') {
        return Err(AofError::MalformedResp(pos));
    }
    pos += 1;
    let (n_str, after_n) = read_line(buf, pos).ok_or(AofError::MalformedResp(pos))?;
    let n: usize = std::str::from_utf8(n_str)
        .map_err(|_| AofError::MalformedResp(pos))?
        .parse()
        .map_err(|_| AofError::MalformedResp(pos))?;
    pos = after_n;
    let mut argv = Vec::with_capacity(n);
    for _ in 0..n {
        if buf.get(pos).copied() != Some(b'$') {
            return Err(AofError::MalformedResp(pos));
        }
        pos += 1;
        let (len_str, after_len) =
            read_line(buf, pos).ok_or(AofError::MalformedResp(pos))?;
        let len: usize = std::str::from_utf8(len_str)
            .map_err(|_| AofError::MalformedResp(pos))?
            .parse()
            .map_err(|_| AofError::MalformedResp(pos))?;
        pos = after_len;
        if pos + len + 2 > buf.len() {
            return Err(AofError::MalformedResp(pos));
        }
        argv.push(buf[pos..pos + len].to_vec());
        pos += len + 2; // skip CRLF
    }
    Ok((argv, pos - start))
}

fn read_line<'a>(buf: &'a [u8], pos: usize) -> Option<(&'a [u8], usize)> {
    let mut end = pos;
    while end + 1 < buf.len() {
        if buf[end] == b'\r' && buf[end + 1] == b'\n' {
            return Some((&buf[pos..end], end + 2));
        }
        end += 1;
    }
    None
}

// ── AOF writer ───────────────────────────────────────────────────────────────

/// On-disk AOF writer. Persists every appended command + tracks last
/// fsync time for the everysec policy.
pub struct AofLog {
    file: File,
    pub path: PathBuf,
    pub policy: FsyncPolicy,
    bytes_written: u64,
    last_fsync: Instant,
}

impl AofLog {
    pub async fn open(path: impl AsRef<Path>, policy: FsyncPolicy) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        let bytes_written = file.metadata().await.map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            file,
            path,
            policy,
            bytes_written,
            last_fsync: Instant::now(),
        })
    }

    /// Append a command to the log. Returns the bytes written.
    pub async fn append(&mut self, argv: &[&[u8]]) -> io::Result<usize> {
        let bytes = encode_command(argv);
        self.file.write_all(&bytes).await?;
        self.bytes_written += bytes.len() as u64;
        match self.policy {
            FsyncPolicy::Always => {
                self.file.sync_data().await?;
                self.last_fsync = Instant::now();
            }
            FsyncPolicy::EverySec => {
                if self.last_fsync.elapsed() >= Duration::from_secs(1) {
                    self.file.sync_data().await?;
                    self.last_fsync = Instant::now();
                }
            }
            FsyncPolicy::No => {}
        }
        Ok(bytes.len())
    }

    pub async fn fsync(&mut self) -> io::Result<()> {
        self.file.sync_data().await?;
        self.last_fsync = Instant::now();
        Ok(())
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    pub fn last_fsync(&self) -> Instant {
        self.last_fsync
    }
}

/// Replay an AOF file: read all bytes, decode every command, return
/// the argv lists. Callers feed these back into the command dispatch.
pub async fn replay<P: AsRef<Path>>(path: P) -> Result<Vec<Vec<Vec<u8>>>, AofError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut f = File::open(path).await?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).await?;
    decode_commands(&buf)
}

// ── AOF rewrite ──────────────────────────────────────────────────────────────

/// Produce the minimal-command stream that reconstructs `db` — one
/// command per key, using the canonical setter (`SET`, `RPUSH`,
/// `HSET`, `SADD`, `ZADD`, `XADD`). Mirrors upstream's
/// `rewriteAppendOnlyFileRio` per-type emitters.
pub fn rewrite_commands(idx: u32, db: &Db) -> Vec<Vec<Vec<u8>>> {
    let mut out: Vec<Vec<Vec<u8>>> = Vec::new();
    if idx > 0 {
        out.push(vec![b"SELECT".to_vec(), idx.to_string().into_bytes()]);
    }
    for (key, entry) in &db.keys {
        if entry.is_expired() {
            continue;
        }
        match &entry.value {
            Value::String(v) => {
                out.push(vec![b"SET".to_vec(), key.clone(), v.clone()]);
            }
            Value::List(l) => {
                if l.is_empty() {
                    continue;
                }
                let mut cmd = vec![b"RPUSH".to_vec(), key.clone()];
                for item in l {
                    cmd.push(item.clone());
                }
                out.push(cmd);
            }
            Value::Set(s) => {
                if s.is_empty() {
                    continue;
                }
                let mut cmd = vec![b"SADD".to_vec(), key.clone()];
                for item in s {
                    cmd.push(item.clone());
                }
                out.push(cmd);
            }
            Value::Hash(h) => {
                if h.is_empty() {
                    continue;
                }
                let mut cmd = vec![b"HSET".to_vec(), key.clone()];
                for (k, v) in h {
                    cmd.push(k.clone());
                    cmd.push(v.clone());
                }
                out.push(cmd);
            }
            Value::ZSet(z) => {
                if z.is_empty() {
                    continue;
                }
                let mut cmd = vec![b"ZADD".to_vec(), key.clone()];
                for (m, score) in z.iter_asc() {
                    cmd.push(score.to_string().into_bytes());
                    cmd.push(m.clone());
                }
                out.push(cmd);
            }
            Value::Stream(s) => {
                for e in &s.entries {
                    let mut cmd = vec![b"XADD".to_vec(), key.clone(), e.id.to_string().into_bytes()];
                    for (k, v) in &e.fields {
                        cmd.push(k.clone());
                        cmd.push(v.clone());
                    }
                    out.push(cmd);
                }
            }
        }
        if let Some(_expiry_at) = entry.expires_at {
            // For simplicity, emit PEXPIRE using a 24h ceiling.
            // Upstream computes the exact ms-since-epoch; we replicate
            // by deriving the remaining duration at rewrite time.
            let now = Instant::now();
            let remaining_ms = entry
                .expires_at
                .as_ref()
                .and_then(|i| i.checked_duration_since(now))
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if remaining_ms > 0 {
                out.push(vec![
                    b"PEXPIRE".to_vec(),
                    key.clone(),
                    remaining_ms.to_string().into_bytes(),
                ]);
            }
        }
    }
    out
}

/// Atomically rewrite the AOF on disk. Writes to a tmp file, fsyncs,
/// renames. `dbs` is `(idx, &Db)` for each database to dump.
pub async fn rewrite_to_file<P: AsRef<Path>>(
    path: P,
    dbs: &[(u32, &Db)],
) -> io::Result<u64> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or(Path::new("."));
    let tmp = parent.join(format!(
        "{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let mut all_bytes: Vec<u8> = Vec::with_capacity(1024);
    for (idx, db) in dbs {
        for cmd in rewrite_commands(*idx, db) {
            let argv_refs: Vec<&[u8]> = cmd.iter().map(|a| a.as_slice()).collect();
            all_bytes.extend_from_slice(&encode_command(&argv_refs));
        }
    }
    {
        let mut f = File::create(&tmp).await?;
        f.write_all(&all_bytes).await?;
        f.sync_all().await?;
    }
    tokio::fs::rename(&tmp, path).await?;
    Ok(all_bytes.len() as u64)
}

// ── Background rewrite scheduler ─────────────────────────────────────────────

/// Decides whether AOF rewrite is overdue. The default rule, matching
/// upstream's `auto-aof-rewrite-percentage` + `auto-aof-rewrite-min-size`:
/// trigger when the log grew at least `min_size` bytes AND the growth
/// since the last rewrite is at least `pct%`.
#[derive(Debug, Clone, Copy)]
pub struct AofRewriteTrigger {
    pub min_size_bytes: u64,
    pub growth_pct: u32,
    /// Bytes at the moment of the last successful rewrite.
    pub last_size_bytes: u64,
}

impl Default for AofRewriteTrigger {
    fn default() -> Self {
        Self {
            min_size_bytes: 64 * 1024 * 1024,
            growth_pct: 100,
            last_size_bytes: 0,
        }
    }
}

impl AofRewriteTrigger {
    pub fn should_rewrite(&self, current_bytes: u64) -> bool {
        if current_bytes < self.min_size_bytes {
            return false;
        }
        if self.last_size_bytes == 0 {
            return true;
        }
        let growth = current_bytes.saturating_sub(self.last_size_bytes);
        let threshold = self.last_size_bytes * (self.growth_pct as u64) / 100;
        growth >= threshold
    }

    pub fn record_rewrite(&mut self, new_size: u64) {
        self.last_size_bytes = new_size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Entry, Stream, StreamEntry, StreamId, Value, ZSet};
    use std::collections::{HashMap, HashSet, VecDeque};

    #[test]
    fn encode_set_command_round_trips() {
        let bytes = encode_command(&[b"SET", b"k", b"v"]);
        let decoded = decode_commands(&bytes).unwrap();
        assert_eq!(
            decoded[0],
            vec![b"SET".to_vec(), b"k".to_vec(), b"v".to_vec()]
        );
    }

    #[test]
    fn encode_command_produces_resp2_array_with_bulk_strings() {
        let bytes = encode_command(&[b"GET", b"foo"]);
        let s = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(s, "*2\r\n$3\r\nGET\r\n$3\r\nfoo\r\n");
    }

    #[test]
    fn decode_handles_multiple_commands_in_stream() {
        let mut b = encode_command(&[b"SET", b"a", b"1"]);
        b.extend(encode_command(&[b"SET", b"b", b"2"]));
        let cmds = decode_commands(&b).unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0][1], b"a");
        assert_eq!(cmds[1][1], b"b");
    }

    #[test]
    fn decode_binary_safe_payload_preserves_nulls() {
        let payload: &[u8] = &[0, 1, 2, b'\r', b'\n', 0xFF];
        let b = encode_command(&[b"SET", b"k", payload]);
        let cmds = decode_commands(&b).unwrap();
        assert_eq!(cmds[0][2], payload);
    }

    #[test]
    fn decode_rejects_missing_dollar_prefix() {
        let bad = b"*2\r\n$3\r\nGET\r\nXXX\r\n";
        assert!(decode_commands(bad).is_err());
    }

    #[test]
    fn decode_rejects_truncated_payload() {
        let bad = b"*2\r\n$3\r\nGE";
        assert!(decode_commands(bad).is_err());
    }

    #[test]
    fn fsync_policy_parses_known_values() {
        assert_eq!(FsyncPolicy::parse("always"), Some(FsyncPolicy::Always));
        assert_eq!(FsyncPolicy::parse("everysec"), Some(FsyncPolicy::EverySec));
        assert_eq!(FsyncPolicy::parse("no"), Some(FsyncPolicy::No));
        assert_eq!(FsyncPolicy::parse("nope"), None);
    }

    #[test]
    fn fsync_policy_as_str_round_trips() {
        for p in [FsyncPolicy::Always, FsyncPolicy::EverySec, FsyncPolicy::No] {
            assert_eq!(FsyncPolicy::parse(p.as_str()).unwrap(), p);
        }
    }

    #[tokio::test]
    async fn aof_open_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appendonly.aof");
        let _log = AofLog::open(&path, FsyncPolicy::EverySec).await.unwrap();
        assert!(path.exists());
    }

    #[tokio::test]
    async fn aof_append_writes_and_tracks_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.aof");
        let mut log = AofLog::open(&path, FsyncPolicy::Always).await.unwrap();
        let n = log.append(&[b"SET", b"k", b"v"]).await.unwrap();
        assert!(n > 0);
        assert_eq!(log.bytes_written(), n as u64);
    }

    #[tokio::test]
    async fn aof_append_then_replay_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.aof");
        {
            let mut log = AofLog::open(&path, FsyncPolicy::Always).await.unwrap();
            log.append(&[b"SET", b"k1", b"v1"]).await.unwrap();
            log.append(&[b"SET", b"k2", b"v2"]).await.unwrap();
            log.fsync().await.unwrap();
        }
        let cmds = replay(&path).await.unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0][1], b"k1");
        assert_eq!(cmds[1][1], b"k2");
    }

    #[tokio::test]
    async fn replay_returns_empty_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.aof");
        let cmds = replay(&path).await.unwrap();
        assert!(cmds.is_empty());
    }

    #[tokio::test]
    async fn replay_reads_empty_file_as_empty_command_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.aof");
        File::create(&path).await.unwrap();
        let cmds = replay(&path).await.unwrap();
        assert!(cmds.is_empty());
    }

    #[tokio::test]
    async fn fsync_policy_no_skips_fsync_on_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no.aof");
        let mut log = AofLog::open(&path, FsyncPolicy::No).await.unwrap();
        let before = log.last_fsync();
        log.append(&[b"PING"]).await.unwrap();
        // last_fsync should be unchanged (we didn't fsync).
        assert_eq!(log.last_fsync(), before);
    }

    fn db_with_string(k: &[u8], v: &[u8]) -> Db {
        let mut db = Db::new();
        db.keys.insert(k.to_vec(), Entry::new(Value::String(v.to_vec())));
        db
    }

    #[test]
    fn rewrite_string_produces_set_command() {
        let db = db_with_string(b"k", b"v");
        let cmds = rewrite_commands(0, &db);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0][0], b"SET");
        assert_eq!(cmds[0][1], b"k");
        assert_eq!(cmds[0][2], b"v");
    }

    #[test]
    fn rewrite_list_produces_rpush_with_all_items() {
        let mut db = Db::new();
        let mut list = VecDeque::new();
        list.push_back(b"a".to_vec());
        list.push_back(b"b".to_vec());
        db.keys.insert(b"L".to_vec(), Entry::new(Value::List(list)));
        let cmds = rewrite_commands(0, &db);
        assert_eq!(cmds[0][0], b"RPUSH");
        assert_eq!(cmds[0].len(), 4);
    }

    #[test]
    fn rewrite_set_produces_sadd_with_all_members() {
        let mut db = Db::new();
        let mut s = HashSet::new();
        s.insert(b"x".to_vec());
        s.insert(b"y".to_vec());
        db.keys.insert(b"S".to_vec(), Entry::new(Value::Set(s)));
        let cmds = rewrite_commands(0, &db);
        assert_eq!(cmds[0][0], b"SADD");
        assert_eq!(cmds[0].len(), 4);
    }

    #[test]
    fn rewrite_hash_produces_hset_with_field_value_pairs() {
        let mut db = Db::new();
        let mut h = HashMap::new();
        h.insert(b"f".to_vec(), b"v".to_vec());
        db.keys.insert(b"H".to_vec(), Entry::new(Value::Hash(h)));
        let cmds = rewrite_commands(0, &db);
        assert_eq!(cmds[0][0], b"HSET");
        assert_eq!(cmds[0][2], b"f");
        assert_eq!(cmds[0][3], b"v");
    }

    #[test]
    fn rewrite_zset_produces_zadd_with_score_member_pairs() {
        let mut db = Db::new();
        let mut z = ZSet::default();
        z.add(b"m".to_vec(), 1.5);
        db.keys.insert(b"Z".to_vec(), Entry::new(Value::ZSet(z)));
        let cmds = rewrite_commands(0, &db);
        assert_eq!(cmds[0][0], b"ZADD");
        assert_eq!(cmds[0][2], b"1.5");
        assert_eq!(cmds[0][3], b"m");
    }

    #[test]
    fn rewrite_stream_produces_xadd_per_entry() {
        let mut db = Db::new();
        let mut s = Stream::default();
        s.entries.push(StreamEntry {
            id: StreamId { ms: 100, seq: 0 },
            fields: vec![(b"f".to_vec(), b"v".to_vec())],
        });
        s.entries.push(StreamEntry {
            id: StreamId { ms: 200, seq: 0 },
            fields: vec![(b"f2".to_vec(), b"v2".to_vec())],
        });
        db.keys.insert(b"X".to_vec(), Entry::new(Value::Stream(s)));
        let cmds = rewrite_commands(0, &db);
        assert_eq!(cmds.len(), 2);
        assert!(cmds.iter().all(|c| c[0] == b"XADD"));
    }

    #[test]
    fn rewrite_skips_expired_keys() {
        let mut db = Db::new();
        let mut e = Entry::new(Value::String(b"v".to_vec()));
        e.expires_at = Some(Instant::now() - Duration::from_secs(60));
        db.keys.insert(b"k".to_vec(), e);
        let cmds = rewrite_commands(0, &db);
        assert!(cmds.is_empty());
    }

    #[test]
    fn rewrite_emits_select_for_non_zero_db() {
        let db = db_with_string(b"k", b"v");
        let cmds = rewrite_commands(2, &db);
        assert_eq!(cmds[0][0], b"SELECT");
        assert_eq!(cmds[0][1], b"2");
    }

    #[test]
    fn rewrite_skips_empty_list_set_hash_zset() {
        let mut db = Db::new();
        db.keys.insert(b"L".to_vec(), Entry::new(Value::List(VecDeque::new())));
        db.keys.insert(b"S".to_vec(), Entry::new(Value::Set(HashSet::new())));
        db.keys.insert(b"H".to_vec(), Entry::new(Value::Hash(HashMap::new())));
        db.keys.insert(b"Z".to_vec(), Entry::new(Value::ZSet(ZSet::default())));
        let cmds = rewrite_commands(0, &db);
        assert!(cmds.is_empty());
    }

    #[tokio::test]
    async fn rewrite_to_file_writes_minimal_stream() {
        let mut db = Db::new();
        db.keys.insert(b"k1".to_vec(), Entry::new(Value::String(b"v1".to_vec())));
        db.keys.insert(b"k2".to_vec(), Entry::new(Value::String(b"v2".to_vec())));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rw.aof");
        let n = rewrite_to_file(&path, &[(0, &db)]).await.unwrap();
        assert!(n > 0);
        let cmds = replay(&path).await.unwrap();
        assert_eq!(cmds.len(), 2);
    }

    #[tokio::test]
    async fn rewrite_to_file_round_trips_multi_db() {
        let mut db0 = Db::new();
        db0.keys.insert(b"a".to_vec(), Entry::new(Value::String(b"0".to_vec())));
        let mut db5 = Db::new();
        db5.keys.insert(b"b".to_vec(), Entry::new(Value::String(b"5".to_vec())));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.aof");
        rewrite_to_file(&path, &[(0, &db0), (5, &db5)]).await.unwrap();
        let cmds = replay(&path).await.unwrap();
        // Expect SET, SELECT 5, SET (no SELECT 0 because idx==0).
        assert_eq!(cmds.iter().filter(|c| c[0] == b"SELECT").count(), 1);
    }

    #[test]
    fn rewrite_trigger_below_min_size_returns_false() {
        let t = AofRewriteTrigger::default();
        assert!(!t.should_rewrite(1024));
    }

    #[test]
    fn rewrite_trigger_first_time_above_min_size_returns_true() {
        let t = AofRewriteTrigger {
            min_size_bytes: 100,
            growth_pct: 100,
            last_size_bytes: 0,
        };
        assert!(t.should_rewrite(200));
    }

    #[test]
    fn rewrite_trigger_growth_pct_threshold_holds() {
        let mut t = AofRewriteTrigger {
            min_size_bytes: 100,
            growth_pct: 100,
            last_size_bytes: 0,
        };
        t.record_rewrite(200);
        // Doubled? 100% growth → rewrite.
        assert!(t.should_rewrite(400));
        assert!(!t.should_rewrite(300));
    }

    #[tokio::test]
    async fn replay_round_trip_with_binary_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin.aof");
        let payload: Vec<u8> = vec![0, 0xFF, b'\n', b'\r', 7];
        {
            let mut log = AofLog::open(&path, FsyncPolicy::Always).await.unwrap();
            log.append(&[b"SET", b"k", &payload]).await.unwrap();
        }
        let cmds = replay(&path).await.unwrap();
        assert_eq!(cmds[0][2], payload);
    }
}
