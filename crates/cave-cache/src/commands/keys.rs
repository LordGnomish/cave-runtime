// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Generic key commands: SCAN, SSCAN, ZSCAN, KEYS, EXISTS, DEL, UNLINK, TYPE,
//! RENAME, RENAMENX, RANDOMKEY, DBSIZE, FLUSHDB, OBJECT, DUMP, RESTORE, WAIT, OBJECT ENCODING.

use crate::db::{Db, glob_match};
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{Entry, Value, bytes_to_i64};

// ── DEL / UNLINK ─────────────────────────────────────────────────────────────

pub fn cmd_del(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("del"));
    }
    let count = args[1..].iter().filter(|k| db.remove(k)).count();
    Ok(Resp::Integer(count as i64))
}

pub fn cmd_unlink(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    // Same as DEL for our purposes (no async deletion)
    cmd_del(args, db)
}

// ── EXISTS ───────────────────────────────────────────────────────────────────

pub fn cmd_exists(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("exists"));
    }
    // Each key can be specified multiple times; each occurrence counts
    let count = args[1..].iter().filter(|k| db.exists(k)).count();
    Ok(Resp::Integer(count as i64))
}

// ── TYPE ─────────────────────────────────────────────────────────────────────

pub fn cmd_type(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("type"));
    }
    match db.get(&args[1]) {
        Some(e) => Ok(Resp::SimpleString(e.value.type_name().as_bytes().to_vec())),
        None => Ok(Resp::SimpleString(b"none".to_vec())),
    }
}

// ── RENAME / RENAMENX ────────────────────────────────────────────────────────

pub fn cmd_rename(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 {
        return Err(CacheError::wrong_arity("rename"));
    }
    let src = &args[1];
    let dst = args[2].clone();

    match db.keys.remove(src.as_slice()) {
        Some(entry) => {
            db.insert(dst, entry);
            Ok(Resp::ok())
        }
        None => Err(CacheError::generic("ERR no such key")),
    }
}

pub fn cmd_renamenx(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 {
        return Err(CacheError::wrong_arity("renamenx"));
    }
    let src = &args[1];
    let dst = &args[2];

    if !db.exists(src) {
        return Err(CacheError::generic("ERR no such key"));
    }
    if db.exists(dst) {
        return Ok(Resp::Integer(0));
    }

    let entry = db.keys.remove(src.as_slice()).unwrap();
    db.insert(dst.clone(), entry);
    Ok(Resp::Integer(1))
}

// ── KEYS ─────────────────────────────────────────────────────────────────────

pub fn cmd_keys(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("keys"));
    }
    let pattern = &args[1];
    let all = pattern == b"*";

    let keys: Vec<Resp> = db
        .keys
        .iter()
        .filter(|(_, e)| !e.is_expired())
        .filter(|(k, _)| all || glob_match(pattern, k))
        .map(|(k, _)| Resp::BulkString(Some(k.clone())))
        .collect();

    Ok(Resp::Array(Some(keys)))
}

// ── SCAN ─────────────────────────────────────────────────────────────────────

pub fn cmd_scan(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("scan"));
    }
    let cursor = bytes_to_i64(&args[1]).ok_or(CacheError::NotInteger)? as usize;

    let mut pattern: Option<&[u8]> = None;
    let mut count = 10usize;
    let mut type_filter: Option<&[u8]> = None;
    let mut i = 2;

    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"MATCH" => {
                i += 1;
                pattern = Some(&args[i]);
                i += 1;
            }
            b"COUNT" => {
                i += 1;
                count = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize;
                i += 1;
            }
            b"TYPE" => {
                i += 1;
                type_filter = Some(&args[i]);
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    // Collect all non-expired keys into a stable sorted list
    let mut all_keys: Vec<Vec<u8>> = db
        .keys
        .iter()
        .filter(|(_, e)| !e.is_expired())
        .filter(|(k, _)| pattern.map(|p| glob_match(p, k)).unwrap_or(true))
        .filter(|(_, e)| {
            type_filter
                .map(|t| e.value.type_name().as_bytes() == t)
                .unwrap_or(true)
        })
        .map(|(k, _)| k.clone())
        .collect();
    all_keys.sort();

    let total = all_keys.len();
    let start = cursor.min(total);
    let end = (cursor + count).min(total);
    let page: Vec<Resp> = all_keys[start..end]
        .iter()
        .map(|k| Resp::BulkString(Some(k.clone())))
        .collect();

    let next_cursor = if end >= total { 0 } else { end };

    Ok(Resp::Array(Some(vec![
        Resp::BulkString(Some(next_cursor.to_string().into_bytes())),
        Resp::Array(Some(page)),
    ])))
}

