// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Server configuration for cave-cache.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    pub port: u16,
    pub max_memory_bytes: Option<usize>,
    pub eviction_policy: EvictionPolicy,
    pub databases: usize,
    pub hz: u64,
    pub aof_enabled: bool,
    pub aof_path: String,
    pub rdb_path: String,
    pub rdb_save_intervals: Vec<(u64, u64)>, // (seconds, changes)
    pub requirepass: Option<String>,
    pub slowlog_log_slower_than: i64, // microseconds, -1 = disabled
    pub slowlog_max_len: usize,
    pub maxclients: usize,
    pub tcp_backlog: u32,
    pub timeout: u64, // connection timeout seconds, 0 = disabled
    pub loglevel: LogLevel,
    pub notify_keyspace_events: String,
    pub cluster_enabled: bool,
    pub cluster_node_timeout: Duration,
    pub list_max_listpack_size: i64,
    pub hash_max_listpack_entries: usize,
    pub hash_max_listpack_value: usize,
    pub zset_max_listpack_entries: usize,
    pub zset_max_listpack_value: usize,
    pub set_max_intset_entries: usize,
    pub active_expire_enabled: bool,
    pub lazyfree_lazy_eviction: bool,
    pub lazyfree_lazy_expire: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bind: "0.0.0.0".into(),
            port: 6379,
            max_memory_bytes: None,
            eviction_policy: EvictionPolicy::NoEviction,
            databases: 16,
            hz: 10,
            aof_enabled: false,
            aof_path: "appendonly.aof".into(),
            rdb_path: "dump.rdb".into(),
            rdb_save_intervals: vec![(3600, 1), (300, 100), (60, 10000)],
            requirepass: None,
            slowlog_log_slower_than: 10000, // 10ms
            slowlog_max_len: 128,
            maxclients: 10000,
            tcp_backlog: 511,
            timeout: 0,
            loglevel: LogLevel::Notice,
            notify_keyspace_events: String::new(),
            cluster_enabled: false,
            cluster_node_timeout: Duration::from_millis(15000),
            list_max_listpack_size: -2,
            hash_max_listpack_entries: 128,
            hash_max_listpack_value: 64,
            zset_max_listpack_entries: 128,
            zset_max_listpack_value: 64,
            set_max_intset_entries: 512,
            active_expire_enabled: true,
            lazyfree_lazy_eviction: false,
            lazyfree_lazy_expire: false,
        }
    }
}

impl Config {
    pub fn addr(&self) -> String {
        format!("{}:{}", self.bind, self.port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    NoEviction,
    AllKeysLru,
    VolatileLru,
    AllKeysLfu,
    VolatileLfu,
    AllKeysRandom,
    VolatileRandom,
    VolatileTtl,
}

impl EvictionPolicy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "noeviction" => Some(EvictionPolicy::NoEviction),
            "allkeys-lru" => Some(EvictionPolicy::AllKeysLru),
            "volatile-lru" => Some(EvictionPolicy::VolatileLru),
            "allkeys-lfu" => Some(EvictionPolicy::AllKeysLfu),
            "volatile-lfu" => Some(EvictionPolicy::VolatileLfu),
            "allkeys-random" => Some(EvictionPolicy::AllKeysRandom),
            "volatile-random" => Some(EvictionPolicy::VolatileRandom),
            "volatile-ttl" => Some(EvictionPolicy::VolatileTtl),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            EvictionPolicy::NoEviction => "noeviction",
            EvictionPolicy::AllKeysLru => "allkeys-lru",
            EvictionPolicy::VolatileLru => "volatile-lru",
            EvictionPolicy::AllKeysLfu => "allkeys-lfu",
            EvictionPolicy::VolatileLfu => "volatile-lfu",
            EvictionPolicy::AllKeysRandom => "allkeys-random",
            EvictionPolicy::VolatileRandom => "volatile-random",
            EvictionPolicy::VolatileTtl => "volatile-ttl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Verbose,
    Notice,
    Warning,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Verbose => "verbose",
            LogLevel::Notice => "notice",
            LogLevel::Warning => "warning",
        }
    }
}

// ── Keyspace notification flags ───────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct NotifyFlags {
    pub keyspace: bool,
    pub keyevent: bool,
    pub generic: bool,
    pub string: bool,
    pub list: bool,
    pub set: bool,
    pub sorted_set: bool,
    pub hash: bool,
    pub expired: bool,
    pub evicted: bool,
    pub stream: bool,
    pub all: bool,
}

impl NotifyFlags {
    pub fn from_str(s: &str) -> Self {
        let mut f = NotifyFlags::default();
        for c in s.chars() {
            match c {
                'K' => f.keyspace = true,
                'E' => f.keyevent = true,
                'g' => f.generic = true,
                '$' => f.string = true,
                'l' => f.list = true,
                's' => f.set = true,
                'z' => f.sorted_set = true,
                'x' => f.expired = true,
                'd' => f.evicted = true,
                't' => f.stream = true,
                'h' => f.hash = true,
                'A' => f.all = true,
                _ => {}
            }
        }
        f
    }

    pub fn any_enabled(&self) -> bool {
        self.keyspace || self.keyevent || self.all
    }
}
