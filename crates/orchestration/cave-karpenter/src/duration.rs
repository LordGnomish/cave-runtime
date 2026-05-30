// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! `NillableDuration` — port of `pkg/apis/v1/duration.go` from
//! kubernetes-sigs/karpenter v1.12.1 (sha ed490e8). Apache-2.0 upstream.
//!
//! Karpenter wraps `*time.Duration` so a field can be either a Go duration
//! string (`"30m"`, `"720h"`) or the sentinel `"Never"`, which disables the
//! field. It keeps the raw marshaled bytes so conversion webhooks don't make
//! GitOps tools see spurious drift (`30m` must remarshal as `30m`, not
//! `30m0s`).
//!
//! Upstream delegates to the Go standard library's `time.ParseDuration` and
//! `time.Duration.String()`. Those are ported here verbatim
//! ([`parse_duration`] / [`format_duration`]) so the sentinel wrapper, the
//! cron-budget `Duration` window (next ray), and any other duration field all
//! share identical Go semantics. Durations are represented as `i64`
//! nanoseconds, exactly as Go's `time.Duration`.

use std::fmt;

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

/// Sentinel string disabling a [`NillableDuration`].
pub const NEVER: &str = "Never";

const NANOSECOND: i64 = 1;
const MICROSECOND: i64 = 1_000 * NANOSECOND;
const MILLISECOND: i64 = 1_000 * MICROSECOND;
const SECOND: i64 = 1_000 * MILLISECOND;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;

/// Failure parsing a Go duration string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurationError {
    /// The string is not a valid Go duration (empty, no digits, bad chars).
    Invalid(String),
    /// A numeric run had no trailing unit.
    MissingUnit(String),
    /// A unit suffix was not one of ns/us/µs/ms/s/m/h.
    UnknownUnit { unit: String, input: String },
    /// The accumulated value overflows `i64` nanoseconds.
    Overflow(String),
}

impl fmt::Display for DurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DurationError::Invalid(s) => write!(f, "time: invalid duration {s:?}"),
            DurationError::MissingUnit(s) => write!(f, "time: missing unit in duration {s:?}"),
            DurationError::UnknownUnit { unit, input } => {
                write!(f, "time: unknown unit {unit:?} in duration {input:?}")
            }
            DurationError::Overflow(s) => write!(f, "time: invalid duration {s:?} (overflow)"),
        }
    }
}

impl std::error::Error for DurationError {}

/// `unitMap` — nanosecond value of each recognised unit suffix. `µs` is
/// accepted as the U+00B5 micro sign and the U+03BC Greek small mu, matching Go.
fn unit_value(unit: &str) -> Option<i64> {
    match unit {
        "ns" => Some(NANOSECOND),
        "us" | "\u{00b5}s" | "\u{03bc}s" => Some(MICROSECOND),
        "ms" => Some(MILLISECOND),
        "s" => Some(SECOND),
        "m" => Some(MINUTE),
        "h" => Some(HOUR),
        _ => None,
    }
}

/// `leadingInt` — consume the leading run of ASCII digits as an integer.
/// Returns `(value, rest, consumed_any)`. Overflow → `Err`.
fn leading_int<'a>(s: &'a str, orig: &str) -> Result<(i128, &'a str, bool), DurationError> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut x: i128 = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        x = x * 10 + (bytes[i] - b'0') as i128;
        if x > (i64::MAX as i128) {
            return Err(DurationError::Overflow(orig.to_string()));
        }
        i += 1;
    }
    Ok((x, &s[i..], i > 0))
}

/// `leadingFraction` — consume the leading run of digits as a fraction.
/// Returns `(value, scale, rest, consumed_any)` where the fraction equals
/// `value / scale`. Excess digits past the precision Go can represent are
/// dropped (Go stops accumulating but keeps consuming).
fn leading_fraction(s: &str) -> (i128, i128, &str, bool) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut x: i128 = 0;
    let mut scale: i128 = 1;
    let mut overflow = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        if !overflow {
            // Cap accumulation as Go does (its threshold is (1<<63-1)/10).
            if x > (i64::MAX as i128) / 10 {
                overflow = true;
            } else {
                let y = x * 10 + (bytes[i] - b'0') as i128;
                x = y;
                scale *= 10;
            }
        }
        i += 1;
    }
    (x, scale, &s[i..], i > 0)
}

