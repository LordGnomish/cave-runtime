// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hash commands: HSET, HGET, HMSET, HMGET, HDEL, HEXISTS, HLEN, HKEYS, HVALS, HGETALL,
//! HINCRBY, HINCRBYFLOAT, HSETNX, HRANDFIELD, HSCAN.

use std::collections::HashMap;

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{bytes_to_f64, bytes_to_i64, f64_to_bytes, i64_to_bytes, Entry, Value};

pub fn cmd_hset(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 || (args.len() - 2) % 2 != 0 {
        return Err(CacheError::wrong_arity("hset"));
    }
    let key = args[1].clone();
    let mut added = 0i64;

    match db.get_typed_mut(&key, "hash")? {
        Some(entry) => match &mut entry.value {
            Value::Hash(hash) => {
                let mut i = 2;
                while i < args.len() {
                    let field = args[i].clone();
                    let val = args[i + 1].clone();
                    if !hash.contains_key(&field) { added += 1; }
                    hash.insert(field, val);
                    i += 2;
                }
            }
            _ => unreachable!(),
        },
        None => {
            let mut hash = HashMap::new();
            let mut i = 2;
            while i < args.len() {
                hash.insert(args[i].clone(), args[i + 1].clone());
                added += 1;
                i += 2;
            }
            db.insert(key, Entry::new(Value::Hash(hash)));
        }
    }
    Ok(Resp::Integer(added))
}

pub fn cmd_hmset(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 || (args.len() - 2) % 2 != 0 {
        return Err(CacheError::wrong_arity("hmset"));
    }
    cmd_hset(args, db)?;
    Ok(Resp::ok())
}

pub fn cmd_hget(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("hget")); }
    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(hash) => Ok(hash.get(args[2].as_slice()).cloned()
                .map(|v| Resp::BulkString(Some(v)))
                .unwrap_or(Resp::nil())),
            _ => unreachable!(),
        },
        None => Ok(Resp::nil()),
    }
}

pub fn cmd_hmget(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("hmget")); }
    let hash_opt: Option<HashMap<Vec<u8>, Vec<u8>>> = match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(h) => Some(h.clone()),
            _ => unreachable!(),
        },
        None => None,
    };
    let results: Vec<Resp> = args[2..]
        .iter()
        .map(|field| {
            hash_opt
                .as_ref()
                .and_then(|h| h.get(field.as_slice()))
                .cloned()
                .map(|v| Resp::BulkString(Some(v)))
                .unwrap_or(Resp::nil())
        })
        .collect();
    Ok(Resp::Array(Some(results)))
}

pub fn cmd_hdel(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("hdel")); }
    let key = &args[1];
    match db.get_typed_mut(key, "hash")? {
        Some(entry) => match &mut entry.value {
            Value::Hash(hash) => {
                let mut deleted = 0i64;
                for field in &args[2..] {
                    if hash.remove(field.as_slice()).is_some() { deleted += 1; }
                }
                let is_empty = hash.is_empty();
                if is_empty { db.remove(key); }
                Ok(Resp::Integer(deleted))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_hexists(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("hexists")); }
    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(h) => Ok(Resp::Integer(if h.contains_key(args[2].as_slice()) { 1 } else { 0 })),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_hlen(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("hlen")); }
    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(h) => Ok(Resp::Integer(h.len() as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_hkeys(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("hkeys")); }
    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(h) => Ok(Resp::Array(Some(
                h.keys().map(|k| Resp::BulkString(Some(k.clone()))).collect()
            ))),
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

pub fn cmd_hvals(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("hvals")); }
    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(h) => Ok(Resp::Array(Some(
                h.values().map(|v| Resp::BulkString(Some(v.clone()))).collect()
            ))),
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

pub fn cmd_hgetall(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("hgetall")); }
    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(h) => {
                let mut items = Vec::with_capacity(h.len() * 2);
                for (k, v) in h {
                    items.push(Resp::BulkString(Some(k.clone())));
                    items.push(Resp::BulkString(Some(v.clone())));
                }
                Ok(Resp::Array(Some(items)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

pub fn cmd_hincrby(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("hincrby")); }
    let key = args[1].clone();
    let field = args[2].clone();
    let delta = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;

    let new_val = match db.get_typed_mut(&key, "hash")? {
        Some(entry) => match &mut entry.value {
            Value::Hash(hash) => {
                let current = hash.get(field.as_slice())
                    .map(|v| bytes_to_i64(v).ok_or(CacheError::NotInteger))
                    .unwrap_or(Ok(0))?;
                let new_val = current.checked_add(delta)
                    .ok_or_else(|| CacheError::generic("ERR increment or decrement would overflow"))?;
                hash.insert(field, i64_to_bytes(new_val));
                new_val
            }
            _ => unreachable!(),
        },
        None => {
            let mut hash = HashMap::new();
            hash.insert(field, i64_to_bytes(delta));
            db.insert(key, Entry::new(Value::Hash(hash)));
            delta
        }
    };
    Ok(Resp::Integer(new_val))
}

pub fn cmd_hincrbyfloat(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("hincrbyfloat")); }
    let key = args[1].clone();
    let field = args[2].clone();
    let delta = bytes_to_f64(&args[3]).ok_or(CacheError::NotFloat)?;

    let new_val = match db.get_typed_mut(&key, "hash")? {
        Some(entry) => match &mut entry.value {
            Value::Hash(hash) => {
                let current = hash.get(field.as_slice())
                    .map(|v| bytes_to_f64(v).ok_or(CacheError::NotFloat))
                    .unwrap_or(Ok(0.0))?;
                let new_val = current + delta;
                let s = format!("{}", new_val);
                hash.insert(field, s.into_bytes());
                new_val
            }
            _ => unreachable!(),
        },
        None => {
            let mut hash = HashMap::new();
            let s = format!("{}", delta);
            hash.insert(field, s.into_bytes());
            db.insert(key, Entry::new(Value::Hash(hash)));
            delta
        }
    };
    Ok(Resp::BulkString(Some(format!("{}", new_val).into_bytes())))
}

