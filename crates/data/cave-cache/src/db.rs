// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core database structures and shared server state.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{RwLock, broadcast, mpsc};

use crate::acl::AclState;
use crate::cluster::ClusterState;
use crate::config::{Config, EvictionPolicy};
use crate::error::{CacheError, CacheResult};
use crate::keyspace::KeyspaceEvent;
use crate::types::{Entry, Value};

// ── Database ──────────────────────────────────────────────────────────────────

/// A single Redis database (one of 16 by default).
#[derive(Debug, Default)]
pub struct Db {
    pub keys: HashMap<Vec<u8>, Entry>,
    /// Waiters for blocking list/zset operations: key → list of senders
    pub blocked_pops: HashMap<Vec<u8>, Vec<BlockedPop>>,
}

#[derive(Debug)]
pub struct BlockedPop {
    pub tx: mpsc::Sender<(Vec<u8>, Vec<u8>)>, // (key, value)
    pub from_right: bool,                     // true = BRPOP
}

impl Db {
    pub fn new() -> Self {
        Db::default()
    }

    /// Get a key if it exists and is not expired. Removes expired key lazily.
    pub fn get(&mut self, key: &[u8]) -> Option<&Entry> {
        if let Some(entry) = self.keys.get(key) {
            if entry.is_expired() {
                self.keys.remove(key);
                return None;
            }
        }
        self.keys.get(key)
    }

    pub fn get_mut(&mut self, key: &[u8]) -> Option<&mut Entry> {
        if let Some(entry) = self.keys.get(key) {
            if entry.is_expired() {
                self.keys.remove(key);
                return None;
            }
        }
        self.keys.get_mut(key)
    }

    /// Get a key with a specific expected type, returning WrongType if mismatched.
    pub fn get_typed(&mut self, key: &[u8], expected: &str) -> CacheResult<Option<&Entry>> {
        match self.get(key) {
            Some(e) => {
                if e.value.type_name() != expected {
                    Err(CacheError::WrongType)
                } else {
                    Ok(Some(self.keys.get(key).unwrap()))
                }
            }
            None => Ok(None),
        }
    }

    pub fn get_typed_mut(&mut self, key: &[u8], expected: &str) -> CacheResult<Option<&mut Entry>> {
        // Check expiry and type first
        let expired = self.keys.get(key).map(|e| e.is_expired()).unwrap_or(false);
        if expired {
            self.keys.remove(key);
        }

        match self.keys.get(key) {
            Some(e) if e.value.type_name() != expected => Err(CacheError::WrongType),
            Some(_) => Ok(self.keys.get_mut(key)),
            None => Ok(None),
        }
    }

    pub fn insert(&mut self, key: Vec<u8>, mut entry: Entry) {
        // Increment version for WATCH dirty detection
        let prev_version = self.keys.get(&key).map(|e| e.version).unwrap_or(0);
        entry.version = prev_version + 1;
        self.keys.insert(key, entry);
    }

    pub fn remove(&mut self, key: &[u8]) -> bool {
        self.keys.remove(key).is_some()
    }

    pub fn exists(&mut self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    pub fn dbsize(&self) -> usize {
        // Approximate (includes expired keys not yet lazily removed)
        self.keys.len()
    }

    pub fn flush(&mut self) {
        self.keys.clear();
        self.blocked_pops.clear();
    }

    /// Collect all expired keys and remove them.
    pub fn expire_cycle(&mut self) -> Vec<Vec<u8>> {
        let now = Instant::now();
        let expired: Vec<Vec<u8>> = self
            .keys
            .iter()
            .filter(|(_, e)| e.expires_at.map(|t| t <= now).unwrap_or(false))
            .map(|(k, _)| k.clone())
            .collect();
        for k in &expired {
            self.keys.remove(k);
        }
        expired
    }

    /// Wake up any blocked clients waiting on this key.
    pub fn notify_blocked(&mut self, key: &[u8]) {
        let _ = self.blocked_pops.get(key); // just check existence
        // Actual waking happens via the blocking command implementations
    }
}

// ── Pub/Sub Registry ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct PubSubRegistry {
    /// channel → list of (client_id, sender)
    pub channels: HashMap<Vec<u8>, Vec<(u64, mpsc::UnboundedSender<PubSubMessage>)>>,
    /// pattern subscribers: (pattern, client_id, sender)
    pub patterns: Vec<(Vec<u8>, u64, mpsc::UnboundedSender<PubSubMessage>)>,
    /// shard channel subscribers (cluster shard pub/sub)
    pub shard_channels: HashMap<Vec<u8>, Vec<(u64, mpsc::UnboundedSender<PubSubMessage>)>>,
}

