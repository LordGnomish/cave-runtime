// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ACL log — persistent audit trail of authentication and authorization
//! violations.
//!
//! Ports the in-memory `ACL LOG` ring buffer behind `src/acl.c` (`ACLLogEntry`
//! / `ACLLog*` helpers) and the persistence sidecar added in Redis 8.0 /
//! Valkey 8.0 — earlier versions of the cave-cache ACL surface kept this
//! ring purely in-memory; this module gives operators an on-disk JSONL
//! tail that survives restart, with `ACL LOG RESET` truncating both the
//! buffer and the backing file.

use crate::acl::AclUser;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AclLogReason {
    AuthFailure,
    KeyAccessDenied,
    ChannelAccessDenied,
    CommandAccessDenied,
}

impl AclLogReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            AclLogReason::AuthFailure => "auth",
            AclLogReason::KeyAccessDenied => "key",
            AclLogReason::ChannelAccessDenied => "channel",
            AclLogReason::CommandAccessDenied => "command",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "auth" => Some(Self::AuthFailure),
            "key" => Some(Self::KeyAccessDenied),
            "channel" => Some(Self::ChannelAccessDenied),
            "command" => Some(Self::CommandAccessDenied),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclLogEntry {
    pub username: String,
    pub reason: AclLogReason,
    pub context: String,
    pub object: String,
    pub timestamp_ms: u64,
    pub client_info: Option<String>,
}

impl AclLogEntry {
    pub fn new(user: &AclUser, reason: AclLogReason, object: impl Into<String>) -> Self {
        AclLogEntry {
            username: user.name.clone(),
            reason,
            context: "default".into(),
            object: object.into(),
            timestamp_ms: now_ms(),
            client_info: None,
        }
    }