pub fn cmd_hsetnx(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("hsetnx")); }
    let key = args[1].clone();
    let field = args[2].clone();
    let val = args[3].clone();

    match db.get_typed_mut(&key, "hash")? {
        Some(entry) => match &mut entry.value {
            Value::Hash(hash) => {
                if hash.contains_key(field.as_slice()) {
                    Ok(Resp::Integer(0))
                } else {
                    hash.insert(field, val);
                    Ok(Resp::Integer(1))
                }
            }
            _ => unreachable!(),
        },
        None => {
            let mut hash = HashMap::new();
            hash.insert(field, val);
            db.insert(key, Entry::new(Value::Hash(hash)));
            Ok(Resp::Integer(1))
        }
    }
}

pub fn cmd_hrandfield(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("hrandfield")); }
    let count: Option<i64> = if args.len() >= 3 {
        Some(bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?)
    } else {
        None
    };
    let withvalues = args.len() >= 4 && args[3].to_ascii_uppercase() == b"WITHVALUES";

    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(hash) => {
                let fields: Vec<(&Vec<u8>, &Vec<u8>)> = hash.iter().collect();
                if fields.is_empty() {
                    return Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() });
                }
                if let Some(n) = count {
                    let items: Vec<Resp> = if n >= 0 {
                        fields.iter().take(n as usize).flat_map(|(f, v)| {
                            let mut r = vec![Resp::BulkString(Some(f.to_vec()))];
                            if withvalues { r.push(Resp::BulkString(Some(v.to_vec()))); }
                            r
                        }).collect()
                    } else {
                        let take = (-n) as usize;
                        (0..take).flat_map(|_| {
                            let idx = rand::random::<usize>() % fields.len();
                            let (f, v) = fields[idx];
                            let mut r = vec![Resp::BulkString(Some(f.to_vec()))];
                            if withvalues { r.push(Resp::BulkString(Some(v.to_vec()))); }
                            r
                        }).collect()
                    };
                    Ok(Resp::Array(Some(items)))
                } else {
                    let idx = rand::random::<usize>() % fields.len();
                    Ok(Resp::BulkString(Some(fields[idx].0.clone())))
                }
            }
            _ => unreachable!(),
        },
        None => Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() }),
    }
}

pub fn cmd_hscan(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("hscan")); }
    let _cursor = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;

    let mut pattern: Option<&[u8]> = None;
    let mut count = 10usize;
    let mut i = 3;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"MATCH" => { pattern = Some(&args[i + 1]); i += 2; }
            b"COUNT" => { count = bytes_to_i64(&args[i + 1]).ok_or(CacheError::NotInteger)? as usize; i += 2; }
            _ => { i += 1; }
        }
    }

    match db.get_typed(&args[1], "hash")? {
        Some(e) => match &e.value {
            Value::Hash(hash) => {
                let mut items = Vec::new();
                for (k, v) in hash.iter().take(count) {
                    if let Some(pat) = pattern {
                        if !crate::db::glob_match(pat, k) { continue; }
                    }
                    items.push(Resp::BulkString(Some(k.clone())));
                    items.push(Resp::BulkString(Some(v.clone())));
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
