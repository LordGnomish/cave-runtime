//! Built-in PostgreSQL functions — string, math, date/time, JSON, array, system.

pub mod array;
pub mod datetime;
pub mod json;
pub mod math;
pub mod string;
pub mod system;

use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{oid, PgValue};

/// Dispatch a function call by name + arguments.
/// Returns the result value or an error.
pub fn call(name: &str, args: Vec<PgValue>) -> Result<PgValue> {
    let lower = name.to_lowercase();
    match lower.as_str() {
        // ── String functions ──────────────────────────────────────────────────
        "length" | "char_length" | "character_length" => string::length(args),
        "octet_length" => string::octet_length(args),
        "bit_length" => string::bit_length(args),
        "upper" => string::upper(args),
        "lower" => string::lower(args),
        "initcap" => string::initcap(args),
        "trim" | "btrim" => string::trim(args),
        "ltrim" => string::ltrim(args),
        "rtrim" => string::rtrim(args),
        "lpad" => string::lpad(args),
        "rpad" => string::rpad(args),
        "left" => string::left(args),
        "right" => string::right(args),
        "reverse" => string::reverse(args),
        "repeat" => string::repeat_str(args),
        "substring" | "substr" => string::substring(args),
        "position" => string::position(args),
        "strpos" => string::strpos(args),
        "replace" => string::replace(args),
        "concat" => string::concat(args),
        "concat_ws" => string::concat_ws(args),
        "split_part" => string::split_part(args),
        "string_to_array" => string::string_to_array(args),
        "array_to_string" => string::array_to_string(args),
        "regexp_match" => string::regexp_match(args),
        "regexp_replace" => string::regexp_replace(args),
        "regexp_split_to_array" => string::regexp_split_to_array(args),
        "encode" => string::encode(args),
        "decode" => string::decode(args),
        "md5" => string::md5(args),
        "sha256" => string::sha256(args),
        "format" => string::format_str(args),
        "quote_ident" => string::quote_ident(args),
        "quote_literal" => string::quote_literal(args),
        "chr" => string::chr_fn(args),
        "ascii" => string::ascii_fn(args),
        "translate" => string::translate(args),
        "overlay" => string::overlay(args),
        "starts_with" => string::starts_with(args),
        "ends_with" => string::ends_with_fn(args),
        "to_hex" => string::to_hex(args),
        "convert" | "convert_from" | "convert_to" => string::convert_encoding(args),

        // ── Math functions ────────────────────────────────────────────────────
        "abs" => math::abs(args),
        "ceil" | "ceiling" => math::ceil(args),
        "floor" => math::floor(args),
        "round" => math::round(args),
        "trunc" => math::trunc(args),
        "sign" => math::sign(args),
        "mod" => math::mod_fn(args),
        "power" | "pow" => math::power(args),
        "sqrt" => math::sqrt(args),
        "cbrt" => math::cbrt(args),
        "log" => math::log(args),
        "log10" => math::log10(args),
        "ln" => math::ln(args),
        "exp" => math::exp(args),
        "pi" => math::pi(args),
        "random" => math::random(args),
        "setseed" => math::setseed(args),
        "greatest" => math::greatest(args),
        "least" => math::least(args),
        "width_bucket" => math::width_bucket(args),
        "degrees" => math::degrees(args),
        "radians" => math::radians(args),
        "sin" => math::sin(args),
        "cos" => math::cos(args),
        "tan" => math::tan(args),
        "asin" => math::asin(args),
        "acos" => math::acos(args),
        "atan" => math::atan(args),
        "atan2" => math::atan2(args),
        "sinh" => math::sinh(args),
        "cosh" => math::cosh(args),
        "tanh" => math::tanh(args),
        "factorial" => math::factorial(args),
        "gcd" => math::gcd(args),
        "lcm" => math::lcm(args),
        "min_scale" => math::min_scale(args),
        "trim_scale" => math::trim_scale(args),
        "scale" => math::scale_fn(args),
        "numeric_scale" => math::scale_fn(args),
        "div" => math::div(args),

        // ── Date/time functions ───────────────────────────────────────────────
        "now" | "current_timestamp" | "transaction_timestamp" => datetime::now(args),
        "clock_timestamp" => datetime::clock_timestamp(args),
        "statement_timestamp" => datetime::now(args),
        "timeofday" => datetime::timeofday(args),
        "current_date" => datetime::current_date(args),
        "current_time" => datetime::current_time(args),
        "localtime" => datetime::localtime(args),
        "localtimestamp" => datetime::localtimestamp(args),
        "date_trunc" => datetime::date_trunc(args),
        "date_part" => datetime::date_part(args),
        "extract" => datetime::date_part(args),
        "date_bin" => datetime::date_bin(args),
        "age" => datetime::age(args),
        "make_date" => datetime::make_date(args),
        "make_time" => datetime::make_time(args),
        "make_timestamp" => datetime::make_timestamp(args),
        "make_timestamptz" => datetime::make_timestamptz(args),
        "make_interval" => datetime::make_interval(args),
        "to_timestamp" => datetime::to_timestamp(args),
        "to_date" => datetime::to_date(args),
        "to_char" => datetime::to_char(args),
        "justify_days" => datetime::justify_days(args),
        "justify_hours" => datetime::justify_hours(args),
        "justify_interval" => datetime::justify_interval(args),
        "isfinite" => datetime::isfinite(args),
        "timezone" => datetime::timezone(args),

        // ── JSON/JSONB functions ──────────────────────────────────────────────
        "json_build_object" | "jsonb_build_object" => json::build_object(args),
        "json_build_array" | "jsonb_build_array" => json::build_array(args),
        "json_object" | "jsonb_object" => json::json_object(args),
        "json_array_length" | "jsonb_array_length" => json::array_length(args),
        "json_typeof" | "jsonb_typeof" => json::typeof_fn(args),
        "json_strip_nulls" | "jsonb_strip_nulls" => json::strip_nulls(args),
        "json_extract_path" | "jsonb_extract_path" => json::extract_path(args),
        "json_extract_path_text" | "jsonb_extract_path_text" => json::extract_path_text(args),
        "json_object_keys" | "jsonb_object_keys" => json::object_keys(args),
        "json_each" | "jsonb_each" => json::each(args),
        "json_each_text" | "jsonb_each_text" => json::each_text(args),
        "json_to_record" | "jsonb_to_record" => json::to_record(args),
        "jsonb_set" => json::jsonb_set(args),
        "jsonb_insert" => json::jsonb_insert(args),
        "jsonb_pretty" => json::jsonb_pretty(args),
        "jsonb_path_query" => json::jsonb_path_query(args),
        "jsonb_path_exists" => json::jsonb_path_exists(args),
        "row_to_json" => json::row_to_json(args),
        "to_json" | "to_jsonb" => json::to_json(args),
        "array_to_json" => json::array_to_json(args),

        // ── Array functions ───────────────────────────────────────────────────
        "array_append" => array::append(args),
        "array_prepend" => array::prepend(args),
        "array_cat" => array::cat(args),
        "array_length" => array::length(args),
        "array_ndims" => array::ndims(args),
        "array_dims" => array::dims(args),
        "array_lower" => array::lower(args),
        "array_upper" => array::upper(args),
        "unnest" => array::unnest(args),
        "array_position" => array::position(args),
        "array_positions" => array::positions(args),
        "array_remove" => array::remove(args),
        "array_replace" => array::replace(args),
        "array_fill" => array::fill(args),
        "cardinality" => array::cardinality(args),
        "array_agg" => array::agg(args),

        // ── System functions ──────────────────────────────────────────────────
        "version" => system::version(args),
        "current_database" => system::current_database(args),
        "current_schema" | "current_schemas" => system::current_schema(args),
        "current_user" | "session_user" => system::current_user(args),
        "user" => system::current_user(args),
        "pg_backend_pid" => system::pg_backend_pid(args),
        "pg_current_wal_lsn" => system::pg_current_wal_lsn(args),
        "pg_postmaster_start_time" => system::pg_postmaster_start_time(args),
        "pg_conf_load_time" => system::pg_postmaster_start_time(args),
        "pg_typeof" => system::pg_typeof(args),
        "pg_column_size" => system::pg_column_size(args),
        "pg_size_pretty" => system::pg_size_pretty(args),
        "pg_relation_size" => system::pg_relation_size(args),
        "pg_total_relation_size" => system::pg_total_relation_size(args),
        "pg_database_size" => system::pg_database_size(args),
        "pg_sleep" => system::pg_sleep(args),
        "pg_is_in_recovery" => system::pg_is_in_recovery(args),
        "has_table_privilege" | "has_column_privilege" |
        "has_schema_privilege" | "has_database_privilege" |
        "has_function_privilege" | "has_sequence_privilege" => system::has_privilege(args),
        "obj_description" | "col_description" | "shobj_description" => {
            Ok(PgValue::Null)
        }
        "coalesce" => {
            for a in args {
                if !a.is_null() { return Ok(a); }
            }
            Ok(PgValue::Null)
        }
        "nullif" => {
            if args.len() != 2 { return Err(Error::Pg(PgError::error(SqlState::TOO_MANY_ARGUMENTS, "nullif requires 2 args"))); }
            let (a, b) = (args[0].clone(), args[1].clone());
            if a == b { Ok(PgValue::Null) } else { Ok(a) }
        }
        "ifnull" | "nvl" => {
            // Not standard SQL but commonly expected
            if args.len() != 2 { return Err(Error::Pg(PgError::error(SqlState::TOO_MANY_ARGUMENTS, "ifnull requires 2 args"))); }
            if args[0].is_null() { Ok(args[1].clone()) } else { Ok(args[0].clone()) }
        }
        "bool_and" | "every" => {
            if args.is_empty() { return Ok(PgValue::Null); }
            for a in &args { if !a.is_true() { return Ok(PgValue::Bool(false)); } }
            Ok(PgValue::Bool(true))
        }
        "bool_or" => {
            if args.is_empty() { return Ok(PgValue::Null); }
            for a in &args { if a.is_true() { return Ok(PgValue::Bool(true)); } }
            Ok(PgValue::Bool(false))
        }
        "generate_series" => generate_series(args),
        "generate_subscripts" => generate_subscripts(args),
        "unnest" => array::unnest(args),
        // Type casting functions
        "int2" | "smallint" => cast_to_int2(args),
        "int4" | "integer" | "int" => cast_to_int4(args),
        "int8" | "bigint" => cast_to_int8(args),
        "float4" | "real" => cast_to_float4(args),
        "float8" | "double precision" => cast_to_float8(args),
        "numeric" | "decimal" => cast_to_numeric(args),
        "text" => cast_to_text(args),
        "bool" | "boolean" => cast_to_bool(args),
        "uuid" => cast_to_uuid(args),
        "date" => cast_to_date(args),
        "timestamp" => cast_to_timestamp(args),
        "timestamptz" => cast_to_timestamptz(args),
        "interval" => cast_to_interval(args),
        "json" => cast_to_json(args),
        "jsonb" => cast_to_jsonb(args),
        "bytea" => cast_to_bytea(args),
        _ => Err(Error::Pg(PgError::error(
            SqlState::UNDEFINED_FUNCTION,
            format!("function {name}() does not exist"),
        ))),
    }
}

