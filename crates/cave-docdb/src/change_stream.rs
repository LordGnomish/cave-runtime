//! Change streams — opLog-style change-event broker with resume tokens.
//!
//! Mirrors the MongoDB change-stream model documented at
//! <https://www.mongodb.com/docs/manual/changeStreams/>:
//!
//! * Each event carries an `_id` (resume token) — base64-hex of the cluster
//!   timestamp + stream sequence.
//! * Operation types: insert, update, replace, delete, drop, rename,
//!   dropDatabase, invalidate.
//! * Subscribers can resume from any token (`$resumeAfter` /
//!   `$startAfter`) or from a wall-clock timestamp (`$startAtOperationTime`).

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationType {
    Insert,
    Update,
    Replace,
    Delete,
    Drop,
    Rename,
    DropDatabase,
    Invalidate,
}

impl OperationType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Replace => "replace",
            Self::Delete => "delete",
            Self::Drop => "drop",
            Self::Rename => "rename",
            Self::DropDatabase => "dropDatabase",
            Self::Invalidate => "invalidate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvent {
    pub resume_token: ResumeToken,
    pub operation_type: OperationType,
    pub cluster_time_ms: u64,
    pub ns: String,
    pub document_key: Option<String>,
    pub full_document: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResumeToken {
    pub cluster_time_ms: u64,
    pub sequence: u64,
}

impl ResumeToken {
    pub fn new(cluster_time_ms: u64, sequence: u64) -> Self {
        Self {
            cluster_time_ms,
            sequence,
        }
    }
    pub fn encode(&self) -> String {
        format!("{:016x}{:016x}", self.cluster_time_ms, self.sequence)
    }
    pub fn decode(s: &str) -> Option<Self> {
        if s.len() != 32 {
            return None;
        }
        let ts = u64::from_str_radix(&s[..16], 16).ok()?;
        let seq = u64::from_str_radix(&s[16..], 16).ok()?;
        Some(Self {
            cluster_time_ms: ts,
            sequence: seq,
        })
    }
}

#[derive(Debug)]
pub struct ChangeStream {
    events: VecDeque<ChangeEvent>,
    capacity: usize,
    next_sequence: u64,
}

impl ChangeStream {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
            next_sequence: 1,
        }
    }

    pub fn publish(
        &mut self,
        op: OperationType,
        ns: &str,
        cluster_time_ms: u64,
        document_key: Option<String>,
        full_document: Option<Vec<u8>>,
    ) -> ResumeToken {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        let token = ResumeToken::new(cluster_time_ms, seq);
        self.events.push_back(ChangeEvent {
            resume_token: token.clone(),
            operation_type: op,
            cluster_time_ms,
            ns: ns.to_string(),
            document_key,
            full_document,
        });
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        token
    }

    /// Returns events strictly after the given token.
    pub fn resume_after(&self, token: &ResumeToken) -> Vec<&ChangeEvent> {
        self.events
            .iter()
            .filter(|e| e.resume_token > *token)
            .collect()
    }

    /// Returns events at or after the given token (inclusive — `$startAfter`).
    pub fn start_after(&self, token: &ResumeToken) -> Vec<&ChangeEvent> {
        self.events
            .iter()
            .filter(|e| e.resume_token >= *token)
            .collect()
    }

    /// Returns events whose cluster time is at or after the given timestamp.
    pub fn start_at_operation_time(&self, ts_ms: u64) -> Vec<&ChangeEvent> {
        self.events
            .iter()
            .filter(|e| e.cluster_time_ms >= ts_ms)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn latest_token(&self) -> Option<ResumeToken> {
        self.events.back().map(|e| e.resume_token.clone())
    }

    pub fn invalidate(&mut self, ns: &str, cluster_time_ms: u64) -> ResumeToken {
        self.publish(OperationType::Invalidate, ns, cluster_time_ms, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_type_strings_match_mongo() {
        assert_eq!(OperationType::Insert.as_str(), "insert");
        assert_eq!(OperationType::Delete.as_str(), "delete");
        assert_eq!(OperationType::Invalidate.as_str(), "invalidate");
    }

    #[test]
    fn resume_token_encode_decode_round_trip() {
        let t = ResumeToken::new(0xDEAD_BEEF, 42);
        let enc = t.encode();
        assert_eq!(enc.len(), 32);
        assert_eq!(ResumeToken::decode(&enc).unwrap(), t);
    }

    #[test]
    fn resume_token_decode_wrong_length() {
        assert!(ResumeToken::decode("abc").is_none());
    }

    #[test]
    fn resume_token_ordering() {
        let a = ResumeToken::new(100, 1);
        let b = ResumeToken::new(100, 2);
        let c = ResumeToken::new(101, 0);
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn publish_assigns_increasing_sequences() {
        let mut s = ChangeStream::new(100);
        let t1 = s.publish(OperationType::Insert, "db.c", 0, None, None);
        let t2 = s.publish(OperationType::Insert, "db.c", 0, None, None);
        assert!(t1 < t2);
    }

    #[test]
    fn publish_respects_capacity() {
        let mut s = ChangeStream::new(3);
        for _ in 0..10 {
            s.publish(OperationType::Insert, "db.c", 0, None, None);
        }
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn resume_after_excludes_token_itself() {
        let mut s = ChangeStream::new(100);
        let t1 = s.publish(OperationType::Insert, "db.c", 0, None, None);
        let _ = s.publish(OperationType::Insert, "db.c", 0, None, None);
        let after = s.resume_after(&t1);
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn start_after_includes_token() {
        let mut s = ChangeStream::new(100);
        let t1 = s.publish(OperationType::Insert, "db.c", 0, None, None);
        let _ = s.publish(OperationType::Insert, "db.c", 0, None, None);
        let after = s.start_after(&t1);
        assert_eq!(after.len(), 2);
    }

    #[test]
    fn start_at_operation_time_filters() {
        let mut s = ChangeStream::new(100);
        s.publish(OperationType::Insert, "db.c", 100, None, None);
        s.publish(OperationType::Insert, "db.c", 200, None, None);
        s.publish(OperationType::Insert, "db.c", 300, None, None);
        let from_200 = s.start_at_operation_time(200);
        assert_eq!(from_200.len(), 2);
    }

    #[test]
    fn latest_token_returns_most_recent() {
        let mut s = ChangeStream::new(100);
        let _ = s.publish(OperationType::Insert, "db.c", 0, None, None);
        let t2 = s.publish(OperationType::Update, "db.c", 0, None, None);
        assert_eq!(s.latest_token().unwrap(), t2);
    }

    #[test]
    fn invalidate_emits_event() {
        let mut s = ChangeStream::new(100);
        let t = s.invalidate("db.c", 100);
        let evt = s.events.back().unwrap();
        assert_eq!(evt.operation_type, OperationType::Invalidate);
        assert_eq!(evt.resume_token, t);
    }

    #[test]
    fn empty_stream() {
        let s = ChangeStream::new(10);
        assert!(s.is_empty());
        assert!(s.latest_token().is_none());
    }

    #[test]
    fn full_document_carried_through() {
        let mut s = ChangeStream::new(10);
        s.publish(
            OperationType::Insert,
            "db.c",
            0,
            Some("\"id\":1".into()),
            Some(b"{\"x\":1}".to_vec()),
        );
        let evt = s.events.back().unwrap();
        assert_eq!(evt.full_document.as_deref(), Some(&b"{\"x\":1}"[..]));
    }
}
