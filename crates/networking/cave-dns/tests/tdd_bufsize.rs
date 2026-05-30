// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: CoreDNS v1.14.3 `plugin/bufsize/bufsize.go` behaviour.
//!
//! Upstream clamps the requester's advertised EDNS0 UDP buffer size when it
//! exceeds the configured maximum (RFC 6891 / dnsflagday.net 2020 → 1232).

use cave_dns::plugins::bufsize::BufsizePlugin;

#[test]
fn clamps_oversized_to_configured_max() {
    // bufsize.go: `if int(option.UDPSize()) > buf.Size { option.SetUDPSize(buf.Size) }`
    let p = BufsizePlugin::new(1232);
    assert_eq!(p.clamp(4096), 1232);
}

#[test]
fn leaves_smaller_payload_untouched() {
    let p = BufsizePlugin::new(1232);
    assert_eq!(p.clamp(512), 512);
    assert_eq!(p.clamp(1232), 1232);
}

#[test]
fn default_follows_dns_flag_day_1232() {
    // setup.go: `const defaultBufSize = 1232`
    assert_eq!(BufsizePlugin::default_size(), 1232);
}

#[test]
fn rejects_out_of_rfc6891_range() {
    // setup.go: `if bufsize < 512 || bufsize > 4096 { return error }`
    assert!(BufsizePlugin::validate(511).is_err());
    assert!(BufsizePlugin::validate(4097).is_err());
    assert!(BufsizePlugin::validate(512).is_ok());
    assert!(BufsizePlugin::validate(4096).is_ok());
}
