// SPDX-License-Identifier: AGPL-3.0-or-later
//! Core value types for cave-cache.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::Instant;

// ── Value ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Value {
    String(Vec<u8>),
    List(VecDeque<Vec<u8>>),
    Set(HashSet<Vec<u8>>),
    ZSet(ZSet),
    Hash(HashMap<Vec<u8>, Vec<u8>>),
    Stream(Stream),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::String(_) => "string",
            Value::List(_) => "list",
            Value::Set(_) => "set",
            Value::ZSet(_) => "zset",
            Value::Hash(_) => "hash",
            Value::Stream(_) => "stream",
        }
    }
}

// ── Entry ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Entry {
    pub value: Value,
    pub expires_at: Option<Instant>,
    /// Monotonic clock tick for LRU tracking.
    pub lru_clock: u64,
    /// Log-counter for LFU tracking (0-255).
    pub lfu_freq: u8,
    /// Version counter incremented on every write (for WATCH).
    pub version: u64,
}

impl Entry {
    pub fn new(value: Value) -> Self {
        Entry {
            value,
            expires_at: None,
            lru_clock: lru_clock_now(),
            lfu_freq: 0,
            version: 1,
        }
    }

    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(t) => Instant::now() >= t,
            None => false,
        }
    }

    /// Returns remaining TTL in milliseconds, None if no expiry.
    pub fn pttl(&self) -> Option<i64> {
        self.expires_at.map(|t| {
            let now = Instant::now();
            if now >= t {
                -2
            } else {
                (t - now).as_millis() as i64
            }
        })
    }

    pub fn touch(&mut self) {
        self.lru_clock = lru_clock_now();
        self.lfu_freq = lfu_increment(self.lfu_freq);
    }
}

fn lru_clock_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn lfu_increment(freq: u8) -> u8 {
    // Log-counter increment with saturation
    if freq == 255 {
        return 255;
    }
    let r: f64 = rand::random();
    let p = 1.0 / ((freq as f64 - 0.0) * 10.0 + 1.0);
    if r < p {
        freq + 1
    } else {
        freq
    }
}

// ── ZSet ─────────────────────────────────────────────────────────────────────

/// Sorted set: maintains both a score-ordered BTreeMap and a member→score HashMap.
#[derive(Debug, Clone, Default)]
pub struct ZSet {
    /// (score bits, member) → () for ordered iteration
    pub ordered: BTreeMap<ZKey, ()>,
    /// member → score for O(1) lookup
    pub scores: HashMap<Vec<u8>, f64>,
}

impl ZSet {
    pub fn new() -> Self {
        ZSet::default()
    }

    /// Returns true if the member was added (false if updated).
    pub fn add(&mut self, member: Vec<u8>, score: f64) -> bool {
        if let Some(&old_score) = self.scores.get(&member) {
            // Remove old position
            self.ordered.remove(&ZKey { score: old_score, member: member.clone() });
            self.scores.insert(member.clone(), score);
            self.ordered.insert(ZKey { score, member }, ());
            false
        } else {
            self.scores.insert(member.clone(), score);
            self.ordered.insert(ZKey { score, member }, ());
            true
        }
    }

    pub fn remove(&mut self, member: &[u8]) -> bool {
        if let Some(score) = self.scores.remove(member) {
            self.ordered.remove(&ZKey { score, member: member.to_vec() });
            true
        } else {
            false
        }
    }

    pub fn score(&self, member: &[u8]) -> Option<f64> {
        self.scores.get(member).copied()
    }

    pub fn len(&self) -> usize {
        self.scores.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }

    /// Rank (0-based) of member by ascending score.
    pub fn rank(&self, member: &[u8]) -> Option<usize> {
        let score = self.scores.get(member)?;
        let key = ZKey { score: *score, member: member.to_vec() };
        let rank = self.ordered.range(..=key).count() - 1;
        Some(rank)
    }

    /// Rank (0-based) of member by descending score.
    pub fn rev_rank(&self, member: &[u8]) -> Option<usize> {
        let score = self.scores.get(member)?;
        let key = ZKey { score: *score, member: member.to_vec() };
        let rank = self.ordered.range(key..).count() - 1;
        Some(rank)
    }

    /// Entries in score range [min, max], optionally with lex bounds within the same score.
    pub fn range_by_score(&self, min: f64, max: f64) -> Vec<(Vec<u8>, f64)> {
        let lo = ZKey { score: min, member: vec![] };
        let hi = ZKey { score: max, member: vec![255u8; 64] };
        self.ordered
            .range(lo..=hi)
            .map(|(k, _)| (k.member.clone(), k.score))
            .collect()
    }

