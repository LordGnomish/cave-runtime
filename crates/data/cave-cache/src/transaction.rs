// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::time::Duration;
use crate::engine::CacheEngine;
use crate::types::{CacheError, CacheResult};

#[allow(dead_code)]
pub enum TxCommand {
    Get(String),
    Set(String, Vec<u8>, Option<Duration>),
    Del(Vec<String>),
    Incr(String),
    Decr(String),
    Expire(String, u64),
    Lpush(String, Vec<Vec<u8>>),
    Rpush(String, Vec<Vec<u8>>),
    Sadd(String, Vec<Vec<u8>>),
    Hset(String, Vec<u8>, Vec<u8>),
}

pub struct Transaction {
    pub commands: Vec<TxCommand>,
    pub watched_keys: Vec<String>,
    pub aborted: bool,
    pub(crate) watch_versions: Vec<(String, u64)>,
}

impl CacheEngine {
    pub fn multi(&self) -> Transaction {
        Transaction {
            commands: vec![],
            watched_keys: vec![],
            aborted: false,
            watch_versions: vec![],
        }
    }

    pub fn watch(&self, tx: &mut Transaction, keys: &[&str]) {
        let store = self.store.lock().unwrap();
        for key in keys {
            let version = store.get(*key).map(|e| e.version).unwrap_or(0);
            tx.watched_keys.push(key.to_string());
            tx.watch_versions.push((key.to_string(), version));
        }
    }

    pub fn discard(&self, tx: Transaction) {
        drop(tx);
    }

    pub fn exec(&self, tx: Transaction) -> CacheResult<Vec<serde_json::Value>> {
        if tx.aborted {
            return Err(CacheError::TxAborted);
        }

        // Check if any watched keys have changed
        {
            let store = self.store.lock().unwrap();
            for (key, orig_version) in &tx.watch_versions {
                let current_version = store.get(key).map(|e| e.version).unwrap_or(0);
                if current_version != *orig_version {
                    return Err(CacheError::TxAborted);
                }
            }
        }

        // Execute commands
        let mut results = Vec::new();
        for cmd in tx.commands {
            let val = match cmd {
                TxCommand::Get(key) => {
                    let v = self.get(&key)?;
                    match v {
                        None => serde_json::Value::Null,
                        Some(bytes) => serde_json::Value::String(
                            String::from_utf8(bytes).unwrap_or_default(),
                        ),
                    }
                }
                TxCommand::Set(key, value, ex) => {
                    self.set(&key, value, ex)?;
                    serde_json::Value::String("OK".to_string())
                }
                TxCommand::Del(keys) => {
                    let key_refs: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();
                    let n = self.del(&key_refs);
                    serde_json::Value::Number(n.into())
                }
                TxCommand::Incr(key) => {
                    let n = self.incr(&key)?;
                    serde_json::Value::Number(n.into())
                }
                TxCommand::Decr(key) => {
                    let n = self.decr(&key)?;
                    serde_json::Value::Number(n.into())
                }
                TxCommand::Expire(key, secs) => {
                    let ok = self.expire(&key, secs)?;
                    serde_json::Value::Bool(ok)
                }
                TxCommand::Lpush(key, values) => {
                    let n = self.lpush(&key, &values)?;
                    serde_json::Value::Number(n.into())
                }
                TxCommand::Rpush(key, values) => {
                    let n = self.rpush(&key, &values)?;
                    serde_json::Value::Number(n.into())
                }
                TxCommand::Sadd(key, members) => {
                    let n = self.sadd(&key, &members)?;
                    serde_json::Value::Number(n.into())
                }
                TxCommand::Hset(key, field, value) => {
                    let n = self.hset(&key, &[(&field, value)])?;
                    serde_json::Value::Number(n.into())
                }
            };
            results.push(val);
        }

        Ok(results)
    }
}
