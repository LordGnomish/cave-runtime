// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BPF ring-buffer reader — userspace model of `BPF_MAP_TYPE_RINGBUF`,
//! the transport grafana/beyla uses to ship events from kernel probes to
//! its userspace pipeline.
//!
//! The real ring buffer is a single mmap'd byte region with a producer
//! and a consumer position; `bpf_ringbuf_reserve` carves out a record
//! (prefixed by an 8-byte `BPF_RINGBUF_HDR_SZ` header carrying length and
//! busy/discard flags), and the consumer walks committed records strictly
//! in submission order — it must stop at a record still being written
//! (the "busy" bit), because reordering would corrupt the stream.
//!
//! This model reproduces that contract — header overhead, capacity-bound
//! reservation, in-order consumption past committed/discarded records, and
//! blocking behind a busy front — over a `VecDeque` of records rather than
//! a raw mmap. It is enough to exercise the event pipeline in tests.

use std::collections::VecDeque;

/// Per-record header size, mirroring `BPF_RINGBUF_HDR_SZ`.
pub const HDR_SZ: usize = 8;

/// Producer reservation handle (a monotonic token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Reserved,
    Committed,
    Discarded,
}

#[derive(Debug, Clone)]
struct Record {
    token: Token,
    state: State,
    payload: Vec<u8>,
}

impl Record {
    fn footprint(&self) -> usize {
        HDR_SZ + self.payload.len()
    }
}

/// Errors from the producer side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingBufError {
    /// Insufficient free space for the reservation (`reserve` returns NULL
    /// in the kernel).
    Full,
}

/// Userspace ring buffer.
#[derive(Debug, Clone)]
pub struct RingBuf {
    capacity: usize,
    outstanding: usize,
    next_token: u64,
    records: VecDeque<Record>,
}

impl RingBuf {
    /// Create a ring buffer with `capacity` bytes of usable space.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            outstanding: 0,
            next_token: 0,
            records: VecDeque::new(),
        }
    }

    /// Reserve space for and stage `payload`. Returns a [`Token`] to later
    /// [`commit`](Self::commit) or [`discard`](Self::discard), or
    /// [`RingBufError::Full`] if the record (with header) does not fit in
    /// the remaining space.
    pub fn reserve(&mut self, payload: Vec<u8>) -> Result<Token, RingBufError> {
        let footprint = HDR_SZ + payload.len();
        if footprint > self.capacity || self.outstanding + footprint > self.capacity {
            return Err(RingBufError::Full);
        }
        let token = Token(self.next_token);
        self.next_token += 1;
        self.outstanding += footprint;
        self.records.push_back(Record {
            token,
            state: State::Reserved,
            payload,
        });
        Ok(token)
    }

    fn set_state(&mut self, token: Token, state: State) {
        if let Some(r) = self.records.iter_mut().find(|r| r.token == token) {
            r.state = state;
        }
    }

    /// Mark a reserved record ready for the consumer.
    pub fn commit(&mut self, token: Token) {
        self.set_state(token, State::Committed);
    }

    /// Mark a reserved record as discarded — it keeps its slot in order
    /// but is skipped by the consumer.
    pub fn discard(&mut self, token: Token) {
        self.set_state(token, State::Discarded);
    }

    /// Drain committed records in submission order, skipping discarded
    /// ones, and stop at the first record still being written (busy).
    /// Reclaims the freed bytes.
    pub fn consume(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(front) = self.records.front() {
            match front.state {
                State::Reserved => break, // busy: cannot advance past it
                State::Committed => {
                    let r = self.records.pop_front().unwrap();
                    self.outstanding -= r.footprint();
                    out.push(r.payload);
                }
                State::Discarded => {
                    let r = self.records.pop_front().unwrap();
                    self.outstanding -= r.footprint();
                }
            }
        }
        out
    }

    /// Free bytes available for new reservations.
    pub fn available(&self) -> usize {
        self.capacity - self.outstanding
    }

    /// Number of records still occupying the buffer.
    pub fn pending(&self) -> usize {
        self.records.len()
    }
}
