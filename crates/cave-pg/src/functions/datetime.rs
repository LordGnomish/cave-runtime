//! PostgreSQL date/time functions.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{Interval, PgValue};

pub fn now(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::TimestampTz(Utc::now()))
}

pub fn clock_timestamp(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::TimestampTz(Utc::now()))
}

pub fn timeofday(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text(Utc::now().format("%a %b %d %H:%M:%S.%6f %Y %Z").to_string()))
}

pub fn current_date(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Date(Utc::now().naive_utc().date()))
}

pub fn current_time(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Time(Utc::now().naive_utc().time()))
}

pub fn localtime(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Time(Utc::now().naive_utc().time()))
}

pub fn localtimestamp(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Timestamp(Utc::now().naive_utc()))
}

pub fn date_trunc(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let field = args[0].to_text().to_lowercase();
    match &args[1] {
        PgValue::Timestamp(ts) => {
            let result = trunc_naive_dt(*ts, &field)?;
            Ok(PgValue::Timestamp(result))
        }
        PgValue::TimestampTz(ts) => {
            let naive = trunc_naive_dt(ts.naive_utc(), &field)?;
            Ok(PgValue::TimestampTz(DateTime::from_naive_utc_and_offset(naive, Utc)))
        }
        PgValue::Date(d) => {
            let ts = d.and_hms_opt(0, 0, 0).unwrap();
            let result = trunc_naive_dt(ts, &field)?;
            Ok(PgValue::Timestamp(result))
        }
        PgValue::Interval(iv) => {
            // For intervals, truncate to the given field
            let result = match field.as_str() {
                "millennium" | "century" | "decade" | "year" => {
                    Interval { months: (iv.months / 12) * 12, days: 0, microseconds: 0 }
                }
                "month" | "quarter" => {
                    Interval { months: iv.months, days: 0, microseconds: 0 }
                }
                "day" => Interval { months: iv.months, days: iv.days, microseconds: 0 },
                "hour" => {
                    let us = iv.microseconds - (iv.microseconds % 3_600_000_000);
                    Interval { months: iv.months, days: iv.days, microseconds: us }
                }
                "minute" => {
                    let us = iv.microseconds - (iv.microseconds % 60_000_000);
                    Interval { months: iv.months, days: iv.days, microseconds: us }
                }
                "second" => {
                    let us = iv.microseconds - (iv.microseconds % 1_000_000);
                    Interval { months: iv.months, days: iv.days, microseconds: us }
                }
                "milliseconds" | "millisecond" | "ms" => {
                    let us = iv.microseconds - (iv.microseconds % 1_000);
                    Interval { months: iv.months, days: iv.days, microseconds: us }
                }
                _ => iv.clone(),
            };
            Ok(PgValue::Interval(result))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "date_trunc requires timestamp, timestamptz, or interval"))),
    }
}

