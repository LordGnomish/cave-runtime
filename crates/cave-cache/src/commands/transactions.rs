// SPDX-License-Identifier: AGPL-3.0-or-later
//! Transaction support: MULTI, EXEC, DISCARD, WATCH, UNWATCH.

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;

/// State for a connection currently in MULTI mode.
#[derive(Debug, Default)]
pub struct TransactionState {
    pub queued: Vec<Vec<Vec<u8>>>,
    pub error: Option<String>,
    pub watched_keys: Vec<WatchedKey>,
    pub aborted: bool,
}

#[derive(Debug, Clone)]
pub struct WatchedKey {
    pub key: Vec<u8>,
    pub version: u64,
    pub db_index: usize,
}

impl TransactionState {
    pub fn new() -> Self {
        TransactionState::default()
    }

    pub fn queue_command(&mut self, args: Vec<Vec<u8>>) {
        self.queued.push(args);
    }

    /// Check if any watched keys have been modified.
    pub fn is_dirty(&self, db: &Db, db_index: usize) -> bool {
        for wk in &self.watched_keys {
            if wk.db_index != db_index {
                continue;
            }
            match db.keys.get(wk.key.as_slice()) {
                Some(entry) => {
                    if entry.version != wk.version {
                        return true;
                    }
                }
                None => {
                    // Key was deleted
                    if wk.version != 0 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

pub fn cmd_multi() -> CacheResult<Resp> {
    Ok(Resp::ok())
}

pub fn cmd_discard() -> CacheResult<Resp> {
    Ok(Resp::ok())
}

pub fn cmd_watch(_args: &[Vec<u8>]) -> CacheResult<Resp> {
    Ok(Resp::ok())
}

pub fn cmd_unwatch() -> CacheResult<Resp> {
    Ok(Resp::ok())
}
