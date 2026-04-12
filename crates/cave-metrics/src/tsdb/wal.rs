//! Write-ahead log for TSDB.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::error::{MetricsError, MetricsResult};
use crate::model::{Labels, Timestamp, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum WalRecord {
    #[serde(rename = "s")]
    Sample { fp: u64, ts: Timestamp, v: Value },
    #[serde(rename = "m")]
    Meta { fp: u64, labels: Labels },
}

pub struct Wal {
    path: Option<PathBuf>,
}

impl Wal {
    pub fn new(dir: Option<&Path>) -> MetricsResult<Self> {
        if let Some(dir) = dir {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self {
            path: dir.map(|d| d.join("wal.log")),
        })
    }

    fn write_record(&self, record: &WalRecord) -> MetricsResult<()> {
        let path = match &self.path {
            Some(p) => p,
            None => return Ok(()),
        };
        let json = serde_json::to_vec(record)?;
        let crc = crc32fast::hash(&json);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(&crc.to_le_bytes())?;
        let len = json.len() as u32;
        file.write_all(&len.to_le_bytes())?;
        file.write_all(&json)?;
        Ok(())
    }

    pub fn append_sample(&self, fp: u64, ts: Timestamp, value: Value) -> MetricsResult<()> {
        self.write_record(&WalRecord::Sample { fp, ts, v: value })
    }

    pub fn append_meta(&self, fp: u64, labels: &Labels) -> MetricsResult<()> {
        self.write_record(&WalRecord::Meta { fp, labels: labels.clone() })
    }

    pub fn replay(path: &Path) -> MetricsResult<Vec<WalRecord>> {
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(MetricsError::Io(e)),
        };
        let mut records = Vec::new();
        loop {
            let mut crc_buf = [0u8; 4];
            match file.read_exact(&mut crc_buf) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(MetricsError::Io(e)),
            }
            let stored_crc = u32::from_le_bytes(crc_buf);
            let mut len_buf = [0u8; 4];
            file.read_exact(&mut len_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut json = vec![0u8; len];
            file.read_exact(&mut json)?;
            let computed_crc = crc32fast::hash(&json);
            if computed_crc != stored_crc {
                return Err(MetricsError::Wal("CRC mismatch in WAL".to_string()));
            }
            let record: WalRecord = serde_json::from_slice(&json)?;
            records.push(record);
        }
        Ok(records)
    }
}
