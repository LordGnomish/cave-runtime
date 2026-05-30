// SPDX-License-Identifier: AGPL-3.0-or-later
//! Secondary refresh scheduling tests (RFC 1035/1982; closes secondary partial).
use cave_dns::secondary_refresh::{serial_newer, RefreshAction, SoaTimers};
use hickory_proto::rr::rdata::{A, SOA};
use hickory_proto::rr::{Name, RData, Record};
use std::net::Ipv4Addr;
use std::str::FromStr;

fn name(s: &str) -> Name { Name::from_str(s).unwrap() }
fn soa(refresh: i32, retry: i32, expire: i32) -> Record {
    let rdata = SOA::new(name("ns.example.org."), name("admin.example.org."), 7, refresh, retry, expire, 3600);
    Record::from_rdata(name("example.org."), 3600, RData::SOA(rdata))
}

#[test]
fn parses_timers_from_soa() {
    let t = SoaTimers::from_soa(&soa(7200, 3600, 1_209_600)).unwrap();
    assert_eq!(t.refresh, 7200);
    assert_eq!(t.retry, 3600);
    assert_eq!(t.expire, 1_209_600);
}
#[test]
fn from_soa_rejects_non_soa() {
    let a = Record::from_rdata(name("x."), 60, RData::A(A("1.2.3.4".parse::<Ipv4Addr>().unwrap())));
    assert!(SoaTimers::from_soa(&a).is_none());
}
#[test]
fn waits_inside_refresh_interval() {
    let t = SoaTimers::from_soa(&soa(7200, 3600, 1_209_600)).unwrap();
    let now = 10_000;
    assert_eq!(t.next_action(now, now - 1000, now - 1000), RefreshAction::Wait(7200 - 1000));
}
#[test]
fn refreshes_when_interval_elapsed() {
    let t = SoaTimers::from_soa(&soa(7200, 3600, 1_209_600)).unwrap();
    let now = 100_000;
    assert_eq!(t.next_action(now, now - 7200, now - 7200), RefreshAction::Refresh);
}
#[test]
fn retries_after_failure() {
    let t = SoaTimers::from_soa(&soa(7200, 3600, 1_209_600)).unwrap();
    let now = 100_000;
    assert_eq!(t.next_action(now, now - 8000, now - 3600), RefreshAction::Retry);
}
#[test]
fn waits_retry_interval_after_recent_failure() {
    let t = SoaTimers::from_soa(&soa(7200, 3600, 1_209_600)).unwrap();
    let now = 100_000;
    assert_eq!(t.next_action(now, now - 8000, now - 1000), RefreshAction::Wait(3600 - 1000));
}
#[test]
fn expires_when_expire_elapsed() {
    let t = SoaTimers::from_soa(&soa(7200, 3600, 1_209_600)).unwrap();
    let now = 5_000_000;
    assert_eq!(t.next_action(now, now - 1_209_600, now - 1000), RefreshAction::Expired);
}
#[test]
fn serial_newer_basic_and_rfc1982_wraparound() {
    assert!(serial_newer(1, 2));
    assert!(!serial_newer(2, 1));
    assert!(!serial_newer(5, 5));
    assert!(serial_newer(u32::MAX, 0));
    assert!(!serial_newer(0, u32::MAX));
}
