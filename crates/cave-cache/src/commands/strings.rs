//! String commands: GET, SET, MGET, MSET, INCR, DECR, INCRBY, DECRBY, INCRBYFLOAT,
//! APPEND, STRLEN, GETRANGE, SETRANGE, SETNX, SETEX, PSETEX, GETSET, GETDEL, GETEX.

use std::time::{Duration, Instant};

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{bytes_to_f64, bytes_to_i64, f64_to_bytes, i64_to_bytes, Entry, Value};

// ── GET ───────────────────────────────────────────────────────────────────────

pub fn cmd_get(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("get"));
    }
    match db.get(&args[1]) {
        Some(e) => match &e.value {
            Value::String(v) => Ok(Resp::BulkString(Some(v.clone()))),
            _ => Err(CacheError::WrongType),
        },
        None => Ok(Resp::nil()),
    }
}

// ── SET ───────────────────────────────────────────────────────────────────────

pub fn cmd_set(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("set"));
    }

    let key = args[1].clone();
    let value = args[2].clone();

    // Parse options: EX seconds | PX milliseconds | EXAT timestamp | PXAT timestamp
    //                NX | XX | GET | KEEPTTL
    let mut ex: Option<i64> = None;
    let mut px: Option<i64> = None;
    let mut exat: Option<i64> = None;
    let mut pxat: Option<i64> = None;
    let mut nx = false;
    let mut xx = false;
    let mut get = false;
    let mut keepttl = false;

    let mut i = 3;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"EX" => {
                i += 1;
                ex = Some(bytes_to_i64(&args[i]).ok_or_else(|| CacheError::InvalidExpire("set".into()))?);
            }
            b"PX" => {
                i += 1;
                px = Some(bytes_to_i64(&args[i]).ok_or_else(|| CacheError::InvalidExpire("set".into()))?);
            }
            b"EXAT" => {
                i += 1;
                exat = Some(bytes_to_i64(&args[i]).ok_or_else(|| CacheError::InvalidExpire("set".into()))?);
            }
            b"PXAT" => {
                i += 1;
                pxat = Some(bytes_to_i64(&args[i]).ok_or_else(|| CacheError::InvalidExpire("set".into()))?);
            }
            b"NX" => nx = true,
            b"XX" => xx = true,
            b"GET" => get = true,
            b"KEEPTTL" => keepttl = true,
            _ => return Err(CacheError::Syntax),
        }
        i += 1;
    }

    // GET: return old value
    let old_value = if get {
        match db.get(&key) {
            Some(e) => match &e.value {
                Value::String(v) => Some(Resp::BulkString(Some(v.clone()))),
                _ => return Err(CacheError::WrongType),
            },
            None => Some(Resp::nil()),
        }
    } else {
        None
    };

    // NX: only set if not exists
    if nx && db.exists(&key) {
        return Ok(old_value.unwrap_or(Resp::nil()));
    }
    // XX: only set if exists
    if xx && !db.exists(&key) {
        return Ok(old_value.unwrap_or(Resp::nil()));
    }

    // Compute expiry
    let old_expiry = if keepttl { db.get(&key).and_then(|e| e.expires_at) } else { None };

    let expires_at = if keepttl {
        old_expiry
    } else if let Some(secs) = ex {
        if secs <= 0 { return Err(CacheError::InvalidExpire("set".into())); }
        Some(Instant::now() + Duration::from_secs(secs as u64))
    } else if let Some(ms) = px {
        if ms <= 0 { return Err(CacheError::InvalidExpire("set".into())); }
        Some(Instant::now() + Duration::from_millis(ms as u64))
    } else if let Some(ts) = exat {
        let duration = Duration::from_secs(ts as u64).checked_sub(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
        );
        duration.map(|d| Instant::now() + d)
    } else if let Some(ts) = pxat {
        let duration = Duration::from_millis(ts as u64).checked_sub(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
        );
        duration.map(|d| Instant::now() + d)
    } else {
        None
    };

    let mut entry = Entry::new(Value::String(value));
    entry.expires_at = expires_at;
    db.insert(key, entry);

    if let Some(old) = old_value {
        Ok(old)
    } else {
        Ok(Resp::ok())
    }
}