// ── SSCAN ────────────────────────────────────────────────────────────────────

pub fn cmd_sscan(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("sscan"));
    }
    let _cursor = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;

    let mut pattern: Option<&[u8]> = None;
    let mut count = 10usize;
    let mut i = 3;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"MATCH" => {
                i += 1;
                pattern = Some(&args[i]);
                i += 1;
            }
            b"COUNT" => {
                i += 1;
                count = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    match db.get_typed(&args[1], "set")? {
        Some(e) => match &e.value {
            Value::Set(set) => {
                let members: Vec<Resp> = set
                    .iter()
                    .filter(|m| pattern.map(|p| glob_match(p, m)).unwrap_or(true))
                    .take(count)
                    .map(|m| Resp::BulkString(Some(m.clone())))
                    .collect();
                Ok(Resp::Array(Some(vec![
                    Resp::BulkString(Some(b"0".to_vec())),
                    Resp::Array(Some(members)),
                ])))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![
            Resp::BulkString(Some(b"0".to_vec())),
            Resp::Array(Some(vec![])),
        ]))),
    }
}

// ── ZSCAN ────────────────────────────────────────────────────────────────────

pub fn cmd_zscan(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("zscan"));
    }
    let _cursor = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;

    let mut pattern: Option<&[u8]> = None;
    let mut count = 10usize;
    let mut i = 3;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"MATCH" => {
                i += 1;
                pattern = Some(&args[i]);
                i += 1;
            }
            b"COUNT" => {
                i += 1;
                count = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let mut items = Vec::new();
                for (member, score) in zset
                    .iter_asc()
                    .filter(|(m, _)| pattern.map(|p| glob_match(p, m)).unwrap_or(true))
                    .take(count)
                {
                    items.push(Resp::BulkString(Some(member.clone())));
                    items.push(Resp::BulkString(Some(crate::types::f64_to_bytes(score))));
                }
                Ok(Resp::Array(Some(vec![
                    Resp::BulkString(Some(b"0".to_vec())),
                    Resp::Array(Some(items)),
                ])))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![
            Resp::BulkString(Some(b"0".to_vec())),
            Resp::Array(Some(vec![])),
        ]))),
    }
}

// ── RANDOMKEY ────────────────────────────────────────────────────────────────

pub fn cmd_randomkey(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 1 {
        return Err(CacheError::wrong_arity("randomkey"));
    }
    let keys: Vec<&Vec<u8>> = db
        .keys
        .iter()
        .filter(|(_, e)| !e.is_expired())
        .map(|(k, _)| k)
        .collect();
    if keys.is_empty() {
        return Ok(Resp::nil());
    }
    let idx = rand::random::<usize>() % keys.len();
    Ok(Resp::BulkString(Some(keys[idx].clone())))
}

// ── DBSIZE ───────────────────────────────────────────────────────────────────

pub fn cmd_dbsize(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 1 {
        return Err(CacheError::wrong_arity("dbsize"));
    }
    let count = db.keys.iter().filter(|(_, e)| !e.is_expired()).count();
    Ok(Resp::Integer(count as i64))
}

// ── FLUSHDB ──────────────────────────────────────────────────────────────────

pub fn cmd_flushdb(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    // ASYNC/SYNC modifier is accepted but we always flush synchronously
    db.flush();
    Ok(Resp::ok())
}

// ── COPY ─────────────────────────────────────────────────────────────────────

pub fn cmd_copy(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("copy"));
    }
    let src = &args[1];
    let dst = args[2].clone();
    let replace = args.len() > 3 && args.iter().any(|a| a.to_ascii_uppercase() == b"REPLACE");

    if !replace && db.exists(&dst) {
        return Ok(Resp::Integer(0));
    }

    match db.keys.get(src.as_slice()).cloned() {
        Some(entry) => {
            if entry.is_expired() {
                db.keys.remove(src.as_slice());
                return Ok(Resp::Integer(0));
            }
            db.insert(dst, entry);
            Ok(Resp::Integer(1))
        }
        None => Ok(Resp::Integer(0)),
    }
}

