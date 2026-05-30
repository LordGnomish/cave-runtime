// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Standard cron schedule parsing + next-activation — port of the parts of
//! `github.com/robfig/cron/v3` that Karpenter's `Budget.IsActive`
//! (`pkg/apis/v1/nodepool.go`, v1.12.1) depends on: `ParseStandard` and
//! `SpecSchedule.Next`. MIT-licensed upstream (robfig/cron); see NOTICE.
//!
//! Standard cron is five space-separated fields — minute, hour,
//! day-of-month, month, day-of-week — each supporting `*`, ranges (`1-5`),
//! steps (`*/15`, `0-30/10`), lists (`1,15,31`), and names (`JAN`–`DEC`,
//! `SUN`–`SAT`). The seconds field is implicit and fixed at `0`, exactly as
//! robfig's standard parser sets it. An optional leading `TZ=…` /
//! `CRON_TZ=…` token is accepted and ignored — Karpenter always evaluates in
//! UTC (`"TZ=UTC <schedule>"`), which is the only timezone this port models.
//!
//! Times are plain `i64` Unix seconds (UTC). The crate stays dependency-free,
//! so the civil-calendar conversions (`days_from_civil` / `civil_from_days`,
//! after Howard Hinnant's `chrono`-algorithms) are implemented inline rather
//! than pulling a date library.

use std::fmt;

/// Bit 63 marks a field that was specified as `*` (or `?`). robfig uses it so
/// the day-of-month / day-of-week intersection can tell "explicitly any" from
/// "a range that happens to cover everything".
const STAR_BIT: u64 = 1 << 63;

/// A parsed standard cron schedule: one `u64` bitmask per field (bit `n` set
/// means value `n` matches), plus the `*`-flags for the day fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    second: u64,
    minute: u64,
    hour: u64,
    dom: u64,
    month: u64,
    dow: u64,
}

/// Failure parsing a cron expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronError {
    /// The expression did not have exactly five fields.
    WrongFieldCount { got: usize, expr: String },
    /// A field value was out of its allowed range or otherwise malformed.
    BadField { field: String, reason: String },
}

impl fmt::Display for CronError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CronError::WrongFieldCount { got, expr } => write!(
                f,
                "expected exactly 5 fields, found {got}: {expr:?}"
            ),
            CronError::BadField { field, reason } => {
                write!(f, "failed to parse field {field:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for CronError {}

struct Bounds {
    min: u64,
    max: u64,
    names: &'static [&'static str],
}

const MINUTES: Bounds = Bounds { min: 0, max: 59, names: &[] };
const HOURS: Bounds = Bounds { min: 0, max: 23, names: &[] };
const DOM: Bounds = Bounds { min: 1, max: 31, names: &[] };
const MONTHS: Bounds = Bounds {
    min: 1,
    max: 12,
    names: &[
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ],
};
const DOW: Bounds = Bounds {
    min: 0,
    max: 6,
    names: &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
};

/// `cron.ParseStandard`: parse a 5-field standard cron expression (UTC).
pub fn parse_standard(spec: &str) -> Result<CronSchedule, CronError> {
    let spec = strip_tz(spec.trim());
    let fields: Vec<&str> = spec.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(CronError::WrongFieldCount {
            got: fields.len(),
            expr: spec.to_string(),
        });
    }
    let minute = get_field(fields[0], &MINUTES)?;
    let hour = get_field(fields[1], &HOURS)?;
    let dom = get_field(fields[2], &DOM)?;
    let month = get_field(fields[3], &MONTHS)?;
    let dow = get_field(fields[4], &DOW)?;
    Ok(CronSchedule {
        second: 1 << 0, // standard parser fixes seconds at 0
        minute,
        hour,
        dom,
        month,
        dow,
    })
}

/// Drop a leading `TZ=…` or `CRON_TZ=…` token. UTC is assumed regardless.
fn strip_tz(spec: &str) -> &str {
    if let Some(rest) = spec.strip_prefix("TZ=").or_else(|| spec.strip_prefix("CRON_TZ=")) {
        // skip to the first whitespace after the TZ token
        match rest.find(char::is_whitespace) {
            Some(i) => rest[i..].trim_start(),
            None => "",
        }
    } else {
        spec
    }
}

