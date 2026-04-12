//! Write-ahead log: append-only journal with CRC32 integrity checks.
//! Format: [4-byte length][4-byte CRC32][N-byte JSON record]

use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use crc32fast::Hasher as Crc32Hasher;
use serde::{Deserialize, Serialize};
use crate::model::{Labels, Sample};
use crate::error::{MetricsError, Result};

#[derive(Debug, Serialize, Deserialize)]
pub enum WalRecord {
    Sample {
        labels: std::collections::BTreeMap<String, String>,
        timestamp_ms: i64,
        value: f64,
    },
    Checkpoint {
        ts: i64,
    },
}

pub struct WalWriter {
    writer: BufWriter<std::fs::File>,
}

impl WalWriter {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self { writer: BufWriter::new(file) })
    }

    pub fn append_sample(&mut self, labels: &Labels, sample: &Sample) -> Result<()> {
        let record = WalRecord::Sample {
            labels: labels.0.clone(),
            timestamp_ms: sample.timestamp_ms,
            value: sample.value,
        };
        self.write_record(&record)
    }

    fn write_record(&mut self, record: &WalRecord) -> Result<()> {
        let payload = serde_json::to_vec(record)?;
        let len = payload.len() as u32;

        let mut h = Crc32Hasher::new();
        h.update(&payload);
        let crc = h.finalize();

        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&crc.to_le_bytes())?;
        self.writer.write_all(&payload)?;
        self.writer.flush()?;
        Ok(())
    }
}

pub struct WalReader {
    file: std::fs::File,
}

impl WalReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        Ok(Self { file })
    }

    /// Replay the WAL, calling `f` for each valid record.
    pub fn replay<F: FnMut(WalRecord)>(&mut self, mut f: F) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut len_buf = [0u8; 4];
        let mut crc_buf = [0u8; 4];

        loop {
            match self.file.read_exact(&mut len_buf) {
                Ok(_)  => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            self.file.read_exact(&mut crc_buf)?;
            let len = u32::from_le_bytes(len_buf) as usize;
            let expected_crc = u32::from_le_bytes(crc_buf);

            let mut payload = vec![0u8; len];
            self.file.read_exact(&mut payload)?;

            let mut h = Crc32Hasher::new();
            h.update(&payload);
            let actual_crc = h.finalize();

            if actual_crc != expected_crc {
                // Corrupted record — skip (could log here).
                continue;
            }

            match serde_json::from_slice::<WalRecord>(&payload) {
                Ok(record) => f(record),
                Err(_)     => continue, // skip malformed
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_wal_roundtrip() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        {
            let mut w = WalWriter::open(path).unwrap();
            let labels = Labels::from_pairs([("__name__", "cpu"), ("job", "test")]);
            w.append_sample(&labels, &Sample::new(1000, 0.5)).unwrap();
            w.append_sample(&labels, &Sample::new(2000, 0.6)).unwrap();
        }

        let mut r = WalReader::open(path).unwrap();
        let mut records = Vec::new();
        r.replay(|rec| records.push(rec)).unwrap();
        assert_eq!(records.len(), 2);
    }
}