#[derive(Debug, Clone)]
pub struct PubSubMessage {
    pub kind: PubSubKind,
    pub channel: Vec<u8>,
    pub pattern: Option<Vec<u8>>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubSubKind {
    Message,
    PMessage,
    Subscribe,
    Unsubscribe,
    PSubscribe,
    PUnsubscribe,
    SSubscribe,
    SUnsubscribe,
}

impl PubSubRegistry {
    pub fn subscribe(
        &mut self,
        client_id: u64,
        channel: Vec<u8>,
        tx: mpsc::UnboundedSender<PubSubMessage>,
    ) {
        self.channels
            .entry(channel)
            .or_default()
            .push((client_id, tx));
    }

    pub fn psubscribe(
        &mut self,
        client_id: u64,
        pattern: Vec<u8>,
        tx: mpsc::UnboundedSender<PubSubMessage>,
    ) {
        self.patterns.push((pattern, client_id, tx));
    }

    pub fn unsubscribe(&mut self, client_id: u64, channel: &[u8]) {
        if let Some(subs) = self.channels.get_mut(channel) {
            subs.retain(|(id, _)| *id != client_id);
            if subs.is_empty() {
                self.channels.remove(channel);
            }
        }
    }

    pub fn punsubscribe(&mut self, client_id: u64, pattern: &[u8]) {
        self.patterns
            .retain(|(p, id, _)| !(*id == client_id && p.as_slice() == pattern));
    }

    pub fn unsubscribe_all(&mut self, client_id: u64) {
        for subs in self.channels.values_mut() {
            subs.retain(|(id, _)| *id != client_id);
        }
        self.channels.retain(|_, v| !v.is_empty());
        self.patterns.retain(|(_, id, _)| *id != client_id);
    }

    pub fn channel_count(&self, client_id: u64) -> usize {
        self.channels
            .values()
            .filter(|subs| subs.iter().any(|(id, _)| *id == client_id))
            .count()
    }

    pub fn pattern_count(&self, client_id: u64) -> usize {
        self.patterns
            .iter()
            .filter(|(_, id, _)| *id == client_id)
            .count()
    }

    /// Publish to channel. Returns number of receivers.
    pub fn publish(&self, channel: &[u8], message: &[u8]) -> usize {
        let mut count = 0;

        // Exact channel subscribers
        if let Some(subs) = self.channels.get(channel) {
            for (_, tx) in subs {
                let msg = PubSubMessage {
                    kind: PubSubKind::Message,
                    channel: channel.to_vec(),
                    pattern: None,
                    data: message.to_vec(),
                };
                if tx.send(msg).is_ok() {
                    count += 1;
                }
            }
        }

        // Pattern subscribers
        for (pattern, _, tx) in &self.patterns {
            if glob_match(pattern, channel) {
                let msg = PubSubMessage {
                    kind: PubSubKind::PMessage,
                    channel: channel.to_vec(),
                    pattern: Some(pattern.clone()),
                    data: message.to_vec(),
                };
                if tx.send(msg).is_ok() {
                    count += 1;
                }
            }
        }

        count
    }

    pub fn active_channels(&self) -> Vec<Vec<u8>> {
        self.channels
            .iter()
            .filter(|(_, subs)| !subs.is_empty())
            .map(|(ch, _)| ch.clone())
            .collect()
    }

    pub fn numsub(&self, channels: &[Vec<u8>]) -> Vec<(Vec<u8>, usize)> {
        channels
            .iter()
            .map(|ch| {
                let count = self
                    .channels
                    .get(ch.as_slice())
                    .map(|s| s.len())
                    .unwrap_or(0);
                (ch.clone(), count)
            })
            .collect()
    }
}

/// Simple glob pattern matching (only * and ? wildcards).
pub fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    glob_match_inner(pattern, text)
}