    /// Range by index [start, stop] ascending.
    pub fn range_by_index(&self, start: isize, stop: isize) -> Vec<(Vec<u8>, f64)> {
        let len = self.len() as isize;
        let start = normalize_index(start, len);
        let stop = normalize_index(stop, len);
        if start > stop || start >= len as usize {
            return vec![];
        }
        self.ordered
            .keys()
            .skip(start)
            .take(stop - start + 1)
            .map(|k| (k.member.clone(), k.score))
            .collect()
    }

    /// Range by index [start, stop] descending.
    pub fn rev_range_by_index(&self, start: isize, stop: isize) -> Vec<(Vec<u8>, f64)> {
        let len = self.len() as isize;
        let start = normalize_index(start, len);
        let stop = normalize_index(stop, len);
        if start > stop || start >= len as usize {
            return vec![];
        }
        let all: Vec<_> = self.ordered.keys().collect();
        let total = all.len();
        all.iter()
            .rev()
            .skip(start)
            .take(stop - start + 1)
            .map(|k| (k.member.clone(), k.score))
            .collect()
    }

    /// Pop min element.
    pub fn pop_min(&mut self) -> Option<(Vec<u8>, f64)> {
        let key = self.ordered.keys().next()?.clone();
        self.ordered.remove(&key);
        self.scores.remove(&key.member);
        Some((key.member, key.score))
    }

    /// Pop max element.
    pub fn pop_max(&mut self) -> Option<(Vec<u8>, f64)> {
        let key = self.ordered.keys().next_back()?.clone();
        self.ordered.remove(&key);
        self.scores.remove(&key.member);
        Some((key.member, key.score))
    }

    /// Count members with score in [min, max].
    pub fn count_in_range(&self, min: f64, max: f64) -> usize {
        self.range_by_score(min, max).len()
    }

    /// Lex range within the same score (for ZRANGEBYLEX). All members must have score 0.
    pub fn range_by_lex(&self, min: &LexBound, max: &LexBound) -> Vec<(Vec<u8>, f64)> {
        self.ordered
            .keys()
            .filter(|k| lex_in_range(&k.member, min, max))
            .map(|k| (k.member.clone(), k.score))
            .collect()
    }

    pub fn count_by_lex(&self, min: &LexBound, max: &LexBound) -> usize {
        self.ordered.keys().filter(|k| lex_in_range(&k.member, min, max)).count()
    }

    pub fn incr_score(&mut self, member: Vec<u8>, delta: f64) -> f64 {
        let current = self.scores.get(&member).copied().unwrap_or(0.0);
        let new_score = current + delta;
        self.add(member, new_score);
        new_score
    }

    pub fn iter_asc(&self) -> impl Iterator<Item = (&Vec<u8>, f64)> {
        self.ordered.keys().map(|k| (&k.member, k.score))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZKey {
    pub score: f64,
    pub member: Vec<u8>,
}

impl Eq for ZKey {}

impl PartialOrd for ZKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ZKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let sa = if self.score.is_nan() { f64::NEG_INFINITY } else { self.score };
        let sb = if other.score.is_nan() { f64::NEG_INFINITY } else { other.score };
        sa.partial_cmp(&sb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| self.member.cmp(&other.member))
    }
}

// ── LexBound ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum LexBound {
    Unbounded,
    Inclusive(Vec<u8>),
    Exclusive(Vec<u8>),
}

impl LexBound {
    pub fn parse(s: &[u8]) -> Option<Self> {
        match s.first()? {
            b'-' if s.len() == 1 => Some(LexBound::Unbounded),
            b'+' if s.len() == 1 => Some(LexBound::Unbounded),
            b'[' => Some(LexBound::Inclusive(s[1..].to_vec())),
            b'(' => Some(LexBound::Exclusive(s[1..].to_vec())),
            _ => None,
        }
    }
}

fn lex_in_range(member: &[u8], min: &LexBound, max: &LexBound) -> bool {
    let above_min = match min {
        LexBound::Unbounded => true,
        LexBound::Inclusive(m) => member >= m.as_slice(),
        LexBound::Exclusive(m) => member > m.as_slice(),
    };
    let below_max = match max {
        LexBound::Unbounded => true,
        LexBound::Inclusive(m) => member <= m.as_slice(),
        LexBound::Exclusive(m) => member < m.as_slice(),
    };
    above_min && below_max
}

// ── ScoreBound ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum ScoreBound {
    NegInf,
    PosInf,
    Inclusive(f64),
    Exclusive(f64),
}