// ── Generate series ───────────────────────────────────────────────────────────

/// generate_series(start, stop [, step]) — returns a set of values.
/// We return it as an array here (the executor handles set-returning functions).
pub fn generate_series(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args.len() > 3 {
        return Err(Error::Pg(PgError::error(
            SqlState::TOO_MANY_ARGUMENTS,
            "generate_series requires 2 or 3 arguments",
        )));
    }
    let start = args[0].to_i64().unwrap_or(0);
    let stop = args[1].to_i64().unwrap_or(0);
    let step = if args.len() == 3 { args[2].to_i64().unwrap_or(1) } else { 1 };
    if step == 0 {
        return Err(Error::Pg(PgError::error(
            SqlState::INVALID_PARAMETER_VALUE,
            "step size cannot equal zero",
        )));
    }
    let mut values = Vec::new();
    let mut v = start;
    while (step > 0 && v <= stop) || (step < 0 && v >= stop) {
        values.push(PgValue::Int8(v));
        v = v.saturating_add(step);
    }
    Ok(PgValue::Array { element_oid: oid::INT8, elements: values })
}

pub fn generate_subscripts(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() {
        return Ok(PgValue::Array { element_oid: oid::INT4, elements: vec![] });
    }
    if let PgValue::Array { elements, element_oid } = &args[0] {
        let len = elements.len() as i64;
        let result: Vec<PgValue> = (1..=len).map(PgValue::Int8).collect();
        return Ok(PgValue::Array { element_oid: oid::INT4, elements: result });
    }
    Ok(PgValue::Array { element_oid: oid::INT4, elements: vec![] })
}

