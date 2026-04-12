//! Write-Ahead Log — append-only, length-prefixed JSON records.
//!
//! Format: [4-byte LE length][JSON bytes]\n  (repeated)
//! On open, all entries are replayed to reconstruct in-memory state.

use std::{
    fs::{File, OpenOptions},
    io::{self, BufReader, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum WalOp {
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
        lease_id: i64,
    },
    Delete {
        key: Vec<u8>,
    },
    LeaseGrant {
        id: i64,
        ttl: i64,
        granted_at: u64,
    },
    LeaseRevoke {
        id: i64,
    },
    Compact {
        revision: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    pub revision: i64,
    pub timestamp: u64,
    pub op: WalOp,
}

impl WalEntry {
    pub fn new(revision: i64, op: WalOp) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self { revision, timestamp, op }
    }
}

pub struct WalFile {
    path: PathBuf,
    file: File,
    sync: bool,
}

impl WalFile {
    /// Open (create if absent) a WAL file for appending.
    pub fn open(path: impl AsRef<Path>, sync: bool) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { path, file, sync })
    }

    /// Append one entry. Writes 4-byte LE length then JSON bytes.
    pub fn append(&mut self, entry: &WalEntry) -> io::Result<()> {
        let json = serde_json::to_vec(entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = json.len() as u32;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&json)?;
        if self.sync {
            self.file.sync_data()?;
        }
        Ok(())
    }

    pub fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// Replay all entries from a WAL path. Corrupt trailing data is skipped with a warning.
    pub fn replay(path: impl AsRef<Path>) -> io::Result<Vec<WalEntry>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(vec![]);
        }
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();

        loop {
            let mut len_buf = [0u8; 4];
            match reader.read_exact(&mut len_buf) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            }
            let len = u32::from_le_bytes(len_buf) as usize;
            if len == 0 || len > 64 * 1024 * 1024 {
                warn!("WAL: suspicious record length {len}, stopping replay");
                break;
            }
            let mut buf = vec![0u8; len];
            match reader.read_exact(&mut buf) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    warn!("WAL: truncated record, stopping replay");
                    break;
                }
                Err(e) => return Err(e),
            }
            match serde_json::from_slice::<WalEntry>(&buf) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    warn!("WAL: corrupt record at offset, skipping: {e}");
                }
            }
        }
        Ok(entries)
    }

    /// Rotate: rename current file, open a fresh one at the same path.
    pub fn rotate(&mut self, archive_path: impl AsRef<Path>) -> io::Result<()> {
        self.file.sync_all()?;
        std::fs::rename(&self.path, archive_path)?;
        self.file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