/// `getField`: OR together the comma-separated ranges of one field.
fn get_field(field: &str, bounds: &Bounds) -> Result<u64, CronError> {
    let mut bits = 0u64;
    for expr in field.split(',') {
        bits |= get_range(expr, bounds, field)?;
    }
    Ok(bits)
}

/// `getRange`: parse `lo[-hi][/step]` or `*[/step]` into a bitmask, setting the
/// star bit for a bare `*` (or `?`).
fn get_range(expr: &str, bounds: &Bounds, field: &str) -> Result<u64, CronError> {
    let bad = |reason: String| CronError::BadField {
        field: field.to_string(),
        reason,
    };

    let range_and_step: Vec<&str> = expr.split('/').collect();
    if range_and_step.len() > 2 {
        return Err(bad(format!("too many slashes: {expr:?}")));
    }
    let low_and_high: Vec<&str> = range_and_step[0].split('-').collect();
    if low_and_high.len() > 2 {
        return Err(bad(format!("too many hyphens: {expr:?}")));
    }
    let single_digit = low_and_high.len() == 1;

    let (start, mut end);
    let mut extra = 0u64;
    if low_and_high[0] == "*" || low_and_high[0] == "?" {
        start = bounds.min;
        end = bounds.max;
        extra = STAR_BIT;
    } else {
        start = parse_int_or_name(low_and_high[0], bounds, field)?;
        end = match low_and_high.len() {
            1 => start,
            _ => parse_int_or_name(low_and_high[1], bounds, field)?,
        };
    }

    let step: u64 = match range_and_step.len() {
        1 => 1,
        _ => {
            let s: u64 = range_and_step[1]
                .parse()
                .map_err(|_| bad(format!("bad step: {:?}", range_and_step[1])))?;
            if s == 0 {
                return Err(bad("step of range should be a positive number".to_string()));
            }
            // "N/step" (single value before slash) means "N-max/step".
            if single_digit {
                end = bounds.max;
            }
            if s > 1 {
                extra = 0;
            }
            s
        }
    };

    if start < bounds.min {
        return Err(bad(format!(
            "value {start} below minimum {}",
            bounds.min
        )));
    }
    if end > bounds.max {
        return Err(bad(format!("value {end} above maximum {}", bounds.max)));
    }
    if start > end {
        return Err(bad(format!("beginning of range ({start}) beyond end ({end})")));
    }

    Ok(get_bits(start, end, step) | extra)
}

/// `getBits`: contiguous (step 1) or strided bitmask over `[min, max]`.
fn get_bits(min: u64, max: u64, step: u64) -> u64 {
    if step == 1 {
        // all bits in [min, max]
        let upper = if max >= 63 {
            u64::MAX
        } else {
            !(u64::MAX << (max + 1))
        };
        return upper & (u64::MAX << min);
    }
    let mut bits = 0u64;
    let mut i = min;
    while i <= max {
        bits |= 1 << i;
        i += step;
    }
    bits
}

/// `parseIntOrName`: a decimal number or a 3-letter month/weekday name.
fn parse_int_or_name(token: &str, bounds: &Bounds, field: &str) -> Result<u64, CronError> {
    if !bounds.names.is_empty() {
        let lower = token.to_ascii_lowercase();
        if let Some(idx) = bounds.names.iter().position(|n| *n == lower) {
            return Ok(idx as u64 + bounds.min);
        }
        // also accept names that are not the token (fall through to numeric)
        if token.chars().any(|c| c.is_ascii_alphabetic()) {
            return Err(CronError::BadField {
                field: field.to_string(),
                reason: format!("unrecognized name {token:?}"),
            });
        }
    }
    token.parse::<u64>().map_err(|_| CronError::BadField {
        field: field.to_string(),
        reason: format!("invalid value {token:?}"),
    })
}

impl CronSchedule {
    fn dom_is_star(&self) -> bool {
        self.dom & STAR_BIT != 0
    }