fn trunc_naive_dt(ts: NaiveDateTime, field: &str) -> Result<NaiveDateTime> {
    Ok(match field {
        "microseconds" | "microsecond" | "us" => ts,
        "milliseconds" | "millisecond" | "ms" => {
            let us = ts.nanosecond() / 1000;
            let ms = (us / 1000) * 1000;
            ts.with_nanosecond(ms * 1000).unwrap_or(ts)
        }
        "second" => ts.with_nanosecond(0).unwrap_or(ts),
        "minute" => ts.with_second(0).unwrap_or(ts).with_nanosecond(0).unwrap_or(ts),
        "hour" => ts.with_minute(0).unwrap_or(ts).with_second(0).unwrap_or(ts).with_nanosecond(0).unwrap_or(ts),
        "day" => ts.date().and_hms_opt(0, 0, 0).unwrap(),
        "week" => {
            use chrono::Weekday;
            let d = ts.date();
            let dow = d.weekday().num_days_from_monday() as i64;
            (d - Duration::days(dow)).and_hms_opt(0, 0, 0).unwrap()
        }
        "month" => NaiveDate::from_ymd_opt(ts.year(), ts.month(), 1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        "quarter" => {
            let q = ((ts.month() - 1) / 3) * 3 + 1;
            NaiveDate::from_ymd_opt(ts.year(), q, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()
        }
        "year" => NaiveDate::from_ymd_opt(ts.year(), 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        "decade" => NaiveDate::from_ymd_opt((ts.year() / 10) * 10, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        "century" => NaiveDate::from_ymd_opt(((ts.year() - 1) / 100 + 1) * 100 - 99, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        "millennium" => NaiveDate::from_ymd_opt(((ts.year() - 1) / 1000 + 1) * 1000 - 999, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        other => return Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
            format!("unknown date_trunc field: {other}")))),
    })
}

pub fn date_part(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let field = args[0].to_text().to_lowercase();
    match &args[1] {
        PgValue::Timestamp(ts) => Ok(PgValue::Float8(extract_naive_dt(*ts, &field)?)),
        PgValue::TimestampTz(ts) => Ok(PgValue::Float8(extract_naive_dt(ts.naive_utc(), &field)?)),
        PgValue::Date(d) => Ok(PgValue::Float8(extract_naive_dt(d.and_hms_opt(0, 0, 0).unwrap(), &field)?)),
        PgValue::Time(t) => Ok(PgValue::Float8(extract_time(*t, &field)?)),
        PgValue::Interval(iv) => Ok(PgValue::Float8(extract_interval(iv, &field)?)),
        _ => Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "date_part requires temporal type"))),
    }
}

fn extract_naive_dt(ts: NaiveDateTime, field: &str) -> Result<f64> {
    Ok(match field {
        "epoch" => ts.and_utc().timestamp() as f64 + ts.nanosecond() as f64 / 1e9,
        "year" => ts.year() as f64,
        "month" => ts.month() as f64,
        "day" => ts.day() as f64,
        "hour" => ts.hour() as f64,
        "minute" => ts.minute() as f64,
        "second" => ts.second() as f64 + ts.nanosecond() as f64 / 1e9,
        "milliseconds" | "millisecond" => ts.second() as f64 * 1000.0 + ts.nanosecond() as f64 / 1e6,
        "microseconds" | "microsecond" => ts.second() as f64 * 1e6 + ts.nanosecond() as f64 / 1000.0,
        "quarter" => ((ts.month() - 1) / 3 + 1) as f64,
        "week" => ts.iso_week().week() as f64,
        "dow" => ts.weekday().num_days_from_sunday() as f64,
        "isodow" => ts.weekday().num_days_from_monday() as f64 + 1.0,
        "doy" => ts.ordinal() as f64,
        "century" => ((ts.year() as f64 - 1.0) / 100.0).floor() + 1.0,
        "decade" => (ts.year() as f64 / 10.0).floor(),
        "millennium" => ((ts.year() as f64 - 1.0) / 1000.0).floor() + 1.0,
        "timezone" | "timezone_hour" | "timezone_minute" => 0.0,
        other => return Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
            format!("unknown date_part field: {other}")))),
    })
}

fn extract_time(t: NaiveTime, field: &str) -> Result<f64> {
    Ok(match field {
        "hour" => t.hour() as f64,
        "minute" => t.minute() as f64,
        "second" => t.second() as f64 + t.nanosecond() as f64 / 1e9,
        "milliseconds" | "millisecond" => t.second() as f64 * 1000.0 + t.nanosecond() as f64 / 1e6,
        "microseconds" | "microsecond" => t.second() as f64 * 1e6 + t.nanosecond() as f64 / 1000.0,
        "epoch" => t.num_seconds_from_midnight() as f64 + t.nanosecond() as f64 / 1e9,
        other => return Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
            format!("unknown date_part field: {other}")))),
    })
}

fn extract_interval(iv: &Interval, field: &str) -> Result<f64> {
    Ok(match field {
        "epoch" => iv.months as f64 * 30.0 * 86400.0 + iv.days as f64 * 86400.0 + iv.microseconds as f64 / 1e6,
        "year" => (iv.months / 12) as f64,
        "month" => (iv.months % 12) as f64,
        "day" => iv.days as f64,
        "hour" => (iv.microseconds / 3_600_000_000) as f64,
        "minute" => ((iv.microseconds % 3_600_000_000) / 60_000_000) as f64,
        "second" => (iv.microseconds % 60_000_000) as f64 / 1e6,
        "milliseconds" | "millisecond" => (iv.microseconds % 60_000_000) as f64 / 1000.0,
        "microseconds" | "microsecond" => (iv.microseconds % 60_000_000) as f64,
        other => return Err(Error::Pg(PgError::error(SqlState::INVALID_PARAMETER_VALUE,
            format!("unknown date_part field: {other}")))),
    })
}

