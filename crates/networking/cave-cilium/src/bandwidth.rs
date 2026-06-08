// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bandwidth manager — EDT (Earliest Departure Time) egress pacing.
//!
//! Upstream reference (cilium v1.19.4):
//!   bpf/lib/edt.h        — `edt_sched_departure` (the datapath pacing decision)
//!   pkg/datapath/linux/bandwidth — the agent that populates the throttle map
//!
//! Cilium replaces classful qdiscs with an EDT model: each rate-limited
//! aggregate gets an [`EdtInfo`] in the `cilium_throttle` BPF map carrying its
//! `bps` limit and the last computed departure time `t_last`. For every egress
//! packet the datapath computes the packet's earliest departure timestamp from
//! the rate, stamps it onto the skb (`ctx->tstamp`) so FQ paces it, and drops
//! the packet if the departure lies beyond the configured horizon (back-pressure
//! rather than unbounded queueing).
//!
//! This is a faithful userspace port of `edt_sched_departure`: a pure function
//! over the aggregate's [`EdtInfo`] plus the per-packet `(now, tstamp, wire_len)`
//! triple, mutating `t_last` exactly as the BPF code does and returning the
//! datapath verdict.

/// Nanoseconds per second (`NSEC_PER_SEC` in bpf/include/bpf/time.h).
pub const NSEC_PER_SEC: u64 = 1_000_000_000;

/// Per-aggregate throttle state — the relevant fields of `struct edt_info`
/// (bpf/lib/edt.h). `t_horizon_drop` is the drop horizon the bandwidth manager
/// writes per aggregate (cilium defaults to 2 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdtInfo {
    /// Rate limit in bytes per second. `0` means "no limit configured".
    pub bps: u64,
    /// Last computed earliest-departure timestamp (ns), monotonic clock.
    pub t_last: u64,
    /// Drop horizon (ns): a packet whose departure is this far past `now` is
    /// dropped instead of queued.
    pub t_horizon_drop: u64,
    /// Scheduler priority (FQ band). Carried through unchanged here.
    pub prio: u32,
}

impl EdtInfo {
    /// A throttle entry with cilium's default 2-second drop horizon.
    pub fn new(bps: u64) -> Self {
        Self {
            bps,
            t_last: 0,
            t_horizon_drop: 2 * NSEC_PER_SEC,
            prio: 0,
        }
    }
}

/// The datapath verdict for one packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdtVerdict {
    /// Pass immediately — either no rate is configured, or the aggregate is at
    /// or below its rate so the packet needs no pacing (`CTX_ACT_OK`, no tstamp
    /// change). `t_last` advances to the packet's effective time.
    Pass,
    /// Paced: the skb's departure timestamp is set to `departure` (ns) so FQ
    /// holds it until then; `t_last` advances to the same value.
    Paced { departure: u64 },
    /// Dropped: the computed departure is at/beyond the drop horizon
    /// (`DROP_EDT_HORIZON`). `t_last` is intentionally left unchanged.
    Drop,
}

