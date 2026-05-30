// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Write-ahead log (WAL).
//!
//! Port of PostgreSQL `src/backend/access/transam/{xlog.c,xloginsert.c}` plus
//! the CRC support in `src/common/pg_crc32c*`.
//!
//! Each emitted record carries an `XLogRecord`-shaped header — total length,
//! transaction id (`xl_xid`), the previous record's position (`xl_prev`),
//! resource-manager id/info bytes, and a CRC-32C over the header+payload. The
//! **insert pointer** (`XLogRecPtr`, our [`Lsn`]) is a monotonically increasing
//! byte offset into the logical log stream that advances by each record's
//! serialized length, and `xl_prev` chains every record back to its
//! predecessor so recovery can walk the log deterministically.

/// `XLogRecPtr`: a byte offset into the logical WAL stream.
pub type Lsn = u64;

/// Where the logical log stream begins (a nominal segment boundary).
const START_LSN: Lsn = 0x0100_0000;

/// A single WAL record.
#[derive(Debug, Clone)]
pub struct WalRecord {
    /// this record's position in the stream (`XLogRecPtr`)
    pub lsn: Lsn,
    /// position of the preceding record (`xl_prev`)
    pub prev: Lsn,
    /// transaction id (`xl_xid`)
    pub xid: u32,
    /// resource manager id (`xl_rmid`)
    pub rmid: u8,
    /// rmgr-specific info bits (`xl_info`)
    pub info: u8,
    /// rmgr payload
    pub data: Vec<u8>,
    /// CRC-32C over the header fields + payload (`xl_crc`)
    pub crc: u32,
}

impl WalRecord {
    /// Fixed `XLogRecord` header size: tot_len(4) + xid(4) + prev(8) +
    /// info(1) + rmid(1) + pad(2) + crc(4).
    pub const HEADER_LEN: u64 = 24;

    /// Serialized length of the record (header + payload).
    pub fn total_len(&self) -> u64 {
        Self::HEADER_LEN + self.data.len() as u64
    }

    /// Bytes the CRC is computed over: every header field except `xl_crc`
    /// itself, followed by the payload (matching `COMP_CRC32C` coverage).
    fn crc_input(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.data.len() + 16);
        buf.extend_from_slice(&self.xid.to_le_bytes());
        buf.extend_from_slice(&self.prev.to_le_bytes());
        buf.push(self.rmid);
        buf.push(self.info);
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Recompute the CRC and compare against the stored value.
    pub fn crc_valid(&self) -> bool {
        crc32c(&self.crc_input()) == self.crc
    }
}

/// The write-ahead log buffer.
pub struct Wal {
    records: Vec<WalRecord>,
    /// next free byte offset (the insert pointer)
    insert_lsn: Lsn,
    /// LSN of the most recently inserted record (or START_LSN before any)
    prev_lsn: Lsn,
}

impl Default for Wal {
    fn default() -> Self {
        Self::new()
    }
}

impl Wal {
    pub fn new() -> Self {
        Wal {
            records: Vec::new(),
            insert_lsn: START_LSN,
            prev_lsn: START_LSN,
        }
    }

    /// Current insert pointer — the LSN the next record will occupy.
    pub fn insert_lsn(&self) -> Lsn {
        self.insert_lsn
    }

    /// LSN of the last inserted record, or `None` if the log is empty.
    pub fn last_lsn(&self) -> Option<Lsn> {
        self.records.last().map(|r| r.lsn)
    }

    pub fn records(&self) -> &[WalRecord] {
        &self.records
    }

    pub fn records_mut(&mut self) -> &mut Vec<WalRecord> {
        &mut self.records
    }

    /// `XLogInsert`: append a record, stamping it with the current insert
    /// pointer and the previous record's LSN, then advance the pointer.
    /// Returns the LSN the record was written at.
    pub fn append(&mut self, xid: u32, rmid: u8, info: u8, data: Vec<u8>) -> Lsn {
        let lsn = self.insert_lsn;
        let mut rec = WalRecord {
            lsn,
            prev: self.prev_lsn,
            xid,
            rmid,
            info,
            data,
            crc: 0,
        };
        rec.crc = crc32c(&rec.crc_input());
        let total = rec.total_len();
        self.records.push(rec);
        self.prev_lsn = lsn;
        self.insert_lsn = lsn + total;
        lsn
    }

    /// Recovery redo loop: invoke `f` on every record in ascending LSN order.
    pub fn replay<F: FnMut(&WalRecord)>(&self, mut f: F) {
        for rec in &self.records {
            f(rec);
        }
    }

    /// Verify the CRC of every record (recovery integrity check).
    pub fn verify(&self) -> bool {
        self.records.iter().all(|r| r.crc_valid())
    }
}

/// CRC-32C (Castagnoli) — reflected polynomial `0x82F63B78`, the checksum
/// PostgreSQL uses for WAL records and data-page checksums since 9.5.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0x82F6_3B78;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_log_has_no_last_lsn() {
        let wal = Wal::new();
        assert_eq!(wal.last_lsn(), None);
        assert_eq!(wal.insert_lsn(), START_LSN);
        assert!(wal.verify());
    }

    #[test]
    fn header_len_matches_total_len_accounting() {
        let mut wal = Wal::new();
        wal.append(1, 1, 0, vec![0u8; 10]);
        assert_eq!(wal.records()[0].total_len(), WalRecord::HEADER_LEN + 10);
    }
}
