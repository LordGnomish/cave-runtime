// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Set commands: SADD, SREM, SMEMBERS, SISMEMBER, SMISMEMBER, SCARD, SPOP, SRANDMEMBER,
//! SDIFF, SINTER, SUNION, SDIFFSTORE, SINTERSTORE, SUNIONSTORE, SMOVE.

use std::collections::HashSet;

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{bytes_to_i64, Entry, Value};

fn get_set<'a>(db: &'a mut Db, key: &[u8]) -> CacheResult<Option<&'a HashSet<Vec<u8>>>> {
    match db.get_typed(key, "set")? {
        Some(e) => match &e.value {
            Value::Set(s) => Ok(Some(unsafe {
                // SAFETY: We hold a mutable borrow of db and we immediately use this ref.
                // This is needed because Rust's borrow checker can't infer the lifetime correctly
                // through the match chain. In practice this is safe as we don't mutate db here.
                &*(s as *const HashSet<Vec<u8>>)
            })),
            _ => unreachable!(),
        },
        None => Ok(None),
    }
}

pub fn cmd_sadd(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("sadd")); }
    let key = args[1].clone();

    match db.get_typed_mut(&key, "set")? {
        Some(entry) => match &mut entry.value {
            Value::Set(set) => {
                let mut added = 0i64;
                for member in &args[2..] {
                    if set.insert(member.clone()) { added += 1; }
                }
                Ok(Resp::Integer(added))
            }
            _ => unreachable!(),
        },
        None => {
            let mut set = HashSet::new();
            let mut added = 0i64;
            for member in &args[2..] {
                if set.insert(member.clone()) { added += 1; }
            }
            db.insert(key, Entry::new(Value::Set(set)));
            Ok(Resp::Integer(added))
        }
    }
}

pub fn cmd_srem(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("srem")); }
    let key = &args[1];
    match db.get_typed_mut(key, "set")? {
        Some(entry) => match &mut entry.value {
            Value::Set(set) => {
                let mut removed = 0i64;
                for member in &args[2..] {
                    if set.remove(member.as_slice()) { removed += 1; }
                }
                let is_empty = set.is_empty();
                if is_empty { db.remove(key); }
                Ok(Resp::Integer(removed))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_smembers(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("smembers")); }
    match db.get_typed(&args[1], "set")? {
        Some(e) => match &e.value {
            Value::Set(set) => {
                let members: Vec<Resp> = set.iter().map(|m| Resp::BulkString(Some(m.clone()))).collect();
                Ok(Resp::Array(Some(members)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

pub fn cmd_sismember(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("sismember")); }
    match db.get_typed(&args[1], "set")? {
        Some(e) => match &e.value {
            Value::Set(set) => Ok(Resp::Integer(if set.contains(args[2].as_slice()) { 1 } else { 0 })),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_smismember(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("smismember")); }
    let set_opt = match db.get_typed(&args[1], "set")? {
        Some(e) => match &e.value {
            Value::Set(s) => Some(s.iter().map(|m| m.clone()).collect::<HashSet<_>>()),
            _ => unreachable!(),
        },
        None => None,
    };
    let results: Vec<Resp> = args[2..]
        .iter()
        .map(|m| {
            let is_member = set_opt.as_ref().map(|s| s.contains(m.as_slice())).unwrap_or(false);
            Resp::Integer(if is_member { 1 } else { 0 })
        })
        .collect();
    Ok(Resp::Array(Some(results)))
}

pub fn cmd_scard(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("scard")); }
    match db.get_typed(&args[1], "set")? {
        Some(e) => match &e.value {
            Value::Set(s) => Ok(Resp::Integer(s.len() as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

pub fn cmd_spop(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 || args.len() > 3 { return Err(CacheError::wrong_arity("spop")); }
    let count: Option<usize> = if args.len() == 3 {
        Some(bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)? as usize)
    } else {
        None
    };

    let key = &args[1];
    match db.get_typed_mut(key, "set")? {
        Some(entry) => match &mut entry.value {
            Value::Set(set) => {
                if let Some(n) = count {
                    let mut popped = Vec::new();
                    let available = set.len().min(n);
                    let members: Vec<Vec<u8>> = set.iter().take(available).cloned().collect();
                    for m in &members {
                        set.remove(m.as_slice());
                        popped.push(Resp::BulkString(Some(m.clone())));
                    }
                    let is_empty = set.is_empty();
                    if is_empty { db.remove(key); }
                    Ok(Resp::Array(Some(popped)))
                } else {
                    let member = set.iter().next().cloned();
                    if let Some(m) = member {
                        set.remove(&m);
                        let is_empty = set.is_empty();
                        if is_empty { db.remove(key); }
                        Ok(Resp::BulkString(Some(m)))
                    } else {
                        Ok(Resp::nil())
                    }
                }
            }
            _ => unreachable!(),
        },
        None => Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() }),
    }
}

pub fn cmd_srandmember(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 || args.len() > 3 { return Err(CacheError::wrong_arity("srandmember")); }
    let count: Option<i64> = if args.len() == 3 {
        Some(bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?)
    } else {
        None
    };

    match db.get_typed(&args[1], "set")? {
        Some(e) => match &e.value {
            Value::Set(set) => {
                let members: Vec<&Vec<u8>> = set.iter().collect();
                if members.is_empty() {
                    return Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() });
                }

                if let Some(n) = count {
                    if n >= 0 {
                        // Distinct members, up to n
                        let take = (n as usize).min(members.len());
                        let result: Vec<Resp> = members.iter().take(take)
                            .map(|m| Resp::BulkString(Some(m.to_vec())))
                            .collect();
                        Ok(Resp::Array(Some(result)))
                    } else {
                        // May repeat, |n| members
                        let take = (-n) as usize;
                        let result: Vec<Resp> = (0..take)
                            .map(|_| {
                                let idx = rand::random::<usize>() % members.len();
                                Resp::BulkString(Some(members[idx].to_vec()))
                            })
                            .collect();
                        Ok(Resp::Array(Some(result)))
                    }
                } else {
                    let idx = rand::random::<usize>() % members.len();
                    Ok(Resp::BulkString(Some(members[idx].to_vec())))
                }
            }
            _ => unreachable!(),
        },
        None => Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() }),
    }
}

// ── Set operations ────────────────────────────────────────────────────────────

fn collect_set(db: &mut Db, key: &[u8]) -> CacheResult<HashSet<Vec<u8>>> {
    match db.get_typed(key, "set")? {
        Some(e) => match &e.value {
            Value::Set(s) => Ok(s.clone()),
            _ => unreachable!(),
        },
        None => Ok(HashSet::new()),
    }
}

pub fn cmd_sdiff(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("sdiff")); }
    let mut result = collect_set(db, &args[1])?;
    for key in &args[2..] {
        let other = collect_set(db, key)?;
        result.retain(|m| !other.contains(m.as_slice()));
    }
    Ok(Resp::Array(Some(result.into_iter().map(|m| Resp::BulkString(Some(m))).collect())))
}