impl ScoreBound {
    pub fn parse(s: &[u8]) -> Option<Self> {
        match s {
            b"-inf" => Some(ScoreBound::NegInf),
            b"+inf" | b"inf" => Some(ScoreBound::PosInf),
            _ if s.first() == Some(&b'(') => {
                let v: f64 = std::str::from_utf8(&s[1..]).ok()?.parse().ok()?;
                Some(ScoreBound::Exclusive(v))
            }
            _ => {
                let v: f64 = std::str::from_utf8(s).ok()?.parse().ok()?;
                Some(ScoreBound::Inclusive(v))
            }
        }
    }

    pub fn min_f64(&self) -> f64 {
        match self {
            ScoreBound::NegInf => f64::NEG_INFINITY,
            ScoreBound::PosInf => f64::INFINITY,
            ScoreBound::Inclusive(v) | ScoreBound::Exclusive(v) => *v,
        }
    }

    pub fn contains_min(&self, score: f64) -> bool {
        match self {
            ScoreBound::NegInf => true,
            ScoreBound::PosInf => false,
            ScoreBound::Inclusive(v) => score >= *v,
            ScoreBound::Exclusive(v) => score > *v,
        }
    }

    pub fn contains_max(&self, score: f64) -> bool {
        match self {
            ScoreBound::NegInf => false,
            ScoreBound::PosInf => true,
            ScoreBound::Inclusive(v) => score <= *v,
            ScoreBound::Exclusive(v) => score < *v,
        }
    }
}

// ── Stream ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Stream {
    pub entries: Vec<StreamEntry>,
    pub groups: HashMap<Vec<u8>, ConsumerGroup>,
    pub last_id: StreamId,
    pub max_len: Option<usize>,
}

impl Stream {
    pub fn new() -> Self {
        Stream {
            entries: Vec::new(),
            groups: HashMap::new(),
            last_id: StreamId { ms: 0, seq: 0 },
            max_len: None,
        }
    }
}

impl Default for Stream {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct StreamEntry {
    pub id: StreamId,
    pub fields: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId {
    pub ms: u64,
    pub seq: u64,
}

impl StreamId {
    pub fn zero() -> Self { StreamId { ms: 0, seq: 0 } }

    pub fn parse(s: &[u8]) -> Option<Self> {
        let s = std::str::from_utf8(s).ok()?;
        if s == "*" {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            return Some(StreamId { ms, seq: 0 });
        }
        if let Some(pos) = s.find('-') {
            let ms: u64 = s[..pos].parse().ok()?;
            let seq: u64 = if s[pos + 1..] == *"*" { 0 } else { s[pos + 1..].parse().ok()? };
            Some(StreamId { ms, seq })
        } else {
            let ms: u64 = s.parse().ok()?;
            Some(StreamId { ms, seq: 0 })
        }
    }

    pub fn to_string(&self) -> String {
        format!("{}-{}", self.ms, self.seq)
    }
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.ms, self.seq)
    }
}

#[derive(Debug, Clone)]
pub struct ConsumerGroup {
    pub name: Vec<u8>,
    pub last_delivered_id: StreamId,
    pub consumers: HashMap<Vec<u8>, Consumer>,
    /// PEL: pending entries list (id -> PendingEntry)
    pub pel: HashMap<StreamId, PendingEntry>,
}

#[derive(Debug, Clone)]
pub struct Consumer {
    pub name: Vec<u8>,
    pub seen_time: u64,
    pub active_time: u64,
    pub pel: Vec<StreamId>,
}

#[derive(Debug, Clone)]
pub struct PendingEntry {
    pub id: StreamId,
    pub consumer: Vec<u8>,
    pub delivery_time: u64,
    pub delivery_count: u64,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn normalize_index(idx: isize, len: isize) -> usize {
    if idx < 0 {
        let adjusted = len + idx;
        if adjusted < 0 { 0 } else { adjusted as usize }
    } else {
        idx as usize
    }
}

pub fn bytes_to_i64(b: &[u8]) -> Option<i64> {
    std::str::from_utf8(b).ok()?.trim().parse().ok()
}

pub fn bytes_to_f64(b: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(b).ok()?.trim();
    match s {
        "-inf" | "-Inf" | "-INF" => Some(f64::NEG_INFINITY),
        "inf" | "+inf" | "Inf" | "+Inf" | "INF" | "+INF" => Some(f64::INFINITY),
        _ => s.parse().ok(),
    }
}

pub fn i64_to_bytes(n: i64) -> Vec<u8> {
    n.to_string().into_bytes()
}

pub fn f64_to_bytes(f: f64) -> Vec<u8> {
    if f == f64::NEG_INFINITY { return b"-inf".to_vec(); }
    if f == f64::INFINITY { return b"inf".to_vec(); }
    // Use minimal representation
    let s = format!("{}", f);
    s.into_bytes()
}
