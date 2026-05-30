// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secondary-zone periodic refresh scheduling (RFC 1035 §3.3.13 / RFC 1996).
//!
//! Closes the periodic-refresh half of the `secondary` feature: the
//! refresh/retry/expire state machine over the SOA timers.

use hickory_proto::rr::{RData, Record};

/// SOA timing parameters, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoaTimers {
    /// REFRESH interval.
    pub refresh: u32,
    /// RETRY interval.
    pub retry: u32,
    /// EXPIRE interval.
    pub expire: u32,
}

/// The next maintenance step a secondary should take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshAction {
    /// Sleep this many seconds.
    Wait(u64),
    /// Refresh now.
    Refresh,
    /// Retry now after a failure.
    Retry,
    /// Zone expired.
    Expired,
}

impl SoaTimers {
    /// Read REFRESH/RETRY/EXPIRE from an SOA record (negative timers clamp to 0).
    #[must_use]
    pub fn from_soa(rr: &Record) -> Option<Self> {
        match rr.data() {
            Some(RData::SOA(soa)) => Some(Self {
                refresh: soa.refresh().max(0) as u32,
                retry: soa.retry().max(0) as u32,
                expire: soa.expire().max(0) as u32,
            }),
            _ => None,
        }
    }

    /// Decide the next action from now + last-success + last-attempt times.
    #[must_use]
    pub fn next_action(&self, now: u64, last_success: u64, last_attempt: u64) -> RefreshAction {
        let since_success = now.saturating_sub(last_success);
        if since_success >= u64::from(self.expire) {
            return RefreshAction::Expired;
        }
        if last_attempt > last_success {
            let since_attempt = now.saturating_sub(last_attempt);
            if since_attempt >= u64::from(self.retry) {
                RefreshAction::Retry
            } else {
                RefreshAction::Wait(u64::from(self.retry) - since_attempt)
            }
        } else if since_success >= u64::from(self.refresh) {
            RefreshAction::Refresh
        } else {
            RefreshAction::Wait(u64::from(self.refresh) - since_success)
        }
    }
}

/// Whether `candidate` is newer than `current` in SOA serial space (RFC 1982).
#[must_use]
pub fn serial_newer(current: u32, candidate: u32) -> bool {
    let diff = candidate.wrapping_sub(current);
    diff != 0 && diff < 0x8000_0000
}