pub fn cmd_sinter(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("sinter")); }
    let mut result = collect_set(db, &args[1])?;
    for key in &args[2..] {
        let other = collect_set(db, key)?;
        result.retain(|m| other.contains(m.as_slice()));
    }
    Ok(Resp::Array(Some(result.into_iter().map(|m| Resp::BulkString(Some(m))).collect())))
}

pub fn cmd_sunion(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("sunion")); }
    let mut result: HashSet<Vec<u8>> = HashSet::new();
    for key in &args[1..] {
        result.extend(collect_set(db, key)?);
    }
    Ok(Resp::Array(Some(result.into_iter().map(|m| Resp::BulkString(Some(m))).collect())))
}

pub fn cmd_sdiffstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("sdiffstore")); }
    let dst = args[1].clone();
    let mut result = collect_set(db, &args[2])?;
    for key in &args[3..] {
        let other = collect_set(db, key)?;
        result.retain(|m| !other.contains(m.as_slice()));
    }
    let len = result.len() as i64;
    db.insert(dst, Entry::new(Value::Set(result)));
    Ok(Resp::Integer(len))
}

pub fn cmd_sinterstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("sinterstore")); }
    let dst = args[1].clone();
    let mut result = collect_set(db, &args[2])?;
    for key in &args[3..] {
        let other = collect_set(db, key)?;
        result.retain(|m| other.contains(m.as_slice()));
    }
    let len = result.len() as i64;
    db.insert(dst, Entry::new(Value::Set(result)));
    Ok(Resp::Integer(len))
}

pub fn cmd_sunionstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("sunionstore")); }
    let dst = args[1].clone();
    let mut result: HashSet<Vec<u8>> = HashSet::new();
    for key in &args[2..] {
        result.extend(collect_set(db, key)?);
    }
    let len = result.len() as i64;
    db.insert(dst, Entry::new(Value::Set(result)));
    Ok(Resp::Integer(len))
}

pub fn cmd_smove(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("smove")); }
    let src = args[1].clone();
    let dst = args[2].clone();
    let member = args[3].clone();

    // Check source exists and has member
    let exists = match db.get_typed_mut(&src, "set")? {
        Some(entry) => match &mut entry.value {
            Value::Set(set) => set.remove(member.as_slice()),
            _ => unreachable!(),
        },
        None => return Ok(Resp::Integer(0)),
    };

    if !exists {
        return Ok(Resp::Integer(0));
    }

    // Remove empty source
    if let Some(e) = db.keys.get(&src) {
        if let Value::Set(s) = &e.value { if s.is_empty() { db.remove(&src); } }
    }

    // Add to destination
    match db.get_typed_mut(&dst, "set")? {
        Some(entry) => match &mut entry.value {
            Value::Set(set) => { set.insert(member); }
            _ => return Err(CacheError::WrongType),
        },
        None => {
            let mut set = HashSet::new();
            set.insert(member);
            db.insert(dst, Entry::new(Value::Set(set)));
        }
    }

    Ok(Resp::Integer(1))
}

// ── SINTERCARD ───────────────────────────────────────────────────────────────

pub fn cmd_sintercard(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("sintercard")); }
    let numkeys = bytes_to_i64(&args[1]).ok_or(CacheError::NotInteger)? as usize;
    if numkeys == 0 { return Err(CacheError::generic("ERR Number of keys can't be non-positive")); }

    let mut limit = 0usize;
    let key_end = 1 + numkeys + 1;
    if args.len() > key_end {
        if args[key_end].to_ascii_uppercase() == b"LIMIT" && args.len() > key_end + 1 {
            limit = bytes_to_i64(&args[key_end + 1]).ok_or(CacheError::NotInteger)? as usize;
        }
    }

    let mut result = collect_set(db, &args[2])?;
    for key in &args[3..2 + numkeys] {
        let other = collect_set(db, key)?;
        result.retain(|m| other.contains(m.as_slice()));
    }

    let count = if limit > 0 { result.len().min(limit) } else { result.len() };
    Ok(Resp::Integer(count as i64))
}