// ── Cast helpers ──────────────────────────────────────────────────────────────

fn single_arg(name: &str, args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() != 1 {
        Err(Error::Pg(PgError::error(
            SqlState::TOO_MANY_ARGUMENTS,
            format!("{name} requires exactly 1 argument"),
        )))
    } else {
        Ok(args.into_iter().next().unwrap())
    }
}

fn cast_to_int2(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("int2", args)?.cast_to(oid::INT2)
}
fn cast_to_int4(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("int4", args)?.cast_to(oid::INT4)
}
fn cast_to_int8(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("int8", args)?.cast_to(oid::INT8)
}
fn cast_to_float4(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("float4", args)?.cast_to(oid::FLOAT4)
}
fn cast_to_float8(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("float8", args)?.cast_to(oid::FLOAT8)
}
fn cast_to_numeric(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("numeric", args)?.cast_to(oid::NUMERIC)
}
fn cast_to_text(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text(single_arg("text", args)?.to_text()))
}
fn cast_to_bool(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("bool", args)?.cast_to(oid::BOOL)
}
fn cast_to_uuid(args: Vec<PgValue>) -> Result<PgValue> {
    single_arg("uuid", args)?.cast_to(oid::UUID)
}
fn cast_to_date(args: Vec<PgValue>) -> Result<PgValue> {
    let v = single_arg("date", args)?;
    match v {
        PgValue::Null => Ok(PgValue::Null),
        PgValue::Date(_) => Ok(v),
        PgValue::Timestamp(ts) => Ok(PgValue::Date(ts.date())),
        PgValue::TimestampTz(ts) => Ok(PgValue::Date(ts.naive_utc().date())),
        PgValue::Text(s) | PgValue::Varchar(s) => {
            let d = s.trim().parse::<chrono::NaiveDate>()
                .map_err(|_| Error::Pg(PgError::invalid_text_representation("date", &s)))?;
            Ok(PgValue::Date(d))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::CANNOT_COERCE, "cannot cast to date"))),
    }
}
fn cast_to_timestamp(args: Vec<PgValue>) -> Result<PgValue> {
    let v = single_arg("timestamp", args)?;
    match v {
        PgValue::Null => Ok(PgValue::Null),
        PgValue::Timestamp(_) => Ok(v),
        PgValue::TimestampTz(ts) => Ok(PgValue::Timestamp(ts.naive_utc())),
        PgValue::Date(d) => Ok(PgValue::Timestamp(d.and_hms_opt(0, 0, 0).unwrap())),
        PgValue::Text(s) | PgValue::Varchar(s) => {
            let ts = s.trim().parse::<chrono::NaiveDateTime>()
                .map_err(|_| Error::Pg(PgError::invalid_text_representation("timestamp", &s)))?;
            Ok(PgValue::Timestamp(ts))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::CANNOT_COERCE, "cannot cast to timestamp"))),
    }
}
fn cast_to_timestamptz(args: Vec<PgValue>) -> Result<PgValue> {
    let v = single_arg("timestamptz", args)?;
    match v {
        PgValue::Null => Ok(PgValue::Null),
        PgValue::TimestampTz(_) => Ok(v),
        PgValue::Timestamp(ts) => Ok(PgValue::TimestampTz(chrono::DateTime::from_naive_utc_and_offset(ts, chrono::Utc))),
        PgValue::Int8(epoch) => {
            use chrono::TimeZone;
            Ok(PgValue::TimestampTz(chrono::Utc.timestamp_opt(epoch, 0).single().unwrap_or_default()))
        }
        PgValue::Float8(epoch) => {
            use chrono::TimeZone;
            Ok(PgValue::TimestampTz(chrono::Utc.timestamp_opt(epoch as i64, ((epoch.fract() * 1e9) as u32)).single().unwrap_or_default()))
        }
        PgValue::Text(s) | PgValue::Varchar(s) => {
            // Try various timestamp formats
            for fmt in &[
                "%Y-%m-%d %H:%M:%S%z",
                "%Y-%m-%dT%H:%M:%S%z",
                "%Y-%m-%d %H:%M:%S",
                "%Y-%m-%dT%H:%M:%S",
            ] {
                if let Ok(ts) = chrono::DateTime::parse_from_str(s.trim(), fmt) {
                    return Ok(PgValue::TimestampTz(ts.into()));
                }
            }
            Err(Error::Pg(PgError::invalid_text_representation("timestamptz", &s)))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::CANNOT_COERCE, "cannot cast to timestamptz"))),
    }
}
pub fn cast_to_interval(args: Vec<PgValue>) -> Result<PgValue> {
    let v = single_arg("interval", args)?;
    match v {
        PgValue::Null => Ok(PgValue::Null),
        PgValue::Interval(_) => Ok(v),
        PgValue::Text(s) | PgValue::Varchar(s) => {
            // Basic interval parsing: "N seconds", "N minutes", "N hours", "N days"
            // This is a simplified parser; full ISO8601 and PostgreSQL interval syntax is complex
            let s = s.trim();
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() == 2 {
                let n: f64 = parts[0].parse().map_err(|_| {
                    Error::Pg(PgError::invalid_text_representation("interval", s))
                })?;
                let unit = parts[1].to_lowercase();
                let iv = match unit.as_str() {
                    "second" | "seconds" | "sec" | "secs" => {
                        crate::types::Interval::from_seconds(n)
                    }
                    "minute" | "minutes" | "min" | "mins" => {
                        crate::types::Interval::from_seconds(n * 60.0)
                    }
                    "hour" | "hours" => {
                        crate::types::Interval::from_seconds(n * 3600.0)
                    }
                    "day" | "days" => {
                        crate::types::Interval { months: 0, days: n as i32, microseconds: 0 }
                    }
                    "week" | "weeks" => {
                        crate::types::Interval { months: 0, days: (n * 7.0) as i32, microseconds: 0 }
                    }
                    "month" | "months" | "mon" | "mons" => {
                        crate::types::Interval { months: n as i32, days: 0, microseconds: 0 }
                    }
                    "year" | "years" => {
                        crate::types::Interval { months: (n * 12.0) as i32, days: 0, microseconds: 0 }
                    }
                    _ => return Err(Error::Pg(PgError::invalid_text_representation("interval", s))),
                };
                return Ok(PgValue::Interval(iv));
            }
            Err(Error::Pg(PgError::invalid_text_representation("interval", s)))
        }
        _ => Err(Error::Pg(PgError::error(SqlState::CANNOT_COERCE, "cannot cast to interval"))),
    }
}
fn cast_to_json(args: Vec<PgValue>) -> Result<PgValue> {
    let v = single_arg("json", args)?;
    match v {
        PgValue::Null => Ok(PgValue::Null),
        PgValue::Json(_) => Ok(v),
        PgValue::Jsonb(j) => Ok(PgValue::Json(j)),
        PgValue::Text(s) | PgValue::Varchar(s) => {
            let j: serde_json::Value = serde_json::from_str(&s)
                .map_err(|e| Error::Pg(PgError::error(SqlState::INVALID_JSON_TEXT, e.to_string())))?;
            Ok(PgValue::Json(j))
        }
        other => Ok(PgValue::Json(serde_json::Value::String(other.to_text()))),
    }
}
fn cast_to_jsonb(args: Vec<PgValue>) -> Result<PgValue> {
    cast_to_json(args).map(|v| if let PgValue::Json(j) = v { PgValue::Jsonb(j) } else { v })
}
fn cast_to_bytea(args: Vec<PgValue>) -> Result<PgValue> {
    let v = single_arg("bytea", args)?;
    match v {
        PgValue::Null => Ok(PgValue::Null),
        PgValue::Bytea(_) => Ok(v),
        PgValue::Text(s) | PgValue::Varchar(s) => {
            // Accept hex: \x... or plain bytes
            let s = s.trim();
            if let Some(hex) = s.strip_prefix("\\x") {
                let bytes = hex::decode(hex)
                    .map_err(|_| Error::Pg(PgError::invalid_text_representation("bytea", s)))?;
                Ok(PgValue::Bytea(bytes))
            } else {
                Ok(PgValue::Bytea(s.as_bytes().to_vec()))
            }
        }
        _ => Err(Error::Pg(PgError::error(SqlState::CANNOT_COERCE, "cannot cast to bytea"))),
    }
}
