// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP/2 settings + stream-id helpers. Envoy `http2_protocol_options` reference.

use crate::error::{AGwError, AGwResult};

#[derive(Debug, Clone, Copy)]
pub struct Http2Settings {
    pub max_concurrent_streams: u32, pub initial_window_size: u32,
    pub max_frame_size: u32, pub max_header_list_size: u32, pub enable_push: bool,
}
impl Default for Http2Settings {
    fn default() -> Self {
        Self { max_concurrent_streams: 100, initial_window_size: 65_535,
            max_frame_size: 16_384, max_header_list_size: 8_192, enable_push: false }
    }
}
impl Http2Settings {
    pub fn validate(&self) -> AGwResult<()> {
        if self.max_frame_size < 16_384 || self.max_frame_size > 16_777_215 {
            return Err(AGwError::BadRequest(format!("max_frame_size out of range: {}", self.max_frame_size)));
        }
        if self.initial_window_size > 2_147_483_647 {
            return Err(AGwError::BadRequest("initial_window_size > 2^31-1".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamId(pub u32);
impl StreamId {
    pub fn is_client(self) -> bool { self.0 % 2 == 1 }
    pub fn next_client(prev: u32) -> StreamId { StreamId(if prev == 0 { 1 } else { prev + 2 }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn defaults_validate() { Http2Settings::default().validate().unwrap(); }
    #[test] fn reject_small_frame() {
        let s = Http2Settings { max_frame_size: 8, ..Default::default() };
        assert!(s.validate().is_err());
    }
    #[test] fn reject_huge_window() {
        let s = Http2Settings { initial_window_size: u32::MAX, ..Default::default() };
        assert!(s.validate().is_err());
    }
    #[test] fn stream_parity() {
        assert!(StreamId(1).is_client()); assert!(StreamId(3).is_client()); assert!(!StreamId(2).is_client());
    }
    #[test] fn next_client_odd() {
        assert_eq!(StreamId::next_client(0).0, 1);
        assert_eq!(StreamId::next_client(1).0, 3);
        assert_eq!(StreamId::next_client(5).0, 7);
    }
}
