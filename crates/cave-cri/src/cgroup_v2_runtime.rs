//! Cgroup v2 runtime knobs — freezer, cgroup.kill, PSI, memory.events.
//!
//! Builds on `crate::cgroup_v2` (limits + devices BPF). New surfaces:
//!
//! - `cgroup.freeze` — write `1`/`0` to suspend/resume the entire
//!   cgroup tree. cave uses this in preference to SIGSTOP/SIGCONT
//!   on the sandbox's pause container so the freeze is applied
//!   atomically across all tasks.
//! - `cgroup.kill` — write `1` to deliver SIGKILL to every process
//!   in the cgroup in a single syscall (Linux 5.14+, kernel
//!   `kernel/cgroup/cgroup.c::cgroup_kill_show`).
//! - **Pressure Stall Information (PSI)** — parse the
//!   `cpu.pressure` / `memory.pressure` / `io.pressure` files into
//!   `PsiStats { some, full }` records.
//! - `memory.events` — `low/high/max/oom/oom_kill` event counters
//!   parsed into a typed struct.
//! - `memory.swap.events` — `high/max/fail` swap counters.
//!
//! Upstream:
//! - kernel docs: <https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html#core-interface-files>
//! - runc:        `libcontainer/cgroups/fs2/freezer.go`,
//!                `libcontainer/cgroups/fs2/memory.go`

use crate::error::{CriError, CriResult};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ── Freezer ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreezerState {
    Thawed,
    Frozen,
}

impl FreezerState {
    pub fn from_str(s: &str) -> Option<FreezerState> {
        match s.trim() {
            "0" => Some(FreezerState::Thawed),
            "1" => Some(FreezerState::Frozen),
            _ => None,
        }
    }

    pub fn as_byte(self) -> &'static str {
        match self {
            FreezerState::Thawed => "0",
            FreezerState::Frozen => "1",
        }
    }
}

/// Freeze every task in `cgroup_dir` by writing `1` to `cgroup.freeze`.
pub fn freeze(cgroup_dir: &Path) -> CriResult<()> {
    write_atomic(&cgroup_dir.join("cgroup.freeze"), FreezerState::Frozen.as_byte())
}

/// Resume every task in `cgroup_dir`.
pub fn thaw(cgroup_dir: &Path) -> CriResult<()> {
    write_atomic(&cgroup_dir.join("cgroup.freeze"), FreezerState::Thawed.as_byte())
}

/// Read the current freezer state.
pub fn read_freezer_state(cgroup_dir: &Path) -> CriResult<FreezerState> {
    let raw = std::fs::read_to_string(cgroup_dir.join("cgroup.freeze")).map_err(CriError::Io)?;
    FreezerState::from_str(&raw)
        .ok_or_else(|| CriError::Cgroup(format!("unrecognised cgroup.freeze contents {:?}", raw)))
}

// ── cgroup.kill ──────────────────────────────────────────────────────────────

/// Deliver SIGKILL to every process in the cgroup atomically (Linux
/// 5.14+). Falls back to a clear error message on older kernels.
pub fn cgroup_kill(cgroup_dir: &Path) -> CriResult<()> {
    let target = cgroup_dir.join("cgroup.kill");
    if !target.exists() {
        return Err(CriError::Cgroup(
            "cgroup.kill not present — kernel < 5.14, fall back to per-pid SIGKILL".into(),
        ));
    }
    write_atomic(&target, "1")
}

// ── Pressure Stall Information (PSI) ─────────────────────────────────────────

/// One pressure axis (`some` or `full`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PsiAxis {
    pub avg10: f64,
    pub avg60: f64,
    pub avg300: f64,
    pub total_us: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PsiStats {
    pub some: PsiAxis,
    pub full: Option<PsiAxis>,
}

/// Parse the kernel's pressure file format:
/// ```text
/// some avg10=0.00 avg60=0.00 avg300=0.00 total=12345
/// full avg10=0.00 avg60=0.00 avg300=0.00 total=678
/// ```
pub fn parse_psi(content: &str) -> CriResult<PsiStats> {
    let mut some = None::<PsiAxis>;
    let mut full = None::<PsiAxis>;
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let axis = match parts.next() {
            Some(a) => a,
            None => continue,
        };
        let mut x = PsiAxis::default();
        for kv in parts {
            let (k, v) = match kv.split_once('=') {
                Some(p) => p,
                None => continue,
            };
            match k {
                "avg10"  => x.avg10  = v.parse().unwrap_or(0.0),
                "avg60"  => x.avg60  = v.parse().unwrap_or(0.0),
                "avg300" => x.avg300 = v.parse().unwrap_or(0.0),
                "total"  => x.total_us = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        match axis {
            "some" => some = Some(x),
            "full" => full = Some(x),
            _ => {}
        }
    }
    Ok(PsiStats {
        some: some.ok_or_else(|| CriError::Cgroup("missing 'some' line in pressure file".into()))?,
        full,
    })
}

/// Read `cpu.pressure` for `cgroup_dir`. CPU only ships the `some`
/// axis; `full` is `None`.
pub fn read_cpu_pressure(cgroup_dir: &Path) -> CriResult<PsiStats> {
    let raw = std::fs::read_to_string(cgroup_dir.join("cpu.pressure")).map_err(CriError::Io)?;
    parse_psi(&raw)
}

/// Read `memory.pressure`.
pub fn read_memory_pressure(cgroup_dir: &Path) -> CriResult<PsiStats> {
    let raw = std::fs::read_to_string(cgroup_dir.join("memory.pressure")).map_err(CriError::Io)?;
    parse_psi(&raw)
}

/// Read `io.pressure`.
pub fn read_io_pressure(cgroup_dir: &Path) -> CriResult<PsiStats> {
    let raw = std::fs::read_to_string(cgroup_dir.join("io.pressure")).map_err(CriError::Io)?;
    parse_psi(&raw)
}

// ── memory.events ────────────────────────────────────────────────────────────

/// Counters from `memory.events`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEvents {
    pub low: u64,
    pub high: u64,
    pub max: u64,
    pub oom: u64,
    pub oom_kill: u64,
    pub oom_group_kill: u64,
}