pub fn date_bin(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 { return Ok(PgValue::Null); }
    // Simplified: just truncate to nearest interval
    date_trunc(vec![args[0].clone(), args[1].clone()])
}

pub fn age(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let (end_ts, start_ts) = if args.len() == 1 {
        let now = Utc::now().naive_utc();
        match &args[0] {
            PgValue::Timestamp(ts) => (now, *ts),
            PgValue::TimestampTz(ts) => (now, ts.naive_utc()),
            PgValue::Date(d) => (now, d.and_hms_opt(0,0,0).unwrap()),
            _ => return Ok(PgValue::Null),
        }
    } else {
        let ts1 = match &args[0] {
            PgValue::Timestamp(ts) => *ts,
            PgValue::TimestampTz(ts) => ts.naive_utc(),
            PgValue::Date(d) => d.and_hms_opt(0,0,0).unwrap(),
            _ => return Ok(PgValue::Null),
        };
        let ts2 = match &args[1] {
            PgValue::Timestamp(ts) => *ts,
            PgValue::TimestampTz(ts) => ts.naive_utc(),
            PgValue::Date(d) => d.and_hms_opt(0,0,0).unwrap(),
            _ => return Ok(PgValue::Null),
        };
        (ts1, ts2)
    };

    let mut years = end_ts.year() - start_ts.year();
    let mut months = end_ts.month() as i32 - start_ts.month() as i32;
    let mut days = end_ts.day() as i32 - start_ts.day() as i32;

    if days < 0 {
        months -= 1;
        // Borrow days from previous month
        let prev = if end_ts.month() == 1 {
            NaiveDate::from_ymd_opt(end_ts.year() - 1, 12, 1)
        } else {
            NaiveDate::from_ymd_opt(end_ts.year(), end_ts.month() - 1, 1)
        };
        let days_in_prev = prev.map(|d| d.with_day(1)
            .map(|d1| (d1 + Duration::days(32)).with_day(1).unwrap() - d1)
            .map(|dur| dur.num_days())
            .unwrap_or(30)).unwrap_or(30);
        days += days_in_prev as i32;
    }
    if months < 0 {
        years -= 1;
        months += 12;
    }

    let total_months = years * 12 + months;
    let us = (end_ts.signed_duration_since(start_ts).num_microseconds().unwrap_or(0)
        % (86_400_000_000i64)) + 0;

    Ok(PgValue::Interval(Interval { months: total_months, days, microseconds: us }))
}

pub fn make_date(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 { return Ok(PgValue::Null); }
    let y = args[0].to_i64().unwrap_or(2000) as i32;
    let m = args[1].to_i64().unwrap_or(1) as u32;
    let d = args[2].to_i64().unwrap_or(1) as u32;
    NaiveDate::from_ymd_opt(y, m, d)
        .map(PgValue::Date)
        .ok_or_else(|| Error::Pg(PgError::error(SqlState::DATETIME_FIELD_OVERFLOW, "invalid date")))
}

pub fn make_time(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 { return Ok(PgValue::Null); }
    let h = args[0].to_i64().unwrap_or(0) as u32;
    let m = args[1].to_i64().unwrap_or(0) as u32;
    let s = args[2].to_f64().unwrap_or(0.0);
    let secs = s.floor() as u32;
    let ns = ((s - s.floor()) * 1e9) as u32;
    NaiveTime::from_hms_nano_opt(h, m, secs, ns)
        .map(PgValue::Time)
        .ok_or_else(|| Error::Pg(PgError::error(SqlState::DATETIME_FIELD_OVERFLOW, "invalid time")))
}