    fn dow_is_star(&self) -> bool {
        self.dow & STAR_BIT != 0
    }

    /// `dayMatches`: day-of-month AND day-of-week when either is `*`, else OR
    /// — robfig's intersection rule.
    fn day_matches(&self, t: &Civil) -> bool {
        let dom_match = self.dom & (1 << t.day) != 0;
        let dow_match = self.dow & (1 << t.weekday()) != 0;
        if self.dom_is_star() || self.dow_is_star() {
            dom_match && dow_match
        } else {
            dom_match || dow_match
        }
    }

    /// `SpecSchedule.Next`: the earliest activation strictly after `after_unix`
    /// (Unix seconds, UTC), or `None` if none occurs within five years.
    pub fn next(&self, after_unix: i64) -> Option<i64> {
        // Start at the upcoming whole second (strictly greater than input).
        let mut t = Civil::from_unix(after_unix + 1);
        let year_limit = t.year + 5;

        'wrap: loop {
            if t.year > year_limit {
                return None;
            }
            // Month.
            while self.month & (1 << t.month) == 0 {
                t = Civil {
                    year: t.year,
                    month: t.month,
                    day: 1,
                    hour: 0,
                    minute: 0,
                    second: 0,
                };
                t.add_month();
                if t.month == 1 {
                    continue 'wrap;
                }
            }
            // Day (dom/dow intersection).
            while !self.day_matches(&t) {
                t.hour = 0;
                t.minute = 0;
                t.second = 0;
                t.add_day();
                if t.day == 1 {
                    continue 'wrap;
                }
            }
            // Hour.
            while self.hour & (1 << t.hour) == 0 {
                t.minute = 0;
                t.second = 0;
                t.add_hour();
                if t.hour == 0 {
                    continue 'wrap;
                }
            }
            // Minute.
            while self.minute & (1 << t.minute) == 0 {
                t.second = 0;
                t.add_minute();
                if t.minute == 0 {
                    continue 'wrap;
                }
            }
            // Second.
            while self.second & (1 << t.second) == 0 {
                t.add_second();
                if t.second == 0 {
                    continue 'wrap;
                }
            }
            return Some(t.to_unix());
        }
    }
}

// ── Civil time (UTC), self-contained ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Civil {
    year: i64,
    month: u64, // 1..=12
    day: u64,   // 1..=31
    hour: u64,  // 0..=23
    minute: u64,
    second: u64,
}

impl Civil {
    fn from_unix(secs: i64) -> Civil {
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Civil {
            year,
            month,
            day,
            hour: (sod / 3600) as u64,
            minute: ((sod % 3600) / 60) as u64,
            second: (sod % 60) as u64,
        }
    }

    fn to_unix(&self) -> i64 {
        let days = days_from_civil(self.year, self.month, self.day);
        days * 86_400 + self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64
    }

    /// 0 = Sunday … 6 = Saturday (matching Go's `time.Weekday`).
    fn weekday(&self) -> u64 {
        let z = days_from_civil(self.year, self.month, self.day);
        // Hinnant: weekday_from_days
        (z.rem_euclid(7) + 4).rem_euclid(7) as u64
    }

    fn add_month(&mut self) {
        self.month += 1;
        if self.month > 12 {
            self.month = 1;
            self.year += 1;
        }
    }

    fn add_day(&mut self) {
        self.day += 1;
        if self.day > days_in_month(self.year, self.month) {
            self.day = 1;
            self.add_month();
        }
    }

    fn add_hour(&mut self) {
        self.hour += 1;
        if self.hour > 23 {
            self.hour = 0;
            self.add_day();
        }
    }

    fn add_minute(&mut self) {
        self.minute += 1;
        if self.minute > 59 {
            self.minute = 0;
            self.add_hour();
        }
    }

    fn add_second(&mut self) {
        self.second += 1;
        if self.second > 59 {
            self.second = 0;
            self.add_minute();
        }
    }
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: u64, d: u64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64; // [0, 399]
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`] (Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u64, u64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u64, d as u64)
}
