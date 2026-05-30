// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral-parity tests for Cilium's EDT (Earliest Departure Time)
//! bandwidth scheduler — `bpf/lib/edt.h` `edt_sched_departure`
//! (pinned cilium/cilium v1.19.3, Apache-2.0).
//!
//! These probe the observable departure-time state machine: the fast
//! "already past" admission, the throttle path that pushes the packet's
//! departure timestamp forward, and the drop-horizon overflow.

use cave_net::ebpf_sim::{
    edt_sched_departure, EdtInfo, EdtThrottleMap, EdtVerdict, DEFAULT_DROP_HORIZON_NS, NSEC_PER_SEC,
};

// 1 Mbit/s expressed as bytes/sec (the throttle map stores bytes/sec —
// the agent's bandwidth manager divides the bit-rate annotation by 8).
const ONE_MBIT_BPS: u64 = 125_000;

#[test]
fn nsec_per_sec_constant_is_one_billion() {
    assert_eq!(NSEC_PER_SEC, 1_000_000_000);
}

#[test]
fn default_drop_horizon_is_two_seconds() {
    // bandwidth manager default horizon: 2 * NSEC_PER_SEC.
    assert_eq!(DEFAULT_DROP_HORIZON_NS, 2 * NSEC_PER_SEC);
}

#[test]
fn zero_bps_is_unthrottled_pass() {
    // bps == 0 means no rate configured → CTX_ACT_OK, tstamp untouched.
    let mut info = EdtInfo::new(0, DEFAULT_DROP_HORIZON_NS);
    let (verdict, tstamp) = edt_sched_departure(&mut info, 1500, 9 * NSEC_PER_SEC, 9 * NSEC_PER_SEC);
    assert_eq!(verdict, EdtVerdict::Pass);
    assert_eq!(tstamp, 9 * NSEC_PER_SEC);
    assert_eq!(info.t_last, 0);
}

#[test]
fn delay_formula_is_len_times_nsec_over_bps() {
    // delay = len * NSEC_PER_SEC / bps. 1500 bytes at 125000 B/s = 12 ms.
    // Construct a throttle so t_next lands exactly t_last + delay.
    let t_last = 10 * NSEC_PER_SEC;
    let mut info = EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, t_last);
    let now = 9 * NSEC_PER_SEC;
    let (verdict, tstamp) = edt_sched_departure(&mut info, 1500, now, now);
    let expected_delay = 1500u64 * NSEC_PER_SEC / ONE_MBIT_BPS; // 12_000_000
    assert_eq!(expected_delay, 12_000_000);
    assert_eq!(verdict, EdtVerdict::Pass);
    // throttled: departure pushed to t_last + delay, t_last advanced.
    assert_eq!(tstamp, t_last + expected_delay);
    assert_eq!(info.t_last, t_last + expected_delay);
}

#[test]
fn past_departure_is_fast_pass_and_clamps_t_last_to_now() {
    // t_last in the distant past → t_next <= t (now) → fast OK.
    // Upstream sets t_last = t (clamped to now) and leaves tstamp alone.
    let mut info = EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, 1000);
    let now = 10 * NSEC_PER_SEC;
    let orig_tstamp = 0; // in the past; gets clamped internally to `now`
    let (verdict, tstamp) = edt_sched_departure(&mut info, 1500, now, orig_tstamp);
    assert_eq!(verdict, EdtVerdict::Pass);
    // fast branch does NOT modify the packet's tstamp.
    assert_eq!(tstamp, orig_tstamp);
    // t_last clamped up to `now`.
    assert_eq!(info.t_last, now);
}

#[test]
fn throttle_within_horizon_pushes_departure_forward() {
    let t_last = 10 * NSEC_PER_SEC;
    let mut info = EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, t_last);
    let now = 9 * NSEC_PER_SEC;
    let (verdict, tstamp) = edt_sched_departure(&mut info, 1500, now, now);
    // t_next = 10.012s; t_next - now = 1.012s < 2s horizon → admit, delay.
    assert_eq!(verdict, EdtVerdict::Pass);
    assert_eq!(tstamp, 10 * NSEC_PER_SEC + 12_000_000);
    assert_eq!(info.t_last, 10 * NSEC_PER_SEC + 12_000_000);
}