/// Port of `edt_sched_departure` (bpf/lib/edt.h).
///
/// `now` is the monotonic clock (`ktime_get_ns()`), `tstamp` is the skb's
/// current departure timestamp (`ctx->tstamp`), and `wire_len` is the on-wire
/// byte length (`ctx_wire_len`). On `Pass`/`Paced` the function updates
/// `info.t_last` exactly as the BPF code's `WRITE_ONCE(info->t_last, …)` does.
pub fn edt_sched_departure(
    info: &mut EdtInfo,
    now: u64,
    tstamp: u64,
    wire_len: u64,
) -> EdtVerdict {
    // `if (!info->bps) goto out;` — no rate configured.
    if info.bps == 0 {
        return EdtVerdict::Pass;
    }
    // `now = ktime_get_ns(); t = ctx->tstamp; if (t < now) t = now;`
    let t = tstamp.max(now);
    // `delay = wire_len * NSEC_PER_SEC / info->bps;`
    let delay = wire_len.saturating_mul(NSEC_PER_SEC) / info.bps;
    // `t_next = READ_ONCE(info->t_last) + delay;`
    let t_next = info.t_last.saturating_add(delay);
    // `if (t_next <= t) { WRITE_ONCE(info->t_last, t); return CTX_ACT_OK; }`
    if t_next <= t {
        info.t_last = t;
        return EdtVerdict::Pass;
    }
    // `if (t_next - now >= info->t_horizon_drop) return DROP_EDT_HORIZON;`
    if t_next - now >= info.t_horizon_drop {
        return EdtVerdict::Drop;
    }
    // `WRITE_ONCE(info->t_last, t_next); ctx->tstamp = t_next;`
    info.t_last = t_next;
    EdtVerdict::Paced { departure: t_next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_rate_configured_passes() {
        let mut info = EdtInfo::new(0);
        assert_eq!(edt_sched_departure(&mut info, 100, 100, 1500), EdtVerdict::Pass);
        // t_last untouched when bps==0.
        assert_eq!(info.t_last, 0);
    }

    #[test]
    fn first_packet_under_rate_passes_and_advances_t_last_to_now() {
        // t_last starts at 0, so t_next = delay; for the first packet t (>= now)
        // exceeds the delay, so it passes and t_last jumps to `t`.
        let mut info = EdtInfo::new(1_000_000); // 1 MB/s
        let now = 10 * NSEC_PER_SEC;
        let v = edt_sched_departure(&mut info, now, now, 1500);
        assert_eq!(v, EdtVerdict::Pass);
        assert_eq!(info.t_last, now);
    }

    #[test]
    fn clamps_stale_tstamp_up_to_now() {
        // A tstamp in the past must be clamped to `now` before comparison.
        let mut info = EdtInfo::new(1_000_000);
        info.t_last = 0;
        let now = 5 * NSEC_PER_SEC;
        let v = edt_sched_departure(&mut info, now, /*tstamp in the past*/ 1, 1500);
        assert_eq!(v, EdtVerdict::Pass);
        assert_eq!(info.t_last, now); // == now, not the stale tstamp
    }

    #[test]
    fn back_to_back_packets_get_paced_to_future_departure() {
        // 1500 bytes at 1 MB/s => delay = 1500 * 1e9 / 1e6 = 1_500_000 ns.
        let mut info = EdtInfo::new(1_000_000);
        let now = 0;
        // First packet at now=0, tstamp=0: t_next(=1.5ms) > t(=0)? yes -> paced.
        let v1 = edt_sched_departure(&mut info, now, 0, 1500);
        assert_eq!(v1, EdtVerdict::Paced { departure: 1_500_000 });
        assert_eq!(info.t_last, 1_500_000);
        // Second back-to-back packet (still now=0): departure stacks by delay.
        let v2 = edt_sched_departure(&mut info, now, 0, 1500);
        assert_eq!(v2, EdtVerdict::Paced { departure: 3_000_000 });
        assert_eq!(info.t_last, 3_000_000);
    }

    #[test]
    fn departure_beyond_horizon_is_dropped_without_touching_t_last() {
        // Tiny rate so a single packet's delay blows past the 2s horizon.
        let mut info = EdtInfo::new(1); // 1 byte/sec
        info.t_last = 0;
        let now = 0;
        // delay = 1500 * 1e9 / 1 = 1.5e12 ns = 1500 s >> 2 s horizon.
        let v = edt_sched_departure(&mut info, now, 0, 1500);
        assert_eq!(v, EdtVerdict::Drop);
        // Dropped packets must NOT advance t_last (upstream returns before WRITE).
        assert_eq!(info.t_last, 0);
    }

    #[test]
    fn stream_passes_once_real_time_catches_up_to_t_last() {
        let mut info = EdtInfo::new(1_000_000);
        let delay = 1_500_000u64;
        // First packet (t_last==0) is paced to its departure.
        assert_eq!(
            edt_sched_departure(&mut info, 0, 0, 1500),
            EdtVerdict::Paced { departure: delay }
        );
        // Once real time has advanced well past t_last+delay, the aggregate is
        // back under rate: the packet passes and t_last snaps forward to `t`.
        let now = 10 * delay;
        assert_eq!(edt_sched_departure(&mut info, now, now, 1500), EdtVerdict::Pass);
        assert_eq!(info.t_last, now);
    }
}