pub fn make_timestamp(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 6 { return Ok(PgValue::Null); }
    let y = args[0].to_i64().unwrap_or(2000) as i32;
    let mo = args[1].to_i64().unwrap_or(1) as u32;
    let d = args[2].to_i64().unwrap_or(1) as u32;
    let h = args[3].to_i64().unwrap_or(0) as u32;
    let mi = args[4].to_i64().unwrap_or(0) as u32;
    let s = args[5].to_f64().unwrap_or(0.0);
    let secs = s.floor() as u32;
    let ns = ((s - s.floor()) * 1e9) as u32;
    let date = NaiveDate::from_ymd_opt(y, mo, d)
        .ok_or_else(|| Error::Pg(PgError::error(SqlState::DATETIME_FIELD_OVERFLOW, "invalid date")))?;
    let time = NaiveTime::from_hms_nano_opt(h, mi, secs, ns)
        .ok_or_else(|| Error::Pg(PgError::error(SqlState::DATETIME_FIELD_OVERFLOW, "invalid time")))?;
    Ok(PgValue::Timestamp(NaiveDateTime::new(date, time)))
}

pub fn make_timestamptz(args: Vec<PgValue>) -> Result<PgValue> {
    make_timestamp(args).map(|v| match v {
        PgValue::Timestamp(ts) => PgValue::TimestampTz(DateTime::from_naive_utc_and_offset(ts, Utc)),
        other => other,
    })
}

pub fn make_interval(args: Vec<PgValue>) -> Result<PgValue> {
    // make_interval(years, months, weeks, days, hours, mins, secs)
    let years = args.get(0).and_then(|v| v.to_i64()).unwrap_or(0) as i32;
    let months = args.get(1).and_then(|v| v.to_i64()).unwrap_or(0) as i32;
    let weeks = args.get(2).and_then(|v| v.to_i64()).unwrap_or(0) as i32;
    let days = args.get(3).and_then(|v| v.to_i64()).unwrap_or(0) as i32;
    let hours = args.get(4).and_then(|v| v.to_i64()).unwrap_or(0);
    let mins = args.get(5).and_then(|v| v.to_i64()).unwrap_or(0);
    let secs = args.get(6).and_then(|v| v.to_f64()).unwrap_or(0.0);
    let total_months = years * 12 + months;
    let total_days = weeks * 7 + days;
    let us = hours * 3_600_000_000 + mins * 60_000_000 + (secs * 1_000_000.0) as i64;
    Ok(PgValue::Interval(Interval { months: total_months, days: total_days, microseconds: us }))
}

pub fn to_timestamp(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Float8(_) | PgValue::Float4(_) => {
            let epoch = match &args[0] {
                PgValue::Float8(v) => *v,
                PgValue::Float4(v) => *v as f64,
                _ => unreachable!(),
            };
            use chrono::TimeZone;
            let ts = Utc.timestamp_opt(epoch as i64, ((epoch.fract() * 1e9) as u32)).single()
                .ok_or_else(|| Error::Pg(PgError::error(SqlState::DATETIME_FIELD_OVERFLOW, "invalid timestamp")))?;
            Ok(PgValue::TimestampTz(ts))
        }
        PgValue::Int4(v) => {
            use chrono::TimeZone;
            Ok(PgValue::TimestampTz(Utc.timestamp_opt(*v as i64, 0).single().unwrap_or_default()))
        }
        PgValue::Int8(v) => {
            use chrono::TimeZone;
            Ok(PgValue::TimestampTz(Utc.timestamp_opt(*v, 0).single().unwrap_or_default()))
        }
        PgValue::Text(s) | PgValue::Varchar(s) => {
            if args.len() > 1 {
                // to_timestamp(text, format)
                let fmt = pg_fmt_to_chrono(&args[1].to_text());
                chrono::NaiveDateTime::parse_from_str(s.trim(), &fmt)
                    .map(|ts| PgValue::TimestampTz(DateTime::from_naive_utc_and_offset(ts, Utc)))
                    .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_DATETIME_FORMAT, e.to_string())))
            } else {
                s.trim().parse::<f64>()
                    .map(|epoch| {
                        use chrono::TimeZone;
                        PgValue::TimestampTz(Utc.timestamp_opt(epoch as i64, 0).single().unwrap_or_default())
                    })
                    .map_err(|_| Error::Pg(PgError::invalid_text_representation("timestamptz", s)))
            }
        }
        _ => Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "to_timestamp requires numeric or text"))),
    }
}