/// `time.ParseDuration`: parse a signed sequence of decimal numbers, each with
/// an optional fraction and a required unit suffix (`"300ms"`, `"-1.5h"`,
/// `"2h45m"`). Returns the duration in `i64` nanoseconds.
pub fn parse_duration(s: &str) -> Result<i64, DurationError> {
    let orig = s;
    let mut neg = false;
    let mut rest = s;

    if let Some(&c) = rest.as_bytes().first() {
        if c == b'-' || c == b'+' {
            neg = c == b'-';
            rest = &rest[1..];
        }
    }
    // Special case: "0" with no unit is allowed.
    if rest == "0" {
        return Ok(0);
    }
    if rest.is_empty() {
        return Err(DurationError::Invalid(orig.to_string()));
    }

    let mut total: i128 = 0;
    while !rest.is_empty() {
        // The next character must be `[0-9.]`.
        let c0 = rest.as_bytes()[0];
        if c0 != b'.' && !c0.is_ascii_digit() {
            return Err(DurationError::Invalid(orig.to_string()));
        }

        // Consume `[0-9]*`.
        let (int_val, after_int, pre) = leading_int(rest, orig)?;
        rest = after_int;

        // Consume `(\.[0-9]*)?`.
        let mut frac: i128 = 0;
        let mut scale: i128 = 1;
        let mut post = false;
        if !rest.is_empty() && rest.as_bytes()[0] == b'.' {
            rest = &rest[1..];
            let (f, sc, after_frac, consumed) = leading_fraction(rest);
            frac = f;
            scale = sc;
            rest = after_frac;
            post = consumed;
        }
        if !pre && !post {
            // No digits (e.g. ".s" or "h").
            return Err(DurationError::Invalid(orig.to_string()));
        }

        // Consume the unit: the run up to the next `[0-9.]`.
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'.' || c.is_ascii_digit() {
                break;
            }
            i += 1;
        }
        if i == 0 {
            return Err(DurationError::MissingUnit(orig.to_string()));
        }
        let unit_str = &rest[..i];
        rest = &rest[i..];
        let unit = unit_value(unit_str).ok_or_else(|| DurationError::UnknownUnit {
            unit: unit_str.to_string(),
            input: orig.to_string(),
        })? as i128;

        let mut v = int_val
            .checked_mul(unit)
            .ok_or_else(|| DurationError::Overflow(orig.to_string()))?;
        if frac > 0 {
            // Go computes uint64(float64(frac) * (float64(unit) / scale)); the
            // integer form (frac*unit)/scale truncates identically for the
            // exact ratios real duration strings produce.
            v += (frac * unit) / scale;
        }
        total += v;
        if total > (i64::MAX as i128) {
            return Err(DurationError::Overflow(orig.to_string()));
        }
    }

    let result = if neg { -total } else { total };
    if result > i64::MAX as i128 || result < i64::MIN as i128 {
        return Err(DurationError::Overflow(orig.to_string()));
    }
    Ok(result as i64)
}

/// `fmtFrac` — emit the fraction digits of `v` at `prec` precision into `buf`
/// (building right-to-left), trimming trailing zeros and the `.` when zero.
/// Returns the remaining integer part of `v`.
fn fmt_frac(buf: &mut Vec<u8>, mut v: u64, prec: usize) -> u64 {
    let mut print = false;
    for _ in 0..prec {
        let digit = v % 10;
        print = print || digit != 0;
        if print {
            buf.push(b'0' + digit as u8);
        }
        v /= 10;
    }
    if print {
        buf.push(b'.');
    }
    v
}

/// `fmtInt` — emit `v` (at least one digit) into `buf`, right-to-left.
fn fmt_int(buf: &mut Vec<u8>, mut v: u64) {
    if v == 0 {
        buf.push(b'0');
        return;
    }
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
}