// ── MGET ─────────────────────────────────────────────────────────────────────

pub fn cmd_mget(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("mget"));
    }
    let results = args[1..]
        .iter()
        .map(|key| match db.get(key) {
            Some(e) => match &e.value {
                Value::String(v) => Resp::BulkString(Some(v.clone())),
                _ => Resp::nil(),
            },
            None => Resp::nil(),
        })
        .collect();
    Ok(Resp::Array(Some(results)))
}

// ── MSET / MSETNX ────────────────────────────────────────────────────────────

pub fn cmd_mset(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 || (args.len() - 1) % 2 != 0 {
        return Err(CacheError::wrong_arity("mset"));
    }
    let mut i = 1;
    while i < args.len() {
        let key = args[i].clone();
        let val = args[i + 1].clone();
        db.insert(key, Entry::new(Value::String(val)));
        i += 2;
    }
    Ok(Resp::ok())
}

pub fn cmd_msetnx(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 || (args.len() - 1) % 2 != 0 {
        return Err(CacheError::wrong_arity("msetnx"));
    }
    // Check none exist first
    let mut i = 1;
    while i < args.len() {
        if db.exists(&args[i]) {
            return Ok(Resp::Integer(0));
        }
        i += 2;
    }
    cmd_mset(args, db)?;
    Ok(Resp::Integer(1))
}

// ── INCR / DECR ──────────────────────────────────────────────────────────────

fn get_or_zero_i64(db: &mut Db, key: &[u8]) -> CacheResult<i64> {
    match db.get(key) {
        None => Ok(0),
        Some(e) => match &e.value {
            Value::String(v) => bytes_to_i64(v).ok_or(CacheError::NotInteger),
            _ => Err(CacheError::WrongType),
        },
    }
}

pub fn cmd_incr(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("incr")); }
    let key = &args[1];
    let current = get_or_zero_i64(db, key)?;
    let new_val = current.checked_add(1).ok_or_else(|| CacheError::generic("ERR increment or decrement would overflow"))?;
    let expires_at = db.get(key).and_then(|e| e.expires_at);
    let mut entry = Entry::new(Value::String(i64_to_bytes(new_val)));
    entry.expires_at = expires_at;
    db.insert(key.to_vec(), entry);
    Ok(Resp::Integer(new_val))
}

pub fn cmd_decr(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("decr")); }
    let key = &args[1];
    let current = get_or_zero_i64(db, key)?;
    let new_val = current.checked_sub(1).ok_or_else(|| CacheError::generic("ERR increment or decrement would overflow"))?;
    let expires_at = db.get(key).and_then(|e| e.expires_at);
    let mut entry = Entry::new(Value::String(i64_to_bytes(new_val)));
    entry.expires_at = expires_at;
    db.insert(key.to_vec(), entry);
    Ok(Resp::Integer(new_val))
}

pub fn cmd_incrby(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("incrby")); }
    let key = &args[1];
    let delta = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let current = get_or_zero_i64(db, key)?;
    let new_val = current.checked_add(delta).ok_or_else(|| CacheError::generic("ERR increment or decrement would overflow"))?;
    let expires_at = db.get(key).and_then(|e| e.expires_at);
    let mut entry = Entry::new(Value::String(i64_to_bytes(new_val)));
    entry.expires_at = expires_at;
    db.insert(key.to_vec(), entry);
    Ok(Resp::Integer(new_val))
}

pub fn cmd_decrby(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("decrby")); }
    let key = &args[1];
    let delta = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let current = get_or_zero_i64(db, key)?;
    let new_val = current.checked_sub(delta).ok_or_else(|| CacheError::generic("ERR increment or decrement would overflow"))?;
    let expires_at = db.get(key).and_then(|e| e.expires_at);
    let mut entry = Entry::new(Value::String(i64_to_bytes(new_val)));
    entry.expires_at = expires_at;
    db.insert(key.to_vec(), entry);
    Ok(Resp::Integer(new_val))
}

