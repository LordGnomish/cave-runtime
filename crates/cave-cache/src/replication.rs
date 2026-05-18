// SPDX-License-Identifier: AGPL-3.0-or-later
//! Master / replica replication.
//!
//! Ports the master-side and replica-side state machines from
//! `src/replication.c`. A replica issues `REPLICAOF host port` to
//! point at a master; the two then engage in either a partial resync
//! (`PSYNC <repl_id> <offset>`) or a full resync (`SYNC` → RDB blob).
//! Each side maintains a replication offset that command writers
//! must advance to keep the streams aligned.
//!
//! Out of scope: the actual on-the-wire RDB encoder + TCP socket
//! lifecycle. This module owns the pure state — repl id, offset,
//! backlog, partial-resync eligibility, replica registry — and the
//! caller bridges it to the wire.

use std::collections::HashMap;
use std::collections::VecDeque;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReplicationError {
    #[error("replica {0} unknown")]
    UnknownReplica(String),
    #[error("master has not been configured (run REPLICAOF first)")]
    NoMaster,
    #[error("partial resync impossible: offset {requested} below backlog floor {floor}")]
    BacklogBelowOffset { requested: u64, floor: u64 },
    #[error("partial resync impossible: replication id mismatch (have {have}, asked {asked})")]
    ReplIdMismatch { have: String, asked: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationRole {
    Master,
    Replica,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaState {
    /// Replica connected, waiting on initial PSYNC reply.
    WaitPsync,
    /// Replica is receiving the initial RDB snapshot.
    Syncing,
    /// Replica is up-to-date and tailing the command stream.
    Online,
    /// Replica disconnected. Eligible for partial resync if backlog covers.
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct ReplicaPeer {
    pub id: String,
    pub addr: String,
    pub state: ReplicaState,
    pub offset: u64,
    pub last_ack_ms: u64,
}

/// Master-side bookkeeping.
#[derive(Debug)]
pub struct MasterState {
    /// 40-char hex replication id, stable until role flips.
    pub repl_id: String,
    /// Bytes written into the replication stream since boot.
    pub master_repl_offset: u64,
    /// Connected replicas, keyed by replica id.
    pub replicas: HashMap<String, ReplicaPeer>,
    /// Replication backlog — last `backlog_capacity` bytes of the
    /// command stream. Stored as one byte per element so the test
    /// fixtures stay readable.
    backlog: VecDeque<u8>,
    /// Maximum backlog size in bytes.
    pub backlog_capacity: usize,
    /// Stream offset of the oldest byte in the backlog.
    pub backlog_floor_offset: u64,
}

impl MasterState {
    pub fn new(repl_id: impl Into<String>, backlog_capacity: usize) -> Self {
        Self {
            repl_id: repl_id.into(),
            master_repl_offset: 0,
            replicas: HashMap::new(),
            backlog: VecDeque::with_capacity(backlog_capacity),
            backlog_capacity,
            backlog_floor_offset: 0,
        }
    }

    pub fn register_replica(&mut self, peer: ReplicaPeer) {
        self.replicas.insert(peer.id.clone(), peer);
    }

    pub fn deregister_replica(&mut self, id: &str) {
        self.replicas.remove(id);
    }

    /// Append `bytes` to the replication stream, advancing the master
    /// offset and ring-buffering the backlog.
    pub fn record_command_bytes(&mut self, bytes: &[u8]) -> u64 {
        for b in bytes {
            if self.backlog.len() == self.backlog_capacity {
                self.backlog.pop_front();
                self.backlog_floor_offset += 1;
            }
            self.backlog.push_back(*b);
        }
        self.master_repl_offset += bytes.len() as u64;
        self.master_repl_offset
    }

    /// Decide between partial and full resync given a replica's
    /// `PSYNC <repl_id> <offset>` request.
    pub fn handle_psync(
        &self,
        requested_repl_id: &str,
        requested_offset: u64,
    ) -> Result<PsyncDecision, ReplicationError> {
        if requested_repl_id != self.repl_id && requested_repl_id != "?" {
            return Err(ReplicationError::ReplIdMismatch {
                have: self.repl_id.clone(),
                asked: requested_repl_id.into(),
            });
        }
        if requested_repl_id == "?" || requested_offset < self.backlog_floor_offset {
            return Ok(PsyncDecision::FullResync {
                repl_id: self.repl_id.clone(),
                master_offset: self.master_repl_offset,
            });
        }
        // Partial resync: caller streams the backlog window
        // `[requested_offset, master_repl_offset)`.
        let already_relayed = (requested_offset - self.backlog_floor_offset) as usize;
        let tail: Vec<u8> = self.backlog.iter().copied().skip(already_relayed).collect();
        Ok(PsyncDecision::PartialResync {
            repl_id: self.repl_id.clone(),
            from_offset: requested_offset,
            to_offset: self.master_repl_offset,
            bytes: tail,
        })
    }

    pub fn update_replica_ack(
        &mut self,
        id: &str,
        offset: u64,
        now_ms: u64,
    ) -> Result<(), ReplicationError> {
        let r = self
            .replicas
            .get_mut(id)
            .ok_or_else(|| ReplicationError::UnknownReplica(id.into()))?;
        r.offset = offset;
        r.last_ack_ms = now_ms;
        if r.state == ReplicaState::Syncing || r.state == ReplicaState::WaitPsync {
            r.state = ReplicaState::Online;
        }
        Ok(())
    }

    /// `INFO replication` master section.
    pub fn info(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("role".to_string(), "master".to_string()),
            ("connected_slaves".to_string(), self.replicas.len().to_string()),
            ("master_replid".to_string(), self.repl_id.clone()),
            (
                "master_repl_offset".to_string(),
                self.master_repl_offset.to_string(),
            ),
            (
                "repl_backlog_size".to_string(),
                self.backlog.len().to_string(),
            ),
            (
                "repl_backlog_first_byte_offset".to_string(),
                self.backlog_floor_offset.to_string(),
            ),
        ];
        for (i, r) in self.replicas.values().enumerate() {
            out.push((
                format!("slave{i}"),
                format!(
                    "ip={},state={:?},offset={},lag={}",
                    r.addr, r.state, r.offset, self.master_repl_offset.saturating_sub(r.offset)
                ),
            ));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PsyncDecision {
    PartialResync {
        repl_id: String,
        from_offset: u64,
        to_offset: u64,
        bytes: Vec<u8>,
    },
    FullResync {
        repl_id: String,
        master_offset: u64,
    },
}

/// Replica-side bookkeeping.
#[derive(Debug)]
pub struct ReplicaSideState {
    pub master_host: String,
    pub master_port: u16,
    pub state: ReplicaState,
    pub repl_id: Option<String>,
    pub offset: u64,
    /// Last full-resync digest the master sent.
    pub last_rdb_size: Option<u64>,
}

impl ReplicaSideState {
    pub fn new(master_host: impl Into<String>, master_port: u16) -> Self {
        Self {
            master_host: master_host.into(),
            master_port,
            state: ReplicaState::WaitPsync,
            repl_id: None,
            offset: 0,
            last_rdb_size: None,
        }
    }

    pub fn on_partial_resync(&mut self, repl_id: &str, to_offset: u64) {
        self.repl_id = Some(repl_id.to_string());
        self.offset = to_offset;
        self.state = ReplicaState::Online;
    }

    pub fn on_full_resync_start(&mut self, repl_id: &str, master_offset: u64) {
        self.repl_id = Some(repl_id.to_string());
        self.offset = master_offset;
        self.state = ReplicaState::Syncing;
    }

    pub fn on_rdb_received(&mut self, rdb_size: u64) {
        self.last_rdb_size = Some(rdb_size);
        self.state = ReplicaState::Online;
    }

    pub fn on_command_byte(&mut self, n: u64) {
        if self.state == ReplicaState::Online {
            self.offset += n;
        }
    }

    pub fn info(&self) -> Vec<(String, String)> {
        let mut out = vec![
            ("role".to_string(), "slave".to_string()),
            ("master_host".to_string(), self.master_host.clone()),
            ("master_port".to_string(), self.master_port.to_string()),
            (
                "master_link_status".to_string(),
                if matches!(self.state, ReplicaState::Online) {
                    "up".into()
                } else {
                    "down".into()
                },
            ),
            ("slave_repl_offset".to_string(), self.offset.to_string()),
        ];
        if let Some(id) = &self.repl_id {
            out.push(("master_replid".into(), id.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, addr: &str) -> ReplicaPeer {
        ReplicaPeer {
            id: id.into(),
            addr: addr.into(),
            state: ReplicaState::WaitPsync,
            offset: 0,
            last_ack_ms: 0,
        }
    }

    #[test]
    fn record_command_bytes_advances_offset() {
        let mut m = MasterState::new("repl-id", 1024);
        let off = m.record_command_bytes(b"SET k v");
        assert_eq!(off, 7);
        assert_eq!(m.master_repl_offset, 7);
    }

    #[test]
    fn backlog_ring_buffer_drops_oldest_bytes() {
        let mut m = MasterState::new("repl-id", 4);
        m.record_command_bytes(b"abc");
        m.record_command_bytes(b"defg");
        assert_eq!(m.master_repl_offset, 7);
        assert_eq!(m.backlog_floor_offset, 3);
        let tail: Vec<u8> = m.backlog.iter().copied().collect();
        assert_eq!(tail, b"defg");
    }

    #[test]
    fn psync_with_question_mark_returns_full_resync() {
        let m = MasterState::new("repl-id", 1024);
        let d = m.handle_psync("?", 0).unwrap();
        assert!(matches!(d, PsyncDecision::FullResync { .. }));
    }

    #[test]
    fn psync_with_wrong_replid_errors() {
        let m = MasterState::new("repl-id", 1024);
        let err = m.handle_psync("other-id", 0).unwrap_err();
        assert!(matches!(err, ReplicationError::ReplIdMismatch { .. }));
    }

    #[test]
    fn psync_below_backlog_floor_returns_full_resync() {
        let mut m = MasterState::new("repl-id", 4);
        m.record_command_bytes(b"abcdefgh");
        // backlog_floor = 4; replica asking from offset 2 → too old.
        let d = m.handle_psync("repl-id", 2).unwrap();
        assert!(matches!(d, PsyncDecision::FullResync { .. }));
    }

    #[test]
    fn psync_within_backlog_returns_partial_resync() {
        let mut m = MasterState::new("repl-id", 16);
        m.record_command_bytes(b"abcdef");
        let d = m.handle_psync("repl-id", 3).unwrap();
        if let PsyncDecision::PartialResync {
            from_offset,
            to_offset,
            bytes,
            ..
        } = d
        {
            assert_eq!(from_offset, 3);
            assert_eq!(to_offset, 6);
            assert_eq!(bytes, b"def");
        } else {
            panic!("expected partial resync");
        }
    }

    #[test]
    fn psync_at_master_offset_returns_empty_partial() {
        let mut m = MasterState::new("repl-id", 16);
        m.record_command_bytes(b"abc");
        let d = m.handle_psync("repl-id", 3).unwrap();
        if let PsyncDecision::PartialResync { bytes, .. } = d {
            assert!(bytes.is_empty());
        } else {
            panic!("expected partial resync");
        }
    }

    #[test]
    fn register_and_ack_brings_replica_online() {
        let mut m = MasterState::new("r", 1024);
        m.register_replica(peer("rp1", "10.0.0.2:6379"));
        m.update_replica_ack("rp1", 0, 100).unwrap();
        assert_eq!(m.replicas["rp1"].state, ReplicaState::Online);
    }

    #[test]
    fn update_unknown_replica_errors() {
        let mut m = MasterState::new("r", 1024);
        assert!(matches!(m.update_replica_ack("nope", 0, 0).unwrap_err(), ReplicationError::UnknownReplica(_)));
    }

    #[test]
    fn info_master_includes_replication_fields() {
        let mut m = MasterState::new("r1", 1024);
        m.register_replica(peer("rp1", "1.1.1.1:6379"));
        m.record_command_bytes(b"abc");
        let i: HashMap<String, String> = m.info().into_iter().collect();
        assert_eq!(i["role"], "master");
        assert_eq!(i["connected_slaves"], "1");
        assert_eq!(i["master_repl_offset"], "3");
        assert_eq!(i["master_replid"], "r1");
    }

    #[test]
    fn replica_full_resync_lifecycle() {
        let mut r = ReplicaSideState::new("10.0.0.1", 6379);
        assert_eq!(r.state, ReplicaState::WaitPsync);
        r.on_full_resync_start("r1", 100);
        assert_eq!(r.state, ReplicaState::Syncing);
        r.on_rdb_received(4096);
        assert_eq!(r.state, ReplicaState::Online);
        assert_eq!(r.offset, 100);
    }

    #[test]
    fn replica_partial_resync_keeps_offset_synced() {
        let mut r = ReplicaSideState::new("10.0.0.1", 6379);
        r.on_partial_resync("r1", 250);
        assert_eq!(r.state, ReplicaState::Online);
        assert_eq!(r.offset, 250);
    }

    #[test]
    fn replica_command_byte_advances_offset_only_when_online() {
        let mut r = ReplicaSideState::new("10.0.0.1", 6379);
        r.on_command_byte(50); // ignored, not online
        assert_eq!(r.offset, 0);
        r.on_partial_resync("r1", 250);
        r.on_command_byte(50);
        assert_eq!(r.offset, 300);
    }

    #[test]
    fn replica_info_reports_link_state() {
        let mut r = ReplicaSideState::new("h", 6379);
        let i: HashMap<String, String> = r.info().into_iter().collect();
        assert_eq!(i["master_link_status"], "down");
        r.on_partial_resync("rid", 1);
        let i: HashMap<String, String> = r.info().into_iter().collect();
        assert_eq!(i["master_link_status"], "up");
        assert_eq!(i["master_replid"], "rid");
    }

    #[test]
    fn deregister_replica_removes_entry() {
        let mut m = MasterState::new("r", 1024);
        m.register_replica(peer("rp1", "1"));
        m.deregister_replica("rp1");
        assert!(m.replicas.is_empty());
    }
}
