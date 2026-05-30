// SPDX-License-Identifier: AGPL-3.0-or-later
//! bufsize plugin tests (CoreDNS v1.14.3 plugin/bufsize).
use cave_dns::bufsize::{Bufsize, MAX_BUFSIZE, MIN_BUFSIZE};

#[test]
fn config_accepts_boundaries() {
    assert!(Bufsize::new(MIN_BUFSIZE).is_ok());
    assert!(Bufsize::new(MAX_BUFSIZE).is_ok());
    assert!(Bufsize::new(1232).is_ok());
}
#[test]
fn config_rejects_out_of_range() {
    assert!(Bufsize::new(511).is_err());
    assert!(Bufsize::new(4097).is_err());
    assert!(Bufsize::new(0).is_err());
}
#[test]
fn clamp_lowers_when_requested_exceeds_cap() {
    assert_eq!(Bufsize::new(1232).unwrap().clamp(4096), 1232);
}
#[test]
fn clamp_leaves_smaller_request_untouched() {
    let b = Bufsize::new(1232).unwrap();
    assert_eq!(b.clamp(512), 512);
    assert_eq!(b.clamp(1232), 1232);
}
#[test]
fn clamp_at_exact_cap_is_identity() {
    let b = Bufsize::new(2048).unwrap();
    assert_eq!(b.clamp(2048), 2048);
    assert_eq!(b.clamp(2049), 2048);
}