pub fn parse_memory_events(content: &str) -> MemoryEvents {
    let mut e = MemoryEvents::default();
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let key = match parts.next() {
            Some(k) => k,
            None => continue,
        };
        let value = parts.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        match key {
            "low"            => e.low = value,
            "high"           => e.high = value,
            "max"            => e.max = value,
            "oom"            => e.oom = value,
            "oom_kill"       => e.oom_kill = value,
            "oom_group_kill" => e.oom_group_kill = value,
            _ => {}
        }
    }
    e
}

pub fn read_memory_events(cgroup_dir: &Path) -> CriResult<MemoryEvents> {
    let raw = std::fs::read_to_string(cgroup_dir.join("memory.events")).map_err(CriError::Io)?;
    Ok(parse_memory_events(&raw))
}

// ── memory.swap.events ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySwapEvents {
    pub high: u64,
    pub max: u64,
    pub fail: u64,
}

pub fn parse_memory_swap_events(content: &str) -> MemorySwapEvents {
    let mut e = MemorySwapEvents::default();
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        let key = match parts.next() {
            Some(k) => k,
            None => continue,
        };
        let value = parts.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        match key {
            "high" => e.high = value,
            "max"  => e.max  = value,
            "fail" => e.fail = value,
            _ => {}
        }
    }
    e
}

pub fn read_memory_swap_events(cgroup_dir: &Path) -> CriResult<MemorySwapEvents> {
    let raw = std::fs::read_to_string(cgroup_dir.join("memory.swap.events")).map_err(CriError::Io)?;
    Ok(parse_memory_swap_events(&raw))
}

// ── shared writer ────────────────────────────────────────────────────────────