pub fn cmd_incrbyfloat(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("incrbyfloat")); }
    let key = &args[1];
    let delta = bytes_to_f64(&args[2]).ok_or(CacheError::NotFloat)?;

    let current: f64 = match db.get(key) {
        None => 0.0,
        Some(e) => match &e.value {
            Value::String(v) => bytes_to_f64(v).ok_or(CacheError::NotFloat)?,
            _ => return Err(CacheError::WrongType),
        },
    };

    let new_val = current + delta;
    if new_val.is_nan() || new_val.is_infinite() {
        return Err(CacheError::generic("ERR increment would produce NaN or Infinity"));
    }

    // Format like Redis: avoid scientific notation for reasonable values
    let s = format_float(new_val);
    let expires_at = db.get(key).and_then(|e| e.expires_at);
    let mut entry = Entry::new(Value::String(s.into_bytes()));
    entry.expires_at = expires_at;
    db.insert(key.to_vec(), entry);
    Ok(Resp::BulkString(Some(entry_to_bytes(&db.get(&args[1]).unwrap().value))))
}

fn entry_to_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::String(s) => s.clone(),
        _ => vec![],
    }
}

fn format_float(f: f64) -> String {
    // Redis uses minimal decimal representation
    let s = format!("{:.17}", f);
    // Trim trailing zeros
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

// ── APPEND ────────────────────────────────────────────────────────────────────

pub fn cmd_append(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("append")); }
    let key = args[1].clone();
    let append = args[2].clone();

    match db.get_typed_mut(&key, "string")? {
        Some(entry) => {
            if let Value::String(v) = &mut entry.value {
                v.extend_from_slice(&append);
                let len = v.len() as i64;
                return Ok(Resp::Integer(len));
            }
            unreachable!()
        }
        None => {
            let len = append.len() as i64;
            db.insert(key, Entry::new(Value::String(append)));
            Ok(Resp::Integer(len))
        }
    }
}

// ── STRLEN ────────────────────────────────────────────────────────────────────

pub fn cmd_strlen(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("strlen")); }
    match db.get_typed(&args[1], "string")? {
        Some(e) => match &e.value {
            Value::String(v) => Ok(Resp::Integer(v.len() as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── GETRANGE / SUBSTR ────────────────────────────────────────────────────────

pub fn cmd_getrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("getrange")); }
    let start = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let end = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;

    let bytes = match db.get_typed(&args[1], "string")? {
        Some(e) => match &e.value {
            Value::String(v) => v.clone(),
            _ => unreachable!(),
        },
        None => return Ok(Resp::BulkString(Some(vec![]))),
    };

    let len = bytes.len() as i64;
    let start = normalize_str_index(start, len);
    let end = normalize_str_index(end, len);

    if start > end || start >= bytes.len() {
        return Ok(Resp::BulkString(Some(vec![])));
    }

    let end = end.min(bytes.len() - 1);
    Ok(Resp::BulkString(Some(bytes[start..=end].to_vec())))
}

fn normalize_str_index(idx: i64, len: i64) -> usize {
    if idx < 0 {
        let adj = len + idx;
        if adj < 0 { 0 } else { adj as usize }
    } else {
        idx as usize
    }
}

// ── SETRANGE ─────────────────────────────────────────────────────────────────

pub fn cmd_setrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("setrange")); }
    let key = args[1].clone();
    let offset = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)? as usize;
    let patch = &args[3];

    if offset > 512 * 1024 * 1024 {
        return Err(CacheError::generic("ERR string exceeds maximum allowed size (512MB)"));
    }

    let mut bytes = match db.get_typed_mut(&key, "string")? {
        Some(e) => match &e.value {
            Value::String(v) => v.clone(),
            _ => unreachable!(),
        },
        None => vec![],
    };

    let needed = offset + patch.len();
    if bytes.len() < needed {
        bytes.resize(needed, 0);
    }
    bytes[offset..offset + patch.len()].copy_from_slice(patch);

    let len = bytes.len() as i64;
    db.insert(key, Entry::new(Value::String(bytes)));
    Ok(Resp::Integer(len))
}