// ── OBJECT ───────────────────────────────────────────────────────────────────

pub fn cmd_object(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("object"));
    }
    match args[1].to_ascii_uppercase().as_slice() {
        b"ENCODING" => {
            if args.len() != 3 {
                return Err(CacheError::wrong_arity("object encoding"));
            }
            match db.get(&args[2]) {
                Some(e) => {
                    let enc = encoding_for_value(&e.value, &args[2]);
                    Ok(Resp::BulkString(Some(enc.as_bytes().to_vec())))
                }
                None => Err(CacheError::generic("ERR no such key")),
            }
        }
        b"REFCOUNT" => {
            if args.len() != 3 {
                return Err(CacheError::wrong_arity("object refcount"));
            }
            if db.exists(&args[2]) {
                Ok(Resp::Integer(1))
            } else {
                Err(CacheError::generic("ERR no such key"))
            }
        }
        b"IDLETIME" => {
            if args.len() != 3 {
                return Err(CacheError::wrong_arity("object idletime"));
            }
            match db.get(&args[2]) {
                Some(e) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    Ok(Resp::Integer((now - e.lru_clock) as i64))
                }
                None => Err(CacheError::generic("ERR no such key")),
            }
        }
        b"FREQ" => {
            if args.len() != 3 {
                return Err(CacheError::wrong_arity("object freq"));
            }
            match db.get(&args[2]) {
                Some(e) => Ok(Resp::Integer(e.lfu_freq as i64)),
                None => Err(CacheError::generic("ERR no such key")),
            }
        }
        b"HELP" => Ok(Resp::Array(Some(vec![
            Resp::BulkString(Some(
                b"OBJECT <subcommand> [<arg> [value] [opt] ...]. Subcommands are:".to_vec(),
            )),
            Resp::BulkString(Some(b"ENCODING <key>".to_vec())),
            Resp::BulkString(Some(b"FREQ <key>".to_vec())),
            Resp::BulkString(Some(b"HELP".to_vec())),
            Resp::BulkString(Some(b"IDLETIME <key>".to_vec())),
            Resp::BulkString(Some(b"REFCOUNT <key>".to_vec())),
        ]))),
        _ => Err(CacheError::generic(format!(
            "ERR unknown subcommand '{}'. Try OBJECT HELP.",
            std::str::from_utf8(&args[1]).unwrap_or("?")
        ))),
    }
}

fn encoding_for_value(v: &Value, _key: &[u8]) -> &'static str {
    match v {
        Value::String(s) => {
            if s.len() <= 44 {
                "embstr"
            } else {
                "raw"
            }
        }
        Value::List(l) => {
            if l.len() <= 128 {
                "listpack"
            } else {
                "quicklist"
            }
        }
        Value::Set(s) => {
            if s.len() <= 128 {
                "listpack"
            } else {
                "hashtable"
            }
        }
        Value::ZSet(z) => {
            if z.len() <= 128 {
                "listpack"
            } else {
                "skiplist"
            }
        }
        Value::Hash(h) => {
            if h.len() <= 128 {
                "listpack"
            } else {
                "hashtable"
            }
        }
        Value::Stream(_) => "stream",
    }
}

// ── WAIT ─────────────────────────────────────────────────────────────────────

pub fn cmd_wait(_args: &[Vec<u8>], _db: &mut Db) -> CacheResult<Resp> {
    // In standalone mode, always returns 0 (no replicas)
    Ok(Resp::Integer(0))
}

// ── DUMP / RESTORE ───────────────────────────────────────────────────────────

pub fn cmd_dump(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 {
        return Err(CacheError::wrong_arity("dump"));
    }
    match db.get(&args[1]) {
        None => Ok(Resp::nil()),
        Some(e) => {
            // Return a simplified serialization (not full RDB format)
            let serialized = format!("CAVE:{}:{:?}", e.value.type_name(), args[1]);
            Ok(Resp::BulkString(Some(serialized.into_bytes())))
        }
    }
}

pub fn cmd_restore(args: &[Vec<u8>], _db: &mut Db) -> CacheResult<Resp> {
    // Simplified: we don't actually restore, just acknowledge
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("restore"));
    }
    Ok(Resp::ok())
}