fn glob_match_inner(pat: &[u8], s: &[u8]) -> bool {
    let mut pi = 0;
    let mut si = 0;
    let mut star_pi: Option<usize> = None;
    let mut star_si = 0;

    while si < s.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_pi = Some(pi);
            star_si = si;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

// ── Script store ──────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct ScriptStore {
    /// sha1hex → script source
    pub scripts: HashMap<String, String>,
}

impl ScriptStore {
    pub fn load(&mut self, script: String) -> String {
        let sha = sha1_hex(script.as_bytes());
        self.scripts.insert(sha.clone(), script);
        sha
    }

    pub fn exists(&self, sha: &str) -> bool {
        self.scripts.contains_key(sha)
    }

    pub fn flush(&mut self) {
        self.scripts.clear();
    }
}

fn sha1_hex(data: &[u8]) -> String {
    use ring::digest;
    let digest = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, data);
    hex_encode(digest.as_ref())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── Slow log ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SlowlogEntry {
    pub id: u64,
    pub timestamp: u64,
    pub duration_us: u64,
    pub args: Vec<Vec<u8>>,
    pub client_addr: String,
    pub client_name: String,
}

// ── Shared server state ───────────────────────────────────────────────────────

pub struct ServerState {
    pub config: Arc<RwLock<Config>>,
    pub dbs: Vec<Arc<RwLock<Db>>>,
    pub pubsub: Arc<RwLock<PubSubRegistry>>,
    pub scripts: Arc<RwLock<ScriptStore>>,
    pub acl: Arc<RwLock<AclState>>,
    pub cluster: Arc<ClusterState>,
    pub slowlog: Arc<tokio::sync::Mutex<VecDeque<SlowlogEntry>>>,
    pub keyspace_tx: broadcast::Sender<KeyspaceEvent>,
    pub start_time: Instant,
    pub start_time_unix: u64,
    pub connected_clients: Arc<AtomicU64>,
    pub total_commands: Arc<AtomicU64>,
    pub total_connections: Arc<AtomicU64>,
    pub next_client_id: Arc<AtomicU64>,
    pub dirty: Arc<AtomicU64>, // number of changes since last save
}

impl ServerState {
    pub fn new(config: Config) -> Arc<Self> {
        let num_dbs = config.databases;
        let (keyspace_tx, _) = broadcast::channel(4096);
        let dbs = (0..num_dbs)
            .map(|_| Arc::new(RwLock::new(Db::new())))
            .collect();

        Arc::new(ServerState {
            config: Arc::new(RwLock::new(config)),
            dbs,
            pubsub: Arc::new(RwLock::new(PubSubRegistry::default())),
            scripts: Arc::new(RwLock::new(ScriptStore::default())),
            acl: Arc::new(RwLock::new(AclState::default())),
            cluster: Arc::new(ClusterState::new()),
            slowlog: Arc::new(tokio::sync::Mutex::new(VecDeque::new())),
            keyspace_tx,
            start_time: Instant::now(),
            start_time_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            connected_clients: Arc::new(AtomicU64::new(0)),
            total_commands: Arc::new(AtomicU64::new(0)),
            total_connections: Arc::new(AtomicU64::new(0)),
            next_client_id: Arc::new(AtomicU64::new(1)),
            dirty: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn next_client_id(&self) -> u64 {
        self.next_client_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn uptime_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    pub async fn record_slowlog(&self, entry: SlowlogEntry) {
        let max_len = {
            let cfg = self.config.read().await;
            cfg.slowlog_max_len
        };
        let mut log = self.slowlog.lock().await;
        log.push_front(entry);
        while log.len() > max_len {
            log.pop_back();
        }
    }

    /// Emit a keyspace notification.
    pub fn notify(&self, db: usize, event: &str, key: &[u8]) {
        let _ = self.keyspace_tx.send(KeyspaceEvent {
            db,
            event: event.to_string(),
            key: key.to_vec(),
        });
    }

    /// Flush all databases.
    pub async fn flushall(&self) {
        for db in &self.dbs {
            db.write().await.flush();
        }
    }
}