// ── SETNX / SETEX / PSETEX ───────────────────────────────────────────────────

pub fn cmd_setnx(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("setnx")); }
    if db.exists(&args[1]) {
        return Ok(Resp::Integer(0));
    }
    db.insert(args[1].clone(), Entry::new(Value::String(args[2].clone())));
    Ok(Resp::Integer(1))
}

pub fn cmd_setex(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("setex")); }
    let secs = bytes_to_i64(&args[2]).ok_or_else(|| CacheError::InvalidExpire("setex".into()))?;
    if secs <= 0 { return Err(CacheError::InvalidExpire("setex".into())); }
    let mut entry = Entry::new(Value::String(args[3].clone()));
    entry.expires_at = Some(Instant::now() + Duration::from_secs(secs as u64));
    db.insert(args[1].clone(), entry);
    Ok(Resp::ok())
}

pub fn cmd_psetex(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("psetex")); }
    let ms = bytes_to_i64(&args[2]).ok_or_else(|| CacheError::InvalidExpire("psetex".into()))?;
    if ms <= 0 { return Err(CacheError::InvalidExpire("psetex".into())); }
    let mut entry = Entry::new(Value::String(args[3].clone()));
    entry.expires_at = Some(Instant::now() + Duration::from_millis(ms as u64));
    db.insert(args[1].clone(), entry);
    Ok(Resp::ok())
}

// ── GETSET ───────────────────────────────────────────────────────────────────

pub fn cmd_getset(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("getset")); }
    let key = args[1].clone();
    let new_val = args[2].clone();

    let old = match db.get(&key) {
        Some(e) => match &e.value {
            Value::String(v) => Resp::BulkString(Some(v.clone())),
            _ => return Err(CacheError::WrongType),
        },
        None => Resp::nil(),
    };

    db.insert(key, Entry::new(Value::String(new_val)));
    Ok(old)
}

// ── GETDEL ───────────────────────────────────────────────────────────────────

pub fn cmd_getdel(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("getdel")); }
    match db.get(&args[1]) {
        Some(e) => match &e.value {
            Value::String(v) => {
                let v = v.clone();
                db.remove(&args[1]);
                Ok(Resp::BulkString(Some(v)))
            }
            _ => Err(CacheError::WrongType),
        },
        None => Ok(Resp::nil()),
    }
}

// ── GETEX ────────────────────────────────────────────────────────────────────

pub fn cmd_getex(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("getex")); }
    let key = &args[1];

    let val = match db.get(key) {
        Some(e) => match &e.value {
            Value::String(v) => v.clone(),
            _ => return Err(CacheError::WrongType),
        },
        None => return Ok(Resp::nil()),
    };

    // Parse options
    let mut i = 2;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"EX" => {
                i += 1;
                let secs = bytes_to_i64(&args[i]).ok_or_else(|| CacheError::InvalidExpire("getex".into()))?;
                if let Some(e) = db.get_mut(key) {
                    e.expires_at = Some(Instant::now() + Duration::from_secs(secs as u64));
                }
            }
            b"PX" => {
                i += 1;
                let ms = bytes_to_i64(&args[i]).ok_or_else(|| CacheError::InvalidExpire("getex".into()))?;
                if let Some(e) = db.get_mut(key) {
                    e.expires_at = Some(Instant::now() + Duration::from_millis(ms as u64));
                }
            }
            b"PERSIST" => {
                if let Some(e) = db.get_mut(key) {
                    e.expires_at = None;
                }
            }
            _ => return Err(CacheError::Syntax),
        }
        i += 1;
    }

    Ok(Resp::BulkString(Some(val)))
}
