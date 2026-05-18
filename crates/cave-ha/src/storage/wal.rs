// SPDX-License-Identifier: AGPL-3.0-or-later
//! Write-ahead log for durable Raft state.
//!
//! WAL format — each record:
//!   [magic: 4B = 0xCAFEBA11][type: 1B][len: 4B (BE)][csum: 4B][data: len B]
//!
//! Record types:
//!   0x01 = LogEntry
//!   0x02 = HardState
//!   0x04 = Truncate(last_index: u64)

use std::io;
use std::path::{Path, PathBuf};

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info};

use crate::error::{HaError, HaResult};
use crate::raft::log::LogEntry;
use crate::raft::types::HardState;

const WAL_MAGIC: u32 = 0xCAFEBA11;
#[allow(dead_code)]
const WAL_VERSION: u8 = 1;

#[repr(u8)]
enum RecordType {
    LogEntry = 0x01,
    HardState = 0x02,
    Truncate = 0x04,
}

/// Append-only WAL file.
pub struct Wal {
    path: PathBuf,
    file: File,
    size: u64,
}

impl Wal {
    /// Open (or create) a WAL file.
    pub async fn open(path: impl AsRef<Path>) -> HaResult<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .await?;
        let size = file.metadata().await?.len();
        info!(path = %path.display(), size, "WAL opened");
        Ok(Self { path, file, size })
    }

    // ── Write operations ──────────────────────────────────────────────────

    pub async fn append_entry(&mut self, entry: &LogEntry) -> HaResult<()> {
        let data = serde_json::to_vec(entry)?;
        self.write_record(RecordType::LogEntry as u8, &data).await
    }

    pub async fn append_hard_state(&mut self, hs: &HardState) -> HaResult<()> {
        let data = serde_json::to_vec(hs)?;
        self.write_record(RecordType::HardState as u8, &data).await
    }

    pub async fn append_truncate(&mut self, last_index: u64) -> HaResult<()> {
        let data = last_index.to_be_bytes().to_vec();
        self.write_record(RecordType::Truncate as u8, &data).await
    }

    async fn write_record(&mut self, rtype: u8, data: &[u8]) -> HaResult<()> {
        let len = data.len() as u32;
        let csum = simple_checksum(data);
        let mut buf = Vec::with_capacity(13 + data.len());
        buf.extend_from_slice(&WAL_MAGIC.to_be_bytes());
        buf.push(rtype);
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&csum.to_be_bytes());
        buf.extend_from_slice(data);
        self.file.write_all(&buf).await?;
        self.file.flush().await?;
        self.size += buf.len() as u64;
        Ok(())
    }

    // ── Read / replay ─────────────────────────────────────────────────────

    /// Replay all records from the beginning of the WAL.
    pub async fn replay(&mut self) -> HaResult<WalReplay> {
        self.file.seek(std::io::SeekFrom::Start(0)).await
            .map_err(|e| HaError::Storage(e.to_string()))?;
        let mut replay = WalReplay::default();
        let mut buf = Vec::new();
        self.file.read_to_end(&mut buf).await?;
        let mut pos = 0usize;

        while pos + 13 <= buf.len() {
            let magic = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
            if magic != WAL_MAGIC {
                debug!(pos, "WAL: unexpected magic, stopping replay");
                break;
            }
            let rtype = buf[pos + 4];
            let len = u32::from_be_bytes(buf[pos + 5..pos + 9].try_into().unwrap()) as usize;
            let csum = u32::from_be_bytes(buf[pos + 9..pos + 13].try_into().unwrap());
            pos += 13;

            if pos + len > buf.len() {
                debug!("WAL: incomplete record at end, truncating");
                break;
            }
            let data = &buf[pos..pos + len];
            pos += len;

            if simple_checksum(data) != csum {
                debug!("WAL: checksum mismatch, stopping replay");
                break;
            }

            match rtype {
                0x01 => {
                    // LogEntry
                    let entry: LogEntry = serde_json::from_slice(data)
                        .map_err(|e| HaError::Storage(e.to_string()))?;
                    replay.entries.push(entry);
                }
                0x02 => {
                    // HardState
                    let hs: HardState = serde_json::from_slice(data)
                        .map_err(|e| HaError::Storage(e.to_string()))?;
                    replay.hard_state = Some(hs);
                }
                0x04 => {
                    // Truncate
                    if data.len() >= 8 {
                        let last_index = u64::from_be_bytes(data[..8].try_into().unwrap());
                        replay.entries.retain(|e| e.index <= last_index);
                    }
                }
                _ => debug!(rtype, "WAL: unknown record type"),
            }
        }
        Ok(replay)
    }

    /// Truncate the WAL to zero — used after snapshotting.
    pub async fn reset(&mut self) -> HaResult<()> {
        self.file.set_len(0).await?;
        self.size = 0;
        Ok(())
    }

    pub fn size(&self) -> u64 { self.size }
    pub fn path(&self) -> &Path { &self.path }
}

// ── Seek shim ─────────────────────────────────────────────────────────────

trait AsyncSeekExt2 {
    async fn seek(&mut self, pos: std::io::SeekFrom) -> io::Result<u64>;
}

impl AsyncSeekExt2 for File {
    async fn seek(&mut self, pos: std::io::SeekFrom) -> io::Result<u64> {
        tokio::io::AsyncSeekExt::seek(self, pos).await
    }
}

// ── Replay result ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct WalReplay {
    pub hard_state: Option<HardState>,
    pub entries: Vec<LogEntry>,
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn simple_checksum(data: &[u8]) -> u32 {
    data.iter().fold(0u32, |acc, &b| acc.wrapping_mul(31).wrapping_add(b as u32))
}