#[test]
fn beyond_horizon_drops_and_leaves_t_last_untouched() {
    // t_last far enough ahead that t_next - now >= horizon → CTX_ACT_DROP.
    let t_last = 12 * NSEC_PER_SEC;
    let mut info = EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, t_last);
    let now = 9 * NSEC_PER_SEC;
    let orig_tstamp = 9 * NSEC_PER_SEC;
    let (verdict, tstamp) = edt_sched_departure(&mut info, 1500, now, orig_tstamp);
    // t_next = 12.012s; t_next - now = 3.012s >= 2s horizon → drop.
    assert_eq!(verdict, EdtVerdict::Drop);
    // drop leaves both t_last and tstamp untouched.
    assert_eq!(info.t_last, t_last);
    assert_eq!(tstamp, orig_tstamp);
}

#[test]
fn horizon_boundary_is_inclusive_drop() {
    // t_next - now == horizon exactly → drop (upstream uses `>=`).
    // Pick bps/len so delay is exact, and t_last so t_next-now == horizon.
    // delay = 12_000_000. Want t_next - now = 2_000_000_000.
    // t_next = t_last + delay; choose now and t_last accordingly.
    let now = 5 * NSEC_PER_SEC;
    let delay = 12_000_000u64;
    // t_next = now + horizon  => t_last = now + horizon - delay
    let t_last = now + DEFAULT_DROP_HORIZON_NS - delay;
    let mut info = EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, t_last);
    let (verdict, _) = edt_sched_departure(&mut info, 1500, now, now);
    assert_eq!(verdict, EdtVerdict::Drop);
}

#[test]
fn tstamp_in_future_is_not_clamped_down() {
    // When the packet already carries a future tstamp > now, `t` keeps it
    // (upstream `if (t < now) t = now;` only raises, never lowers).
    let t_last = 1000u64;
    let mut info = EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, t_last);
    let now = 1 * NSEC_PER_SEC;
    let future_tstamp = 20 * NSEC_PER_SEC; // ahead of now
    let (verdict, tstamp) = edt_sched_departure(&mut info, 1500, now, future_tstamp);
    // t = 20s, t_next = 1000 + 12ms << 20s → fast pass, t_last = t = 20s.
    assert_eq!(verdict, EdtVerdict::Pass);
    assert_eq!(tstamp, future_tstamp);
    assert_eq!(info.t_last, future_tstamp);
}

// ---- ThrottleMap routing (mirrors the full edt_sched_departure flow) ----

#[test]
fn throttle_map_unknown_aggregate_passes_unmodified() {
    let mut map = EdtThrottleMap::new();
    // aggregate id 0 means "no aggregate" → CTX_ACT_OK.
    let (v0, t0) = map.schedule(0, 1500, 9 * NSEC_PER_SEC, 9 * NSEC_PER_SEC);
    assert_eq!(v0, EdtVerdict::Pass);
    assert_eq!(t0, 9 * NSEC_PER_SEC);
    // unknown (non-zero) id with no map entry → CTX_ACT_OK.
    let (v1, t1) = map.schedule(42, 1500, 9 * NSEC_PER_SEC, 9 * NSEC_PER_SEC);
    assert_eq!(v1, EdtVerdict::Pass);
    assert_eq!(t1, 9 * NSEC_PER_SEC);
}

#[test]
fn throttle_map_applies_and_persists_t_last() {
    let mut map = EdtThrottleMap::new();
    map.insert(7, EdtInfo::with_t_last(ONE_MBIT_BPS, DEFAULT_DROP_HORIZON_NS, 10 * NSEC_PER_SEC));
    let now = 9 * NSEC_PER_SEC;
    let (v, tstamp) = map.schedule(7, 1500, now, now);
    assert_eq!(v, EdtVerdict::Pass);
    assert_eq!(tstamp, 10 * NSEC_PER_SEC + 12_000_000);
    // The WRITE_ONCE(info->t_last, ...) must persist into the map entry.
    assert_eq!(map.get(7).unwrap().t_last, 10 * NSEC_PER_SEC + 12_000_000);
}
