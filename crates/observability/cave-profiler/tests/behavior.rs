// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for cave-profiler.
//!
//! These cover the profile-aggregation surface that ports upstream
//! Grafana Pyroscope v1.3.0 behaviors (sample-rate computation, session
//! duration, top-N self-weight reduction, hotspot selection, and the
//! profile-type / session model serde contract). They exercise only the
//! public `cave_profiler::{engine, models}` API and assert concrete
//! expected values derived from the implementation logic in
//! `src/engine.rs` and `src/models.rs`.

use cave_profiler::engine;
use cave_profiler::models::{ProfileSession, ProfileType, StackFrame};
use chrono::{Duration, Utc};
use uuid::Uuid;

fn make_frame(function: &str, self_samples: u64) -> StackFrame {
    StackFrame {
        function: function.to_string(),
        file: "main.rs".to_string(),
        line: 7,
        self_samples,
        cumulative_samples: self_samples + 100,
    }
}

fn make_session(samples: u64, duration_secs: Option<i64>) -> ProfileSession {
    let started_at = Utc::now();
    let ended_at = duration_secs.map(|d| started_at + Duration::seconds(d));
    ProfileSession {
        id: Uuid::new_v4(),
        service: "api".to_string(),
        profile_type: ProfileType::Cpu,
        started_at,
        ended_at,
        samples,
        frames: vec![],
    }
}

// ---- engine::samples_per_second ------------------------------------------

/// rate = samples / duration. 1000 samples over a 10s completed window = 100.0.
#[test]
fn test_samples_per_second_basic() {
    let session = make_session(1000, Some(10));
    assert_eq!(engine::samples_per_second(&session), Some(100.0));
}

/// Divide-by-zero guard: a completed session whose duration is 0s must yield
/// Some(0.0) (engine.rs:21 branch) rather than NaN/inf or a panic.
#[test]
fn test_samples_per_second_zero_duration() {
    let session = make_session(500, Some(0));
    assert_eq!(engine::samples_per_second(&session), Some(0.0));
}

/// A still-running session (ended_at == None) has no duration, so the rate is
/// None — the map over session_duration_secs short-circuits.
#[test]
fn test_samples_per_second_none_for_running() {
    let session = make_session(1000, None);
    assert_eq!(engine::samples_per_second(&session), None);
}

// ---- engine::session_duration_secs ---------------------------------------

/// Completed-session (ended_at = Some) branch: ended_at = started + 30s yields
/// Some(30). The existing unit suite only covers the running (None) branch.
#[test]
fn test_session_duration_secs_completed() {
    let session = make_session(0, Some(30));
    assert_eq!(engine::session_duration_secs(&session), Some(30));
}

// ---- engine::top_functions -----------------------------------------------

/// Equal self_samples weights: sort_by is stable, so the truncated top-N
/// preserves insertion order. With three equal-weight frames and n=2, the
/// first two inserted ("a", "b") are returned in order.
#[test]
fn test_top_functions_equal_weight_stable() {
    let frames = vec![
        make_frame("a", 42),
        make_frame("b", 42),
        make_frame("c", 42),
    ];
    let top = engine::top_functions(&frames, 2);
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].function, "a");
    assert_eq!(top[1].function, "b");
}

/// n == 0 truncates to an empty result without panicking.
#[test]
fn test_top_functions_zero_n_is_empty() {
    let frames = vec![make_frame("a", 10), make_frame("b", 20)];
    let top = engine::top_functions(&frames, 0);
    assert!(top.is_empty());
}

// ---- engine::find_hotspot ------------------------------------------------

/// On a tie for max self_samples, Iterator::max_by_key returns the LAST of the
/// equally-maximum elements. Frames "x" and "z" both have 200; the hotspot is
/// "z" (the later one).
#[test]
fn test_find_hotspot_tie_returns_last() {
    let frames = vec![
        make_frame("x", 200),
        make_frame("y", 50),
        make_frame("z", 200),
    ];
    let hotspot = engine::find_hotspot(&frames).expect("non-empty frames");
    assert_eq!(hotspot.function, "z");
}

// ---- models::ProfileType serde -------------------------------------------

/// ProfileType uses #[serde(rename_all = "snake_case")]: each variant
/// serializes to its lowercase token and round-trips back to the same variant.
#[test]
fn test_profile_type_serde_roundtrip() {
    let cases = [
        (ProfileType::Cpu, "\"cpu\""),
        (ProfileType::Memory, "\"memory\""),
        (ProfileType::Goroutine, "\"goroutine\""),
        (ProfileType::Mutex, "\"mutex\""),
        (ProfileType::Block, "\"block\""),
    ];
    for (variant, expected_json) in cases {
        let encoded = serde_json::to_string(&variant).unwrap();
        assert_eq!(encoded, expected_json);
        let decoded: ProfileType = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, variant);
    }
}

// ---- models::ProfileSession serde ----------------------------------------

/// A populated ProfileSession (with frames + a completed window) survives a
/// serde_json round-trip intact, exercising the PartialEq + Serialize +
/// Deserialize derives on ProfileSession and its nested StackFrame.
#[test]
fn test_profile_session_serde_roundtrip() {
    let mut session = make_session(1234, Some(15));
    session.profile_type = ProfileType::Goroutine;
    session.frames = vec![make_frame("hot", 90), make_frame("cold", 3)];

    let encoded = serde_json::to_string(&session).unwrap();
    let decoded: ProfileSession = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, session);
    assert_eq!(decoded.profile_type, ProfileType::Goroutine);
    assert_eq!(decoded.frames.len(), 2);
    assert_eq!(decoded.frames[0].function, "hot");
    assert_eq!(decoded.samples, 1234);
}
