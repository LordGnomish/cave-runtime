//! Idempotent-producer state machine — KIP-98 §3.1.
//!
//! Each producer obtains a Producer ID (PID) and starts at epoch 0. Every
//! record carries `(PID, epoch, sequence)`; the broker accepts only when
//! `(epoch, sequence)` is exactly `(current_epoch, last_sequence + 1)` —
//! anything else is a duplicate or out-of-order error.
//!
//! `epoch` is bumped on producer fence (e.g. another producer registered
//! with the same transactional id).

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProducerError {
    #[error("duplicate sequence {0}")]
    DuplicateSequence(i32),
    #[error("out-of-order sequence: expected {expected}, got {got}")]
    OutOfOrderSequence { expected: i32, got: i32 },
    #[error("unknown producer id: {0}")]
    UnknownPid(i64),
    #[error("producer fenced: epoch {0} < current {1}")]
    Fenced(i16, i16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerState {
    pub pid: i64,
    pub epoch: i16,
    /// last_sequence per (topic, partition).
    pub sequences: BTreeMap<(String, i32), i32>,
}

impl ProducerState {
    pub fn new(pid: i64) -> Self {
        Self {
            pid,
            epoch: 0,
            sequences: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct ProducerRegistry {
    producers: BTreeMap<i64, ProducerState>,
    next_pid: i64,
}

impl ProducerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allocate_pid(&mut self) -> i64 {
        self.next_pid += 1;
        let pid = self.next_pid;
        self.producers.insert(pid, ProducerState::new(pid));
        pid
    }

    pub fn fence(&mut self, pid: i64) -> Result<i16, ProducerError> {
        let state = self
            .producers
            .get_mut(&pid)
            .ok_or(ProducerError::UnknownPid(pid))?;
        state.epoch += 1;
        state.sequences.clear();
        Ok(state.epoch)
    }

    /// Validate a record `(pid, epoch, sequence)` for `(topic, partition)`.
    /// On success, advances `last_sequence`.
    pub fn validate(
        &mut self,
        pid: i64,
        epoch: i16,
        topic: &str,
        partition: i32,
        sequence: i32,
    ) -> Result<(), ProducerError> {
        let state = self
            .producers
            .get_mut(&pid)
            .ok_or(ProducerError::UnknownPid(pid))?;
        if epoch < state.epoch {
            return Err(ProducerError::Fenced(epoch, state.epoch));
        }
        let key = (topic.to_string(), partition);
        let last = state.sequences.get(&key).copied().unwrap_or(-1);
        if sequence == last {
            return Err(ProducerError::DuplicateSequence(sequence));
        }
        if sequence != last + 1 {
            return Err(ProducerError::OutOfOrderSequence {
                expected: last + 1,
                got: sequence,
            });
        }
        state.sequences.insert(key, sequence);
        Ok(())
    }

    pub fn epoch(&self, pid: i64) -> Option<i16> {
        self.producers.get(&pid).map(|p| p.epoch)
    }

    pub fn sequence(&self, pid: i64, topic: &str, partition: i32) -> Option<i32> {
        self.producers
            .get(&pid)
            .and_then(|p| p.sequences.get(&(topic.into(), partition)).copied())
    }

    pub fn count(&self) -> usize {
        self.producers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_pid_returns_monotonic() {
        let mut r = ProducerRegistry::new();
        let p1 = r.allocate_pid();
        let p2 = r.allocate_pid();
        assert!(p2 > p1);
    }

    #[test]
    fn first_sequence_must_be_zero() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        // Sequence 0 = (last_sequence -1) + 1 = 0 ✓
        assert!(r.validate(pid, 0, "t", 0, 0).is_ok());
    }

    #[test]
    fn duplicate_sequence_errors() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        r.validate(pid, 0, "t", 0, 0).unwrap();
        let err = r.validate(pid, 0, "t", 0, 0).unwrap_err();
        assert_eq!(err, ProducerError::DuplicateSequence(0));
    }

    #[test]
    fn out_of_order_sequence_errors() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        let err = r.validate(pid, 0, "t", 0, 5).unwrap_err();
        assert_eq!(
            err,
            ProducerError::OutOfOrderSequence {
                expected: 0,
                got: 5
            }
        );
    }

    #[test]
    fn fence_increments_epoch() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        let new_epoch = r.fence(pid).unwrap();
        assert_eq!(new_epoch, 1);
        assert_eq!(r.epoch(pid), Some(1));
    }

    #[test]
    fn fence_resets_sequences() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        r.validate(pid, 0, "t", 0, 0).unwrap();
        r.validate(pid, 0, "t", 0, 1).unwrap();
        r.fence(pid).unwrap();
        // After fence, sequence resets — first record at new epoch must be 0 again.
        assert!(r.validate(pid, 1, "t", 0, 0).is_ok());
    }

    #[test]
    fn old_epoch_fenced() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        r.fence(pid).unwrap();
        // Producer with old epoch 0 should be fenced (current epoch = 1).
        let err = r.validate(pid, 0, "t", 0, 0).unwrap_err();
        assert!(matches!(err, ProducerError::Fenced(0, 1)));
    }

    #[test]
    fn separate_partitions_have_separate_sequences() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        r.validate(pid, 0, "t", 0, 0).unwrap();
        // Partition 1 starts fresh.
        assert!(r.validate(pid, 0, "t", 1, 0).is_ok());
    }

    #[test]
    fn separate_topics_have_separate_sequences() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        r.validate(pid, 0, "t1", 0, 0).unwrap();
        assert!(r.validate(pid, 0, "t2", 0, 0).is_ok());
    }

    #[test]
    fn unknown_pid_errors() {
        let mut r = ProducerRegistry::new();
        let err = r.validate(999, 0, "t", 0, 0).unwrap_err();
        assert_eq!(err, ProducerError::UnknownPid(999));
    }

    #[test]
    fn fence_unknown_pid_errors() {
        let mut r = ProducerRegistry::new();
        let err = r.fence(999).unwrap_err();
        assert_eq!(err, ProducerError::UnknownPid(999));
    }

    #[test]
    fn sequence_advances_after_each_validate() {
        let mut r = ProducerRegistry::new();
        let pid = r.allocate_pid();
        for i in 0..5 {
            r.validate(pid, 0, "t", 0, i).unwrap();
        }
        assert_eq!(r.sequence(pid, "t", 0), Some(4));
    }

    #[test]
    fn count_reflects_allocations() {
        let mut r = ProducerRegistry::new();
        r.allocate_pid();
        r.allocate_pid();
        assert_eq!(r.count(), 2);
    }
}