pub fn to_date(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let s = args[0].to_text();
    let fmt = pg_fmt_to_chrono(&args[1].to_text());
    NaiveDate::parse_from_str(s.trim(), &fmt)
        .map(PgValue::Date)
        .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_DATETIME_FORMAT, e.to_string())))
}

pub fn to_char(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let fmt = pg_fmt_to_chrono(&args[1].to_text());
    let result = match &args[0] {
        PgValue::Timestamp(ts) => ts.format(&fmt).to_string(),
        PgValue::TimestampTz(ts) => ts.format(&fmt).to_string(),
        PgValue::Date(d) => d.format(&fmt).to_string(),
        PgValue::Time(t) => t.format(&fmt).to_string(),
        PgValue::Int4(v) => format_number(*v as f64, &args[1].to_text()),
        PgValue::Int8(v) => format_number(*v as f64, &args[1].to_text()),
        PgValue::Float8(v) => format_number(*v, &args[1].to_text()),
        PgValue::Numeric(v) => format_number(v.to_string().parse::<f64>().unwrap_or(0.0), &args[1].to_text()),
        other => other.to_text(),
    };
    Ok(PgValue::Text(result))
}

fn format_number(n: f64, fmt: &str) -> String {
    // Very simplified numeric formatting — just return the number
    format!("{n}")
}

/// Convert PostgreSQL date format codes to strftime format codes.
fn pg_fmt_to_chrono(fmt: &str) -> String {
    fmt.replace("YYYY", "%Y")
        .replace("YYY", "%Y")
        .replace("YY", "%y")
        .replace("MM", "%m")
        .replace("DD", "%d")
        .replace("HH24", "%H")
        .replace("HH12", "%I")
        .replace("HH", "%H")
        .replace("MI", "%M")
        .replace("SS", "%S")
        .replace("US", "%6f")
        .replace("MS", "%3f")
        .replace("TZ", "%Z")
        .replace("tz", "%z")
        .replace("OF", "%z")
        .replace("Day", "%A")
        .replace("DAY", "%A")
        .replace("day", "%a")
        .replace("Mon", "%b")
        .replace("MON", "%B")
        .replace("mon", "%b")
        .replace("Month", "%B")
        .replace("MONTH", "%B")
        .replace("DY", "%a")
        .replace("WW", "%U")
        .replace("IW", "%V")
        .replace("J", "%j")
        .replace("Q", "") // quarter — no strftime equivalent
}

pub fn justify_days(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    if let PgValue::Interval(iv) = &args[0] {
        let extra_months = iv.days / 30;
        let days = iv.days % 30;
        Ok(PgValue::Interval(Interval {
            months: iv.months + extra_months,
            days,
            microseconds: iv.microseconds,
        }))
    } else {
        Ok(args[0].clone())
    }
}

pub fn justify_hours(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    if let PgValue::Interval(iv) = &args[0] {
        let extra_days = iv.microseconds / 86_400_000_000;
        let us = iv.microseconds % 86_400_000_000;
        Ok(PgValue::Interval(Interval {
            months: iv.months,
            days: iv.days + extra_days as i32,
            microseconds: us,
        }))
    } else {
        Ok(args[0].clone())
    }
}

pub fn justify_interval(args: Vec<PgValue>) -> Result<PgValue> {
    let a = justify_hours(args)?;
    justify_days(vec![a])
}

pub fn isfinite(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Bool(match &args[0] {
        PgValue::Float4(v) => v.is_finite(),
        PgValue::Float8(v) => v.is_finite(),
        PgValue::Timestamp(_) | PgValue::TimestampTz(_) | PgValue::Date(_) => true,
        _ => true,
    }))
}

pub fn timezone(args: Vec<PgValue>) -> Result<PgValue> {
    // Simplified: we only support UTC, so just return the value unchanged
    if args.len() < 2 || args[1].is_null() { return Ok(PgValue::Null); }
    Ok(args[1].clone())
}
