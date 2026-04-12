//! PostgreSQL system information functions.

use crate::error::Result;
use crate::types::{oid, PgValue};
use chrono::Utc;

pub fn version(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text(
        "PostgreSQL 16.0 (cave-pg 0.1.0) on x86_64-linux, compiled by Rust".to_string()
    ))
}

pub fn current_database(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text("postgres".to_string()))
}

pub fn current_schema(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text("public".to_string()))
}

pub fn current_user(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text("postgres".to_string()))
}

pub fn pg_backend_pid(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Int4(std::process::id() as i32))
}

pub fn pg_current_wal_lsn(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Text("0/1000000".to_string()))
}

pub fn pg_postmaster_start_time(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::TimestampTz(Utc::now()))
}

pub fn pg_typeof(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() {
        return Ok(PgValue::Text("unknown".to_string()));
    }
    Ok(PgValue::Text(
        crate::types::type_name_for_oid(args[0].oid()).to_string()
    ))
}

pub fn pg_column_size(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let size = match &args[0] {
        PgValue::Bool(_) => 1,
        PgValue::Int2(_) => 2,
        PgValue::Int4(_) => 4,
        PgValue::Int8(_) => 8,
        PgValue::Float4(_) => 4,
        PgValue::Float8(_) => 8,
        PgValue::Text(s) | PgValue::Varchar(s) | PgValue::Char(s) => s.len() as i32,
        PgValue::Bytea(b) => b.len() as i32,
        PgValue::Uuid(_) => 16,
        PgValue::Date(_) => 4,
        PgValue::Time(_) => 8,
        PgValue::Timestamp(_) | PgValue::TimestampTz(_) => 8,
        PgValue::Json(j) | PgValue::Jsonb(j) => j.to_string().len() as i32,
        _ => -1,
    };
    if size < 0 { Ok(PgValue::Null) } else { Ok(PgValue::Int4(size)) }
}

pub fn pg_size_pretty(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let bytes = args[0].to_i64().unwrap_or(0);
    let result = if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} kB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    };
    Ok(PgValue::Text(result))
}

pub fn pg_relation_size(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Int8(8192)) // 1 page
}

pub fn pg_total_relation_size(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Int8(8192))
}

pub fn pg_database_size(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Int8(1024 * 1024)) // 1 MB placeholder
}

pub fn pg_sleep(args: Vec<PgValue>) -> Result<PgValue> {
    // In an async context we should use tokio::time::sleep, but as a function
    // we just return void without actually sleeping (caller would need to handle)
    Ok(PgValue::Void)
}

pub fn pg_is_in_recovery(_args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Bool(false))
}

pub fn has_privilege(args: Vec<PgValue>) -> Result<PgValue> {
    // Always return true (no ACL enforcement)
    Ok(PgValue::Bool(true))
}