/// `time.Duration.String`: render `nanos` as Go does (`"1h30m0s"`, `"300ms"`,
/// `"-1.5h"` etc.). Sub-second durations use the largest fitting unit.
pub fn format_duration(nanos: i64) -> String {
    // Built right-to-left then reversed, mirroring Go's fixed buffer.
    let mut buf: Vec<u8> = Vec::with_capacity(32);
    let neg = nanos < 0;
    let mut u = (nanos as i128).unsigned_abs() as u64;

    if u < SECOND as u64 {
        // Sub-second: pick the unit, emit fraction + integer.
        let prec: usize;
        let unit_suffix: &[u8];
        if u == 0 {
            return "0s".to_string();
        } else if u < MICROSECOND as u64 {
            prec = 0;
            unit_suffix = b"ns";
        } else if u < MILLISECOND as u64 {
            prec = 3;
            unit_suffix = "µs".as_bytes(); // U+00B5 + 's'
        } else {
            prec = 6;
            unit_suffix = b"ms";
        }
        // suffix pushed reversed
        for &b in unit_suffix.iter().rev() {
            buf.push(b);
        }
        u = fmt_frac(&mut buf, u, prec);
        fmt_int(&mut buf, u);
    } else {
        buf.push(b's');
        u = fmt_frac(&mut buf, u, 9);
        // seconds
        fmt_int(&mut buf, u % 60);
        u /= 60;
        if u > 0 {
            buf.push(b'm');
            fmt_int(&mut buf, u % 60);
            u /= 60;
            if u > 0 {
                buf.push(b'h');
                fmt_int(&mut buf, u);
            }
        }
    }

    if neg {
        buf.push(b'-');
    }
    buf.reverse();
    String::from_utf8(buf).expect("ascii + µ bytes are valid utf8 when reversed by byte")
}

// ── NillableDuration ─────────────────────────────────────────────────────────

/// A `*time.Duration` that also accepts the sentinel `"Never"`. `None` nanos
/// means disabled. `raw` preserves the exact serialized form for drift-free
/// remarshaling, mirroring upstream's `Raw []byte`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NillableDuration {
    nanos: Option<i64>,
    raw: Option<String>,
}

impl NillableDuration {
    /// A disabled duration (`"Never"`).
    pub fn never() -> Self {
        NillableDuration {
            nanos: None,
            raw: None,
        }
    }

    /// Build directly from a nanosecond count (no preserved raw form, so it
    /// marshals via [`format_duration`]).
    pub fn from_nanos(nanos: i64) -> Self {
        NillableDuration {
            nanos: Some(nanos),
            raw: None,
        }
    }

    /// `UnmarshalJSON` semantics over the already-unquoted string: `"Never"`
    /// disables; anything else is parsed as a Go duration, preserving the raw
    /// form for remarshaling.
    pub fn parse(s: &str) -> Result<Self, DurationError> {
        if s == NEVER {
            return Ok(NillableDuration::never());
        }
        let nanos = parse_duration(s)?;
        Ok(NillableDuration {
            nanos: Some(nanos),
            raw: Some(s.to_string()),
        })
    }

    /// The duration in nanoseconds, or `None` when disabled.
    pub fn nanos(&self) -> Option<i64> {
        self.nanos
    }

    /// `true` when the duration is `"Never"` (disabled).
    pub fn is_never(&self) -> bool {
        self.nanos.is_none()
    }

    /// `MarshalJSON`/`ToUnstructured` value: the preserved raw form if any, else
    /// the `Duration.String()` form, else the `"Never"` sentinel.
    pub fn to_value_string(&self) -> String {
        if let Some(raw) = &self.raw {
            raw.clone()
        } else if let Some(n) = self.nanos {
            format_duration(n)
        } else {
            NEVER.to_string()
        }
    }
}

impl Serialize for NillableDuration {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_value_string())
    }
}

impl<'de> Deserialize<'de> for NillableDuration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        NillableDuration::parse(&s).map_err(serde::de::Error::custom)
    }
}
