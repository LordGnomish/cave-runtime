// SPDX-License-Identifier: AGPL-3.0-or-later
//! RDB snapshot and AOF (Append-Only File) persistence.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

use crate::db::{Db, ServerState};
use crate::resp::encode_resp2;
use crate::resp::Resp;

// ── AOF ───────────────────────────────────────────────────────────────────────

pub struct AofWriter {
    file: tokio::fs::File,
}

impl AofWriter {
    pub async fn open(path: &str) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        Ok(AofWriter { file })
    }

    /// Append a command to the AOF file.
    pub async fn append(&mut self, args: &[Vec<u8>]) -> io::Result<()> {
        let resp = Resp::Array(Some(
            args.iter()
                .map(|a| Resp::BulkString(Some(a.clone())))
                .collect(),
        ));
        let encoded = encode_resp2(&resp);
        self.file.write_all(&encoded).await?;
        Ok(())
    }

    pub async fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data().await?;
        Ok(())
    }
}

// ── RDB ───────────────────────────────────────────────────────────────────────

/// Background save task.
pub async fn bgsave_task(state: Arc<ServerState>) -> io::Result<()> {
    let rdb_path = {
        let cfg = state.config.read().await;
        cfg.rdb_path.clone()
    };

    let tmp_path = format!("{}.tmp", rdb_path);

    // Serialize all databases to a binary format
    let mut data = Vec::new();

    // RDB header
    data.extend_from_slice(b"REDIS0011"); // RDB version 11

    for (db_idx, db_lock) in state.dbs.iter().enumerate() {
        let db = db_lock.read().await;
        if db.keys.is_empty() {
            continue;
        }

        // Select DB opcode
        data.push(0xFE); // SELECTDB
        data.push(db_idx as u8);

        // Resize DB opcode
        data.push(0xFB); // RESIZEDB
        write_length(&mut data, db.keys.len());
        write_length(&mut data, db.keys.values().filter(|e| e.expires_at.is_some()).count());

        // Write each key
        for (key, entry) in &db.keys {
            if entry.is_expired() {
                continue;
            }

            // Expiry if set
            if let Some(expires_at) = entry.expires_at {
                let remaining = expires_at
                    .checked_duration_since(Instant::now())
                    .unwrap_or(Duration::ZERO);
                let expire_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
                    + remaining.as_millis() as u64;
                data.push(0xFC); // EXPIRETIME_MS
                data.extend_from_slice(&expire_ms.to_le_bytes());
            }

            // Type byte
            let type_byte: u8 = match &entry.value {
                crate::types::Value::String(_) => 0,
                crate::types::Value::List(_) => 10,   // listpack encoding
                crate::types::Value::Set(_) => 11,    // listpack encoding
                crate::types::Value::ZSet(_) => 17,   // listpack encoding
                crate::types::Value::Hash(_) => 16,   // listpack encoding
                crate::types::Value::Stream(_) => 19, // stream listpack v3
            };
            data.push(type_byte);

            // Key
            write_string(&mut data, key);

            // Value (simplified encoding)
            match &entry.value {
                crate::types::Value::String(v) => write_string(&mut data, v),
                crate::types::Value::List(list) => {
                    write_length(&mut data, list.len());
                    for item in list {
                        write_string(&mut data, item);
                    }
                }
                crate::types::Value::Set(set) => {
                    write_length(&mut data, set.len());
                    for item in set {
                        write_string(&mut data, item);
                    }
                }
                crate::types::Value::Hash(hash) => {
                    write_length(&mut data, hash.len());
                    for (k, v) in hash {
                        write_string(&mut data, k);
                        write_string(&mut data, v);
                    }
                }
                crate::types::Value::ZSet(zset) => {
                    write_length(&mut data, zset.len());
                    for (member, score) in zset.iter_asc() {
                        write_string(&mut data, member);
                        data.extend_from_slice(&score.to_le_bytes());
                    }
                }
                crate::types::Value::Stream(stream) => {
                    write_length(&mut data, stream.entries.len());
                    for entry in &stream.entries {
                        let id_str = entry.id.to_string();
                        write_string(&mut data, id_str.as_bytes());
                        write_length(&mut data, entry.fields.len());
                        for (k, v) in &entry.fields {
                            write_string(&mut data, k);
                            write_string(&mut data, v);
                        }
                    }
                }
            }
        }
    }

    // EOF marker + CRC64 checksum placeholder
    data.push(0xFF);
    data.extend_from_slice(&[0u8; 8]); // CRC64 (zero for now)

    // Write to tmp file and rename
    tokio::fs::write(&tmp_path, &data).await?;
    tokio::fs::rename(&tmp_path, &rdb_path).await?;

    Ok(())
}

fn write_length(buf: &mut Vec<u8>, len: usize) {
    if len < 64 {
        buf.push(len as u8);
    } else if len < 16384 {
        buf.push(0x40 | ((len >> 8) as u8));
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

// ── Background save scheduler ─────────────────────────────────────────────────

pub async fn save_scheduler_task(state: Arc<ServerState>) {
    let mut last_save = Instant::now();

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let (intervals, rdb_path) = {
            let cfg = state.config.read().await;
            (cfg.rdb_save_intervals.clone(), cfg.rdb_path.clone())
        };

        let dirty = state.dirty.load(std::sync::atomic::Ordering::Relaxed);
        let elapsed = last_save.elapsed().as_secs();

        let should_save = intervals.iter().any(|(secs, changes)| {
            elapsed >= *secs && dirty >= *changes
        });

        if should_save && dirty > 0 {
            match bgsave_task(Arc::clone(&state)).await {
                Ok(()) => {
                    tracing::info!("RDB saved to {}", rdb_path);
                    state.dirty.store(0, std::sync::atomic::Ordering::Relaxed);
                    last_save = Instant::now();
                }
                Err(e) => {
                    tracing::error!("RDB save failed: {}", e);
                }
            }
        }
    }
}
