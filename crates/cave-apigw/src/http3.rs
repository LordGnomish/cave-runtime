// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP/3 (QUIC) listener config. Envoy `Http3ProtocolOptions` reference.
//! Real packet plumbing lives in `cave-net::quic` (Quinn-based).

use crate::error::{AGwError, AGwResult};

#[derive(Debug, Clone, Copy)]
pub struct Http3Settings {
    pub enabled: bool, pub max_concurrent_streams: u64, pub max_idle_timeout_ms: u64,
    pub max_udp_payload_size: u16, pub initial_max_data: u64,
    pub advertise_alt_svc: bool, pub alt_svc_ma_secs: u32,
}
impl Default for Http3Settings {
    fn default() -> Self {
        Self { enabled: false, max_concurrent_streams: 100, max_idle_timeout_ms: 30_000,
            max_udp_payload_size: 1_500, initial_max_data: 10 * 1024 * 1024,
            advertise_alt_svc: true, alt_svc_ma_secs: 86_400 }
    }
}
impl Http3Settings {
    pub fn validate(&self) -> AGwResult<()> {
        if !self.enabled { return Ok(()); }
        if self.max_udp_payload_size < 1_200 { return Err(AGwError::BadRequest("max_udp_payload_size < 1200".into())); }
        if self.max_idle_timeout_ms == 0 { return Err(AGwError::BadRequest("idle timeout > 0".into())); }
        Ok(())
    }
    pub fn alt_svc_header(&self, port: u16) -> Option<String> {
        if !self.enabled || !self.advertise_alt_svc { return None; }
        Some(format!("h3=\":{port}\"; ma={}", self.alt_svc_ma_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn disabled_ok() { Http3Settings::default().validate().unwrap(); }
    #[test] fn small_udp_rejected() {
        let s = Http3Settings { enabled: true, max_udp_payload_size: 500, ..Default::default() };
        assert!(s.validate().is_err());
    }
    #[test] fn alt_svc_none_when_disabled() { assert!(Http3Settings::default().alt_svc_header(443).is_none()); }
    #[test] fn alt_svc_when_enabled() {
        let s = Http3Settings { enabled: true, advertise_alt_svc: true, ..Default::default() };
        let h = s.alt_svc_header(443).unwrap();
        assert!(h.starts_with("h3=\":443\"")); assert!(h.contains("ma=86400"));
    }
    #[test] fn zero_idle_rejected() {
        let s = Http3Settings { enabled: true, max_idle_timeout_ms: 0, ..Default::default() };
        assert!(s.validate().is_err());
    }
}
