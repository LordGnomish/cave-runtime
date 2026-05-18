// SPDX-License-Identifier: AGPL-3.0-or-later
use serde::{Deserialize, Serialize};
use crate::raft::types::{LogIndex, MembershipConfig, NodeId, SnapshotMeta, Term};

/// A complete snapshot ready for installation or transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: SnapshotMeta,
    pub data: Vec<u8>,
}

impl Snapshot {
    pub fn new(index: LogIndex, term: Term, membership: MembershipConfig, data: Vec<u8>) -> Self {
        Self { meta: SnapshotMeta { index, term, membership }, data }
    }

    /// Split data into chunks of `chunk_size` bytes for streaming transfer.
    pub fn chunks(&self, chunk_size: usize) -> Vec<SnapshotChunk> {
        if self.data.is_empty() {
            return vec![SnapshotChunk {
                meta: self.meta.clone(),
                offset: 0,
                data: vec![],
                done: true,
            }];
        }
        let total = self.data.len();
        let mut chunks = Vec::new();
        let mut offset = 0usize;
        while offset < total {
            let end = (offset + chunk_size).min(total);
            let done = end == total;
            chunks.push(SnapshotChunk {
                meta: self.meta.clone(),
                offset: offset as u64,
                data: self.data[offset..end].to_vec(),
                done,
            });
            offset = end;
        }
        chunks
    }
}

/// A single chunk of a snapshot being transferred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotChunk {
    pub meta: SnapshotMeta,
    pub offset: u64,
    pub data: Vec<u8>,
    pub done: bool,
}

/// Assembles incoming snapshot chunks into a complete snapshot.
#[derive(Debug, Default)]
pub struct SnapshotReceiver {
    pub meta: Option<SnapshotMeta>,
    pub buffer: Vec<u8>,
    pub next_offset: u64,
}

impl SnapshotReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk; returns the complete snapshot when done is true.
    pub fn feed(&mut self, chunk: SnapshotChunk) -> Option<Snapshot> {
        if self.meta.is_none() {
            self.meta = Some(chunk.meta.clone());
        }
        // Allow gaps to be reset on new snapshot (higher term).
        if chunk.offset == 0 {
            self.buffer.clear();
            self.next_offset = 0;
            self.meta = Some(chunk.meta.clone());
        }
        if chunk.offset != self.next_offset {
            // Out of order; reset.
            self.buffer.clear();
            self.next_offset = 0;
            return None;
        }
        self.next_offset += chunk.data.len() as u64;
        self.buffer.extend_from_slice(&chunk.data);

        if chunk.done {
            let meta = self.meta.take().unwrap_or_default();
            let data = std::mem::take(&mut self.buffer);
            self.next_offset = 0;
            Some(Snapshot { meta, data })
        } else {
            None
        }
    }
}

/// Tracks in-flight snapshot transfers to peers (leader side).
#[derive(Debug)]
pub struct SnapshotTransfer {
    pub to: NodeId,
    pub snapshot: Snapshot,
    pub next_chunk: usize,
    pub chunk_size: usize,
}

impl SnapshotTransfer {
    pub fn new(to: NodeId, snapshot: Snapshot, chunk_size: usize) -> Self {
        Self { to, snapshot, next_chunk: 0, chunk_size }
    }

    /// Get the next chunk to send, or None if complete.
    pub fn next(&mut self) -> Option<SnapshotChunk> {
        let chunks = self.snapshot.chunks(self.chunk_size);
        if self.next_chunk >= chunks.len() {
            return None;
        }
        let chunk = chunks[self.next_chunk].clone();
        self.next_chunk += 1;
        Some(chunk)
    }

    pub fn is_done(&self) -> bool {
        let total_chunks = if self.snapshot.data.is_empty() {
            1
        } else {
            (self.snapshot.data.len() + self.chunk_size - 1) / self.chunk_size
        };
        self.next_chunk >= total_chunks
    }
}