    fn to_line(&self) -> String {
        let client = self.client_info.as_deref().unwrap_or("");
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            self.timestamp_ms,
            escape(&self.username),
            self.reason.as_str(),
            escape(&self.context),
            escape(&self.object),
            escape(client),
        )
    }

    fn parse(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.trim_end_matches('\n').splitn(6, '\t').collect();
        if parts.len() < 5 {
            return None;
        }
        Some(AclLogEntry {
            timestamp_ms: parts[0].parse().ok()?,
            username: unescape(parts[1]),
            reason: AclLogReason::from_str(parts[2])?,
            context: unescape(parts[3]),
            object: unescape(parts[4]),
            client_info: parts.get(5).map(|s| unescape(s)).filter(|s| !s.is_empty()),
        })
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('\t', r"\t").replace('\n', r"\n")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Persistent, capacity-bounded ACL log.  Writes append to a JSONL-ish
/// tab-delimited file; reads return entries newest-first to mirror
/// upstream's `ACL LOG <count>` semantics.
#[derive(Debug)]
pub struct AclLog {
    path: Option<PathBuf>,
    capacity: usize,
    entries: Mutex<Vec<AclLogEntry>>,
}

impl AclLog {
    /// In-memory-only ACL log with no disk backing.
    pub fn in_memory(capacity: usize) -> Self {
        AclLog {
            path: None,
            capacity: capacity.max(1),
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Open or create the on-disk ACL log file at `path`.  Existing entries
    /// are loaded; new appends are persisted synchronously.
    pub fn open(path: impl AsRef<Path>, capacity: usize) -> std::io::Result<Self> {
        let p = path.as_ref().to_path_buf();
        let mut entries: Vec<AclLogEntry> = Vec::new();
        if p.exists() {
            let f = File::open(&p)?;
            for line in BufReader::new(f).lines().map_while(Result::ok) {
                if let Some(e) = AclLogEntry::parse(&line) {
                    entries.push(e);
                }
            }
            // Capacity trim — keep the *newest* tail.
            if entries.len() > capacity {
                let drop = entries.len() - capacity;
                entries.drain(0..drop);
            }
        } else if let Some(dir) = p.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        Ok(AclLog {
            path: Some(p),
            capacity: capacity.max(1),
            entries: Mutex::new(entries),
        })
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Append a new entry. Persists to disk if backed by a file. When the
    /// buffer is full, the oldest entry is evicted (LRU on age).
    pub fn append(&self, entry: AclLogEntry) -> std::io::Result<()> {
        let mut guard = self.entries.lock().unwrap();
        if guard.len() >= self.capacity {
            guard.remove(0);
        }
        guard.push(entry.clone());
        if let Some(path) = &self.path {
            let mut f = OpenOptions::new().create(true).append(true).open(path)?;
            f.write_all(entry.to_line().as_bytes())?;
        }
        Ok(())
    }

    /// `ACL LOG <count>` — newest-first, up to `count`.
    pub fn tail(&self, count: usize) -> Vec<AclLogEntry> {
        let guard = self.entries.lock().unwrap();
        guard.iter().rev().take(count).cloned().collect()
    }

    /// `ACL LOG RESET` — clears the buffer AND truncates the disk file.
    pub fn reset(&self) -> std::io::Result<()> {
        let mut guard = self.entries.lock().unwrap();
        guard.clear();
        if let Some(path) = &self.path {
            if path.exists() {
                File::create(path)?; // truncate
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::AclUser;
    use std::env;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("cave-cache-acl-log-{}-{}.log", name, now_ms()));
        p
    }

    fn fake_user(name: &str) -> AclUser {
        let mut u = AclUser::default_user();
        u.name = name.into();
        u
    }

    #[test]
    fn in_memory_append_and_tail() {
        let log = AclLog::in_memory(10);
        log.append(AclLogEntry::new(
            &fake_user("u1"),
            AclLogReason::AuthFailure,
            "wrong-password",
        ))
        .unwrap();
        log.append(AclLogEntry::new(
            &fake_user("u2"),
            AclLogReason::KeyAccessDenied,
            "secret:42",
        ))
        .unwrap();
        let entries = log.tail(10);
        assert_eq!(entries.len(), 2);
        // newest-first
        assert_eq!(entries[0].username, "u2");
        assert_eq!(entries[1].username, "u1");
    }

    #[test]
    fn capacity_evicts_oldest() {
        let log = AclLog::in_memory(2);
        for i in 0..5 {
            log.append(AclLogEntry::new(
                &fake_user(&format!("u{i}")),
                AclLogReason::CommandAccessDenied,
                format!("CMD{i}"),
            ))
            .unwrap();
        }
        assert_eq!(log.len(), 2);
        let entries = log.tail(10);
        assert_eq!(entries[0].username, "u4");
        assert_eq!(entries[1].username, "u3");
    }

    #[test]
    fn persistence_round_trip() {
        let path = temp_path("rt");
        let _ = std::fs::remove_file(&path);
        {
            let log = AclLog::open(&path, 100).unwrap();
            log.append(AclLogEntry::new(
                &fake_user("alice"),
                AclLogReason::ChannelAccessDenied,
                "alerts.*",
            ))
            .unwrap();
            log.append(AclLogEntry::new(
                &fake_user("bob"),
                AclLogReason::AuthFailure,
                "no-password",
            ))
            .unwrap();
        }
        let log2 = AclLog::open(&path, 100).unwrap();
        assert_eq!(log2.len(), 2);
        let entries = log2.tail(10);
        assert_eq!(entries[0].username, "bob");
        assert_eq!(entries[0].reason, AclLogReason::AuthFailure);
        assert_eq!(entries[1].username, "alice");
        assert_eq!(entries[1].reason, AclLogReason::ChannelAccessDenied);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reset_truncates_file() {
        let path = temp_path("reset");
        let _ = std::fs::remove_file(&path);
        let log = AclLog::open(&path, 100).unwrap();
        log.append(AclLogEntry::new(
            &fake_user("c"),
            AclLogReason::AuthFailure,
            "x",
        ))
        .unwrap();
        assert!(log.len() > 0);
        log.reset().unwrap();
        assert_eq!(log.len(), 0);
        // Reopening shows the disk is also empty.
        let log2 = AclLog::open(&path, 100).unwrap();
        assert_eq!(log2.len(), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn escape_round_trip_handles_tabs_and_newlines() {
        let raw = "alice\twith\ttabs\nand\\back";
        let escaped = escape(raw);
        assert!(!escaped.contains('\t'));
        assert!(!escaped.contains('\n'));
        assert_eq!(unescape(&escaped), raw);
    }

    #[test]
    fn capacity_trim_on_load_keeps_newest() {
        let path = temp_path("trim");
        let _ = std::fs::remove_file(&path);
        {
            let log = AclLog::open(&path, 100).unwrap();
            for i in 0..10 {
                log.append(AclLogEntry::new(
                    &fake_user(&format!("u{i}")),
                    AclLogReason::AuthFailure,
                    format!("o{i}"),
                ))
                .unwrap();
            }
        }
        let log2 = AclLog::open(&path, 3).unwrap();
        assert_eq!(log2.len(), 3);
        let entries = log2.tail(10);
        assert_eq!(entries[0].username, "u9");
        assert_eq!(entries[1].username, "u8");
        assert_eq!(entries[2].username, "u7");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reason_round_trip() {
        for r in [
            AclLogReason::AuthFailure,
            AclLogReason::KeyAccessDenied,
            AclLogReason::ChannelAccessDenied,
            AclLogReason::CommandAccessDenied,
        ] {
            assert_eq!(AclLogReason::from_str(r.as_str()), Some(r.clone()));
        }
    }
}
