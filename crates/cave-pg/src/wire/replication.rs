//! Logical/physical replication subset.
//!
//! Implements the inner messages carried inside `CopyData` frames once a
//! connection has issued `START_REPLICATION`:
//!
//! | byte | meaning             | direction |
//! |------|---------------------|-----------|
//! | 'w'  | XLogData            | server → |
//! | 'k'  | Primary keepalive   | server → |
//! | 'r'  | Standby status      | ← client |
//! | 'h'  | Hot-standby feedback| ← client |
//!
//! Layout follows `src/backend/replication/walsender.c` and the documentation
//! at <https://www.postgresql.org/docs/current/protocol-replication.html>.

use bytes::{Buf, BytesMut};

use super::codec::{ProtocolError, ProtocolResult};

/// PostgreSQL epoch for replication timestamps: 2000-01-01 00:00:00 UTC,
/// expressed as microseconds since the Unix epoch.
pub const PG_EPOCH_MICROS: i64 = 946_684_800_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XLogData {
    /// LSN starting position of this WAL record chunk.
    pub wal_start: u64,
    /// LSN of the last byte transmitted on the connection.
    pub wal_end: u64,
    /// Server send time, microseconds since PG epoch.
    pub server_send_time_pg: i64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryKeepalive {
    pub wal_end: u64,
    pub server_send_time_pg: i64,
    pub reply_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandbyStatusUpdate {
    pub last_received_lsn: u64,
    pub last_flushed_lsn: u64,
    pub last_applied_lsn: u64,
    pub client_send_time_pg: i64,
    pub reply_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotStandbyFeedback {
    pub client_send_time_pg: i64,
    pub current_xmin: u32,
    pub current_xmin_epoch: u32,
    pub current_catalog_xmin: u32,
    pub current_catalog_xmin_epoch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationServerMsg {
    XLogData(XLogData),
    Keepalive(PrimaryKeepalive),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationClientMsg {
    StandbyStatus(StandbyStatusUpdate),
    HotStandbyFeedback(HotStandbyFeedback),
}

pub fn encode_xlog_data(x: &XLogData) -> Vec<u8> {
    let mut out = Vec::with_capacity(25 + x.data.len());
    out.push(b'w');
    out.extend_from_slice(&x.wal_start.to_be_bytes());
    out.extend_from_slice(&x.wal_end.to_be_bytes());
    out.extend_from_slice(&x.server_send_time_pg.to_be_bytes());
    out.extend_from_slice(&x.data);
    out
}

pub fn encode_keepalive(k: &PrimaryKeepalive) -> Vec<u8> {
    let mut out = Vec::with_capacity(18);
    out.push(b'k');
    out.extend_from_slice(&k.wal_end.to_be_bytes());
    out.extend_from_slice(&k.server_send_time_pg.to_be_bytes());
    out.push(if k.reply_requested { 1 } else { 0 });
    out
}

pub fn encode_standby_status(s: &StandbyStatusUpdate) -> Vec<u8> {
    let mut out = Vec::with_capacity(34);
    out.push(b'r');
    out.extend_from_slice(&s.last_received_lsn.to_be_bytes());
    out.extend_from_slice(&s.last_flushed_lsn.to_be_bytes());
    out.extend_from_slice(&s.last_applied_lsn.to_be_bytes());
    out.extend_from_slice(&s.client_send_time_pg.to_be_bytes());
    out.push(if s.reply_requested { 1 } else { 0 });
    out
}

pub fn encode_hot_standby_feedback(h: &HotStandbyFeedback) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    out.push(b'h');
    out.extend_from_slice(&h.client_send_time_pg.to_be_bytes());
    out.extend_from_slice(&h.current_xmin.to_be_bytes());
    out.extend_from_slice(&h.current_xmin_epoch.to_be_bytes());
    out.extend_from_slice(&h.current_catalog_xmin.to_be_bytes());
    out.extend_from_slice(&h.current_catalog_xmin_epoch.to_be_bytes());
    out
}

pub fn decode_server_msg(payload: &[u8]) -> ProtocolResult<ReplicationServerMsg> {
    let mut buf = BytesMut::from(payload);
    if buf.is_empty() {
        return Err(ProtocolError::Incomplete { needed: 1 });
    }
    let tag = buf.get_u8();
    match tag {
        b'w' => {
            if buf.len() < 24 {
                return Err(ProtocolError::Incomplete {
                    needed: 24 - buf.len(),
                });
            }
            let wal_start = buf.get_u64();
            let wal_end = buf.get_u64();
            let server_send_time_pg = buf.get_i64();
            Ok(ReplicationServerMsg::XLogData(XLogData {
                wal_start,
                wal_end,
                server_send_time_pg,
                data: buf.to_vec(),
            }))
        }
        b'k' => {
            if buf.len() < 17 {
                return Err(ProtocolError::Incomplete {
                    needed: 17 - buf.len(),
                });
            }
            let wal_end = buf.get_u64();
            let server_send_time_pg = buf.get_i64();
            let reply_requested = buf.get_u8() != 0;
            Ok(ReplicationServerMsg::Keepalive(PrimaryKeepalive {
                wal_end,
                server_send_time_pg,
                reply_requested,
            }))
        }
        other => Err(ProtocolError::InvalidValue {
            field: "replication.server_tag",
            reason: format!("0x{:02x}", other),
        }),
    }
}

pub fn decode_client_msg(payload: &[u8]) -> ProtocolResult<ReplicationClientMsg> {
    let mut buf = BytesMut::from(payload);
    if buf.is_empty() {
        return Err(ProtocolError::Incomplete { needed: 1 });
    }
    let tag = buf.get_u8();
    match tag {
        b'r' => {
            if buf.len() < 33 {
                return Err(ProtocolError::Incomplete {
                    needed: 33 - buf.len(),
                });
            }
            let last_received_lsn = buf.get_u64();
            let last_flushed_lsn = buf.get_u64();
            let last_applied_lsn = buf.get_u64();
            let client_send_time_pg = buf.get_i64();
            let reply_requested = buf.get_u8() != 0;
            Ok(ReplicationClientMsg::StandbyStatus(StandbyStatusUpdate {
                last_received_lsn,
                last_flushed_lsn,
                last_applied_lsn,
                client_send_time_pg,
                reply_requested,
            }))
        }
        b'h' => {
            if buf.len() < 24 {
                return Err(ProtocolError::Incomplete {
                    needed: 24 - buf.len(),
                });
            }
            let client_send_time_pg = buf.get_i64();
            let current_xmin = buf.get_u32();
            let current_xmin_epoch = buf.get_u32();
            let current_catalog_xmin = buf.get_u32();
            let current_catalog_xmin_epoch = buf.get_u32();
            Ok(ReplicationClientMsg::HotStandbyFeedback(HotStandbyFeedback {
                client_send_time_pg,
                current_xmin,
                current_xmin_epoch,
                current_catalog_xmin,
                current_catalog_xmin_epoch,
            }))
        }
        other => Err(ProtocolError::InvalidValue {
            field: "replication.client_tag",
            reason: format!("0x{:02x}", other),
        }),
    }
}

/// Format an LSN as the canonical `XX/YY` hex string used by `pg_lsn`.
pub fn format_lsn(lsn: u64) -> String {
    format!("{:X}/{:X}", lsn >> 32, lsn & 0xFFFFFFFF)
}

// ── Replication command-language verbs (`repl_gram.y`) ───────────────────────
//
// Once a connection has flagged `replication=true` in its startup parameters,
// the Query path no longer accepts SQL — the only verbs the walsender accepts
// are the ones below.  We model the parser as a small string-based decoder
// that produces a typed `ReplCommand`; the wire-level frame is still a
// `FrontendMessage::Query`, so the server hands the command string here for
// classification.

/// One of the replication-protocol verbs accepted by `walsender` after a
/// `replication=true` startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// `IDENTIFY_SYSTEM` — returns a single row with system identifier,
    /// timeline ID, latest LSN, and (optionally) database name.
    IdentifySystem,
    /// `START_REPLICATION [SLOT name] [PHYSICAL|LOGICAL] LSN [TIMELINE n]`
    /// or for logical: `START_REPLICATION SLOT name LOGICAL LSN [(opt = val, ...)]`
    StartReplication {
        slot: Option<String>,
        kind: ReplKind,
        start_lsn: u64,
        timeline: Option<u32>,
    },
    /// `TIMELINE_HISTORY tli` — fetch the .history file for a timeline.
    TimelineHistory { timeline: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplKind {
    Physical,
    Logical,
}

/// Reply payload for `IDENTIFY_SYSTEM`.  Encoded as a four-column DataRow
/// with text-format cells.  Mirrors `walsender.c::IdentifySystem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifySystemReply {
    pub system_id: String,
    pub timeline: u32,
    pub xlog_pos: u64,
    pub dbname: Option<String>,
}

impl IdentifySystemReply {
    /// Encode as a `Vec<Option<Vec<u8>>>` ready to drop into `BackendMessage::DataRow`.
    pub fn to_data_row(&self) -> Vec<Option<Vec<u8>>> {
        vec![
            Some(self.system_id.as_bytes().to_vec()),
            Some(self.timeline.to_string().into_bytes()),
            Some(format_lsn(self.xlog_pos).into_bytes()),
            self.dbname.as_ref().map(|d| d.as_bytes().to_vec()),
        ]
    }
}

/// Parse a replication-protocol command string, as produced by libpq's
/// walsender client.  Whitespace-tolerant; case-insensitive on keywords.
///
/// Returns `None` for an empty/whitespace string and `Err` for a recognised
/// command with malformed arguments.
pub fn parse_repl_command(s: &str) -> ProtocolResult<Option<ReplCommand>> {
    let trimmed = s.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut tokens = trimmed.split_whitespace();
    let head = match tokens.next() {
        Some(h) => h.to_ascii_uppercase(),
        None => return Ok(None),
    };
    match head.as_str() {
        "IDENTIFY_SYSTEM" => {
            if tokens.next().is_some() {
                return Err(ProtocolError::InvalidValue {
                    field: "IDENTIFY_SYSTEM",
                    reason: "unexpected trailing tokens".into(),
                });
            }
            Ok(Some(ReplCommand::IdentifySystem))
        }
        "TIMELINE_HISTORY" => {
            let tli = tokens.next().ok_or(ProtocolError::InvalidValue {
                field: "TIMELINE_HISTORY",
                reason: "missing timeline".into(),
            })?;
            let timeline = tli.parse::<u32>().map_err(|e| ProtocolError::InvalidValue {
                field: "TIMELINE_HISTORY.timeline",
                reason: e.to_string(),
            })?;
            Ok(Some(ReplCommand::TimelineHistory { timeline }))
        }
        "START_REPLICATION" => {
            // Grammar (subset):
            //   START_REPLICATION [SLOT slot_name] (PHYSICAL|LOGICAL) lsn [TIMELINE tli]
            let mut slot = None;
            let mut kind: Option<ReplKind> = None;
            let mut start_lsn: Option<u64> = None;
            let mut timeline: Option<u32> = None;
            let rest: Vec<&str> = tokens.collect();
            let mut i = 0;
            while i < rest.len() {
                let tok = rest[i].to_ascii_uppercase();
                match tok.as_str() {
                    "SLOT" => {
                        i += 1;
                        let name = rest.get(i).ok_or(ProtocolError::InvalidValue {
                            field: "START_REPLICATION.SLOT",
                            reason: "missing slot name".into(),
                        })?;
                        slot = Some((*name).to_string());
                    }
                    "PHYSICAL" => kind = Some(ReplKind::Physical),
                    "LOGICAL" => kind = Some(ReplKind::Logical),
                    "TIMELINE" => {
                        i += 1;
                        let tli = rest.get(i).ok_or(ProtocolError::InvalidValue {
                            field: "START_REPLICATION.TIMELINE",
                            reason: "missing timeline".into(),
                        })?;
                        timeline = Some(tli.parse::<u32>().map_err(|e| {
                            ProtocolError::InvalidValue {
                                field: "START_REPLICATION.TIMELINE",
                                reason: e.to_string(),
                            }
                        })?);
                    }
                    other => {
                        // First non-keyword positional → LSN
                        if start_lsn.is_none() && other.contains('/') {
                            start_lsn = Some(parse_lsn(other)?);
                        } else {
                            return Err(ProtocolError::InvalidValue {
                                field: "START_REPLICATION",
                                reason: format!("unexpected token '{}'", rest[i]),
                            });
                        }
                    }
                }
                i += 1;
            }
            let kind = kind.unwrap_or(ReplKind::Physical);
            let start_lsn = start_lsn.ok_or(ProtocolError::InvalidValue {
                field: "START_REPLICATION",
                reason: "missing start LSN".into(),
            })?;
            Ok(Some(ReplCommand::StartReplication {
                slot,
                kind,
                start_lsn,
                timeline,
            }))
        }
        _ => Ok(None),
    }
}

/// Parse a `pg_lsn` text representation back into a u64.
pub fn parse_lsn(s: &str) -> ProtocolResult<u64> {
    let (hi, lo) = s.split_once('/').ok_or(ProtocolError::InvalidValue {
        field: "replication.lsn",
        reason: "missing /".into(),
    })?;
    let hi = u64::from_str_radix(hi, 16).map_err(|e| ProtocolError::InvalidValue {
        field: "replication.lsn.hi",
        reason: e.to_string(),
    })?;
    let lo = u64::from_str_radix(lo, 16).map_err(|e| ProtocolError::InvalidValue {
        field: "replication.lsn.lo",
        reason: e.to_string(),
    })?;
    if hi > u32::MAX as u64 || lo > u32::MAX as u64 {
        return Err(ProtocolError::InvalidValue {
            field: "replication.lsn",
            reason: "out of range".into(),
        });
    }
    Ok((hi << 32) | lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_epoch_constant_is_2000_01_01_utc() {
        // Unix epoch 2000-01-01 00:00:00 UTC = 946684800 seconds.
        assert_eq!(PG_EPOCH_MICROS, 946_684_800 * 1_000_000);
    }

    #[test]
    fn xlog_data_round_trip() {
        let x = XLogData {
            wal_start: 0x1234,
            wal_end: 0x5678,
            server_send_time_pg: 12345,
            data: b"hello".to_vec(),
        };
        let bytes = encode_xlog_data(&x);
        let decoded = decode_server_msg(&bytes).unwrap();
        assert_eq!(decoded, ReplicationServerMsg::XLogData(x));
    }

    #[test]
    fn xlog_data_empty_payload_round_trip() {
        let x = XLogData {
            wal_start: 0,
            wal_end: 0,
            server_send_time_pg: 0,
            data: vec![],
        };
        let bytes = encode_xlog_data(&x);
        let decoded = decode_server_msg(&bytes).unwrap();
        assert_eq!(decoded, ReplicationServerMsg::XLogData(x));
    }

    #[test]
    fn keepalive_round_trip_requesting_reply() {
        let k = PrimaryKeepalive {
            wal_end: 0xDEAD_BEEF,
            server_send_time_pg: 999,
            reply_requested: true,
        };
        let bytes = encode_keepalive(&k);
        let decoded = decode_server_msg(&bytes).unwrap();
        assert_eq!(decoded, ReplicationServerMsg::Keepalive(k));
    }

    #[test]
    fn keepalive_no_reply_round_trip() {
        let k = PrimaryKeepalive {
            wal_end: 1,
            server_send_time_pg: -10,
            reply_requested: false,
        };
        let bytes = encode_keepalive(&k);
        let decoded = decode_server_msg(&bytes).unwrap();
        assert_eq!(decoded, ReplicationServerMsg::Keepalive(k));
    }

    #[test]
    fn standby_status_round_trip() {
        let s = StandbyStatusUpdate {
            last_received_lsn: 0x100,
            last_flushed_lsn: 0x90,
            last_applied_lsn: 0x80,
            client_send_time_pg: 1234567890,
            reply_requested: false,
        };
        let bytes = encode_standby_status(&s);
        let decoded = decode_client_msg(&bytes).unwrap();
        assert_eq!(decoded, ReplicationClientMsg::StandbyStatus(s));
    }

    #[test]
    fn hot_standby_feedback_round_trip() {
        let h = HotStandbyFeedback {
            client_send_time_pg: 4242,
            current_xmin: 100,
            current_xmin_epoch: 0,
            current_catalog_xmin: 99,
            current_catalog_xmin_epoch: 0,
        };
        let bytes = encode_hot_standby_feedback(&h);
        let decoded = decode_client_msg(&bytes).unwrap();
        assert_eq!(decoded, ReplicationClientMsg::HotStandbyFeedback(h));
    }

    #[test]
    fn server_msg_unknown_tag_errors() {
        let err = decode_server_msg(b"X").unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => {
                assert_eq!(field, "replication.server_tag")
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn client_msg_unknown_tag_errors() {
        let err = decode_client_msg(b"Z").unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => {
                assert_eq!(field, "replication.client_tag")
            }
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn server_msg_truncated_errors() {
        let err = decode_server_msg(b"w\x00\x00").unwrap_err();
        match err {
            ProtocolError::Incomplete { .. } => {}
            other => panic!("expected Incomplete, got {:?}", other),
        }
    }

    #[test]
    fn lsn_format_zero() {
        assert_eq!(format_lsn(0), "0/0");
    }

    #[test]
    fn lsn_format_split() {
        assert_eq!(format_lsn(0x1_8000_0000), "1/80000000");
    }

    #[test]
    fn lsn_format_max() {
        assert_eq!(format_lsn(u64::MAX), "FFFFFFFF/FFFFFFFF");
    }

    #[test]
    fn lsn_parse_round_trip() {
        for v in [0, 1, 0x80000000, 0x1_8000_0000, u64::MAX] {
            let s = format_lsn(v);
            assert_eq!(parse_lsn(&s).unwrap(), v);
        }
    }

    #[test]
    fn lsn_parse_invalid_no_slash() {
        let err = parse_lsn("12345").unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => assert_eq!(field, "replication.lsn"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn lsn_parse_overflow_errors() {
        let err = parse_lsn("100000000/0").unwrap_err();
        match err {
            ProtocolError::InvalidValue { .. } => {}
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn parse_identify_system() {
        let cmd = parse_repl_command("IDENTIFY_SYSTEM").unwrap().unwrap();
        assert_eq!(cmd, ReplCommand::IdentifySystem);
    }

    #[test]
    fn parse_identify_system_lowercase() {
        let cmd = parse_repl_command("identify_system").unwrap().unwrap();
        assert_eq!(cmd, ReplCommand::IdentifySystem);
    }

    #[test]
    fn parse_identify_system_with_semicolon() {
        let cmd = parse_repl_command("IDENTIFY_SYSTEM;").unwrap().unwrap();
        assert_eq!(cmd, ReplCommand::IdentifySystem);
    }

    #[test]
    fn parse_start_replication_physical_no_slot() {
        let cmd = parse_repl_command("START_REPLICATION 0/16000000")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            ReplCommand::StartReplication {
                slot: None,
                kind: ReplKind::Physical,
                start_lsn: 0x16000000,
                timeline: None,
            }
        );
    }

    #[test]
    fn parse_start_replication_physical_with_slot_and_timeline() {
        let cmd = parse_repl_command("START_REPLICATION SLOT my_slot PHYSICAL 1/0 TIMELINE 7")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            ReplCommand::StartReplication {
                slot: Some("my_slot".into()),
                kind: ReplKind::Physical,
                start_lsn: 0x1_0000_0000,
                timeline: Some(7),
            }
        );
    }

    #[test]
    fn parse_start_replication_logical() {
        let cmd = parse_repl_command("START_REPLICATION SLOT decoder LOGICAL 0/0")
            .unwrap()
            .unwrap();
        assert_eq!(
            cmd,
            ReplCommand::StartReplication {
                slot: Some("decoder".into()),
                kind: ReplKind::Logical,
                start_lsn: 0,
                timeline: None,
            }
        );
    }

    #[test]
    fn parse_start_replication_missing_lsn_errors() {
        let err = parse_repl_command("START_REPLICATION").unwrap_err();
        match err {
            ProtocolError::InvalidValue { field, .. } => assert_eq!(field, "START_REPLICATION"),
            other => panic!("expected InvalidValue, got {:?}", other),
        }
    }

    #[test]
    fn parse_timeline_history() {
        let cmd = parse_repl_command("TIMELINE_HISTORY 5").unwrap().unwrap();
        assert_eq!(cmd, ReplCommand::TimelineHistory { timeline: 5 });
    }

    #[test]
    fn parse_unknown_returns_none() {
        let cmd = parse_repl_command("SELECT 1").unwrap();
        assert!(cmd.is_none());
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(parse_repl_command("").unwrap().is_none());
        assert!(parse_repl_command("   ").unwrap().is_none());
        assert!(parse_repl_command(";").unwrap().is_none());
    }

    #[test]
    fn identify_system_data_row_text_format() {
        let reply = IdentifySystemReply {
            system_id: "7398765432109876543".into(),
            timeline: 1,
            xlog_pos: 0x16ABCDEF,
            dbname: Some("postgres".into()),
        };
        let row = reply.to_data_row();
        assert_eq!(row.len(), 4);
        assert_eq!(row[0].as_deref(), Some(&b"7398765432109876543"[..]));
        assert_eq!(row[1].as_deref(), Some(&b"1"[..]));
        assert_eq!(row[2].as_deref(), Some(&b"0/16ABCDEF"[..]));
        assert_eq!(row[3].as_deref(), Some(&b"postgres"[..]));
    }

    #[test]
    fn identify_system_data_row_no_dbname_is_null() {
        let reply = IdentifySystemReply {
            system_id: "1".into(),
            timeline: 1,
            xlog_pos: 0,
            dbname: None,
        };
        let row = reply.to_data_row();
        assert_eq!(row[3], None);
    }
}
