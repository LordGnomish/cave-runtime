// SPDX-License-Identifier: AGPL-3.0-or-later
//! `bufsize` plugin — EDNS0 UDP buffer size policy.
//!
//! Port of the size-policy core of CoreDNS v1.14.3 `plugin/bufsize`
//! (`setup.go` range validation + `bufsize.go` clamp-down rule):
//! ```go
//! if option.UDPSize() > buf.Size { option.SetUDPSize(buf.Size) }
//! ```

use crate::{DnsError, DnsResult};

/// Smallest permitted EDNS0 buffer size (`setup.go`).
pub const MIN_BUFSIZE: u16 = 512;
/// Largest permitted EDNS0 buffer size (`setup.go`).
pub const MAX_BUFSIZE: u16 = 4096;

/// Configured EDNS0 buffer-size limiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bufsize {
    /// The cap applied to advertised UDP payload sizes.
    pub size: u16,
}

impl Bufsize {
    /// Build a `Bufsize`, rejecting any cap outside `[512, 4096]`.
    pub fn new(size: u16) -> DnsResult<Self> {
        if !(MIN_BUFSIZE..=MAX_BUFSIZE).contains(&size) {
            return Err(DnsError::Config(format!(
                "size value {size} not in the range 512 - 4096"
            )));
        }
        Ok(Self { size })
    }

    /// Clamp an advertised UDP payload size down to the configured cap; never
    /// raises a smaller request.
    #[must_use]
    pub fn clamp(&self, requested: u16) -> u16 {
        if requested > self.size {
            self.size
        } else {
            requested
        }
    }
}