fn write_atomic(path: &Path, content: &str) -> CriResult<()> {
    std::fs::write(path, content).map_err(|e| {
        CriError::Cgroup(format!("write {} failed: {}", path.display(), e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── FreezerState ─────────────────────────────────────────────────────────

    #[test]
    fn freezer_state_from_str_known_values() {
        assert_eq!(FreezerState::from_str("0"), Some(FreezerState::Thawed));
        assert_eq!(FreezerState::from_str("1"), Some(FreezerState::Frozen));
        assert_eq!(FreezerState::from_str(" 1\n"), Some(FreezerState::Frozen));
        assert!(FreezerState::from_str("nope").is_none());
    }

    #[test]
    fn freezer_state_byte_round_trips() {
        for s in [FreezerState::Thawed, FreezerState::Frozen] {
            assert_eq!(FreezerState::from_str(s.as_byte()), Some(s));
        }
    }

    #[test]
    fn freeze_writes_one_to_cgroup_freeze() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.freeze"), "0").unwrap();
        freeze(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("cgroup.freeze")).unwrap(), "1");
    }

    #[test]
    fn thaw_writes_zero_to_cgroup_freeze() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.freeze"), "1").unwrap();
        thaw(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("cgroup.freeze")).unwrap(), "0");
    }

    #[test]
    fn read_freezer_state_returns_typed_value() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.freeze"), "1").unwrap();
        assert_eq!(read_freezer_state(dir.path()).unwrap(), FreezerState::Frozen);
    }

    #[test]
    fn read_freezer_state_invalid_contents_errors() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.freeze"), "garbage").unwrap();
        assert!(read_freezer_state(dir.path()).is_err());
    }

    // ── cgroup.kill ──────────────────────────────────────────────────────────

    #[test]
    fn cgroup_kill_writes_one_when_supported() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cgroup.kill"), "0").unwrap();
        cgroup_kill(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("cgroup.kill")).unwrap(), "1");
    }

    #[test]
    fn cgroup_kill_missing_file_errors() {
        let dir = tempdir().unwrap();
        let err = cgroup_kill(dir.path()).unwrap_err();
        assert!(err.to_string().contains("cgroup.kill not present"));
    }

    // ── PSI parser ───────────────────────────────────────────────────────────

    #[test]
    fn parse_psi_with_some_and_full() {
        let raw = "some avg10=0.10 avg60=0.20 avg300=0.30 total=1000\n\
                   full avg10=0.05 avg60=0.10 avg300=0.15 total=500\n";
        let p = parse_psi(raw).unwrap();
        assert_eq!(p.some.avg10, 0.10);
        assert_eq!(p.some.total_us, 1000);
        let full = p.full.unwrap();
        assert_eq!(full.avg60, 0.10);
        assert_eq!(full.total_us, 500);
    }

    #[test]
    fn parse_psi_some_only_no_full() {
        let raw = "some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";
        let p = parse_psi(raw).unwrap();
        assert!(p.full.is_none());
    }

    #[test]
    fn parse_psi_missing_some_errors() {
        let raw = "full avg10=0.0 avg60=0.0 avg300=0.0 total=0\n";
        assert!(parse_psi(raw).is_err());
    }

    #[test]
    fn parse_psi_ignores_unknown_keys() {
        let raw = "some avg10=0.1 mystery=999 total=42\n";
        let p = parse_psi(raw).unwrap();
        assert_eq!(p.some.avg10, 0.1);
        assert_eq!(p.some.total_us, 42);
    }

    #[test]
    fn read_cpu_pressure_typed() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("cpu.pressure"),
            "some avg10=1.2 avg60=2.3 avg300=3.4 total=12345\n").unwrap();
        let p = read_cpu_pressure(dir.path()).unwrap();
        assert_eq!(p.some.avg10, 1.2);
        assert!(p.full.is_none()); // CPU doesn't carry full
    }

    #[test]
    fn read_memory_pressure_handles_full_axis() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("memory.pressure"),
            "some avg10=0.5 avg60=0.5 avg300=0.5 total=100\n\
             full avg10=0.1 avg60=0.1 avg300=0.1 total=10\n").unwrap();
        let p = read_memory_pressure(dir.path()).unwrap();
        assert!(p.full.is_some());
        assert_eq!(p.full.unwrap().total_us, 10);
    }

    #[test]
    fn read_io_pressure_handles_full_axis() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("io.pressure"),
            "some avg10=0.0 avg60=0.0 avg300=0.0 total=0\n\
             full avg10=0.0 avg60=0.0 avg300=0.0 total=0\n").unwrap();
        let p = read_io_pressure(dir.path()).unwrap();
        assert!(p.full.is_some());
    }

    // ── memory.events parser ─────────────────────────────────────────────────

    #[test]
    fn parse_memory_events_extracts_all_known_fields() {
        let raw = "low 0\nhigh 5\nmax 1\noom 2\noom_kill 3\noom_group_kill 4\n";
        let e = parse_memory_events(raw);
        assert_eq!(e.low, 0);
        assert_eq!(e.high, 5);
        assert_eq!(e.max, 1);
        assert_eq!(e.oom, 2);
        assert_eq!(e.oom_kill, 3);
        assert_eq!(e.oom_group_kill, 4);
    }

    #[test]
    fn parse_memory_events_missing_fields_default_to_zero() {
        let e = parse_memory_events("max 10\n");
        assert_eq!(e.max, 10);
        assert_eq!(e.oom, 0);
    }

    #[test]
    fn parse_memory_events_handles_blank_input() {
        let e = parse_memory_events("");
        assert_eq!(e, MemoryEvents::default());
    }

    #[test]
    fn read_memory_events_from_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("memory.events"), "oom 7\noom_kill 7\n").unwrap();
        let e = read_memory_events(dir.path()).unwrap();
        assert_eq!(e.oom, 7);
        assert_eq!(e.oom_kill, 7);
    }

    // ── memory.swap.events parser ────────────────────────────────────────────

    #[test]
    fn parse_memory_swap_events_extracts_all_fields() {
        let raw = "high 1\nmax 2\nfail 3\n";
        let e = parse_memory_swap_events(raw);
        assert_eq!(e.high, 1);
        assert_eq!(e.max, 2);
        assert_eq!(e.fail, 3);
    }

    #[test]
    fn parse_memory_swap_events_missing_field_zero() {
        let e = parse_memory_swap_events("max 5\n");
        assert_eq!(e.max, 5);
        assert_eq!(e.fail, 0);
    }

    #[test]
    fn read_memory_swap_events_from_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("memory.swap.events"), "fail 11\n").unwrap();
        let e = read_memory_swap_events(dir.path()).unwrap();
        assert_eq!(e.fail, 11);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn psi_stats_roundtrip_through_json() {
        let raw = "some avg10=1.0 avg60=2.0 avg300=3.0 total=4\n\
                   full avg10=5.0 avg60=6.0 avg300=7.0 total=8\n";
        let p = parse_psi(raw).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let back: PsiStats = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn memory_events_roundtrip_through_json() {
        let e = MemoryEvents { low: 1, high: 2, max: 3, oom: 4, oom_kill: 5, oom_group_kill: 6 };
        let json = serde_json::to_string(&e).unwrap();
        let back: MemoryEvents = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
