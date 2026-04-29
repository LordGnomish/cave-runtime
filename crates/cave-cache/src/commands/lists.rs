//! List commands: LPUSH, RPUSH, LPOP, RPOP, LLEN, LRANGE, LINDEX, LSET, LINSERT, LREM,
//! LTRIM, LPOS, LMOVE, BLPOP, BRPOP, BLMOVE.

use std::collections::VecDeque;

use tokio::sync::mpsc;

use crate::db::{BlockedPop, Db};
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{bytes_to_f64, bytes_to_i64, normalize_index, Entry, Value};

// ── LPUSH / RPUSH / LPUSHX / RPUSHX ─────────────────────────────────────────

pub fn cmd_lpush(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    push_impl(args, db, true, false)
}

pub fn cmd_rpush(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    push_impl(args, db, false, false)
}

pub fn cmd_lpushx(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    push_impl(args, db, true, true)
}

pub fn cmd_rpushx(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    push_impl(args, db, false, true)
}

fn push_impl(args: &[Vec<u8>], db: &mut Db, left: bool, nx: bool) -> CacheResult<Resp> {
    let cmd = if left { if nx { "lpushx" } else { "lpush" } } else { if nx { "rpushx" } else { "rpush" } };
    if args.len() < 3 { return Err(CacheError::wrong_arity(cmd)); }

    let key = args[1].clone();

    if nx && !db.exists(&key) {
        return Ok(Resp::Integer(0));
    }

    match db.get_typed_mut(&key, "list")? {
        Some(entry) => {
            if let Value::List(list) = &mut entry.value {
                for val in &args[2..] {
                    if left { list.push_front(val.clone()); }
                    else { list.push_back(val.clone()); }
                }
                let len = list.len() as i64;
                Ok(Resp::Integer(len))
            } else {
                unreachable!()
            }
        }
        None => {
            let mut list = VecDeque::new();
            for val in &args[2..] {
                if left { list.push_front(val.clone()); }
                else { list.push_back(val.clone()); }
            }
            let len = list.len() as i64;
            db.insert(key, Entry::new(Value::List(list)));
            Ok(Resp::Integer(len))
        }
    }
}

// ── LPOP / RPOP ──────────────────────────────────────────────────────────────

pub fn cmd_lpop(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    pop_impl(args, db, true)
}

pub fn cmd_rpop(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    pop_impl(args, db, false)
}

fn pop_impl(args: &[Vec<u8>], db: &mut Db, left: bool) -> CacheResult<Resp> {
    let cmd = if left { "lpop" } else { "rpop" };
    if args.len() < 2 || args.len() > 3 { return Err(CacheError::wrong_arity(cmd)); }

    let count: Option<usize> = if args.len() == 3 {
        Some(bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)? as usize)
    } else {
        None
    };

    let key = &args[1];

    match db.get_typed_mut(key, "list")? {
        Some(entry) => {
            if let Value::List(list) = &mut entry.value {
                if let Some(n) = count {
                    let mut results = Vec::new();
                    for _ in 0..n {
                        let v = if left { list.pop_front() } else { list.pop_back() };
                        match v {
                            Some(v) => results.push(Resp::BulkString(Some(v))),
                            None => break,
                        }
                    }
                    if list.is_empty() {
                        db.remove(key);
                    }
                    Ok(Resp::Array(Some(results)))
                } else {
                    let v = if left { list.pop_front() } else { list.pop_back() };
                    let is_empty = list.is_empty();
                    if is_empty { db.remove(key); }
                    Ok(v.map(|v| Resp::BulkString(Some(v))).unwrap_or(Resp::nil()))
                }
            } else {
                unreachable!()
            }
        }
        None => {
            if count.is_some() {
                Ok(Resp::nil_array())
            } else {
                Ok(Resp::nil())
            }
        }
    }
}

// ── LLEN ─────────────────────────────────────────────────────────────────────

pub fn cmd_llen(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("llen")); }
    match db.get_typed(&args[1], "list")? {
        Some(e) => match &e.value {
            Value::List(l) => Ok(Resp::Integer(l.len() as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── LRANGE ───────────────────────────────────────────────────────────────────

pub fn cmd_lrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("lrange")); }
    let start = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let stop = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;

    match db.get_typed(&args[1], "list")? {
        Some(e) => match &e.value {
            Value::List(list) => {
                let len = list.len() as isize;
                let start = normalize_index(start as isize, len);
                let stop = normalize_index(stop as isize, len);
                if start > stop || start >= list.len() {
                    return Ok(Resp::Array(Some(vec![])));
                }
                let stop = stop.min(list.len() - 1);
                let items: Vec<Resp> = list
                    .iter()
                    .skip(start)
                    .take(stop - start + 1)
                    .map(|v| Resp::BulkString(Some(v.clone())))
                    .collect();
                Ok(Resp::Array(Some(items)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── LINDEX ───────────────────────────────────────────────────────────────────

pub fn cmd_lindex(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("lindex")); }
    let index = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;

    match db.get_typed(&args[1], "list")? {
        Some(e) => match &e.value {
            Value::List(list) => {
                let len = list.len() as isize;
                let idx = normalize_index(index as isize, len);
                if idx >= list.len() {
                    return Ok(Resp::nil());
                }
                Ok(Resp::BulkString(list.get(idx).cloned().map(Some).unwrap_or(None)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::nil()),
    }
}

// ── LSET ─────────────────────────────────────────────────────────────────────

pub fn cmd_lset(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("lset")); }
    let index = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let value = args[3].clone();

    match db.get_typed_mut(&args[1], "list")? {
        Some(entry) => match &mut entry.value {
            Value::List(list) => {
                let len = list.len() as isize;
                let idx = normalize_index(index as isize, len);
                if idx >= list.len() {
                    return Err(CacheError::OutOfRange);
                }
                list[idx] = value;
                Ok(Resp::ok())
            }
            _ => unreachable!(),
        },
        None => Err(CacheError::generic("ERR no such key")),
    }
}

// ── LINSERT ──────────────────────────────────────────────────────────────────

pub fn cmd_linsert(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 5 { return Err(CacheError::wrong_arity("linsert")); }
    let before = match args[2].to_ascii_uppercase().as_slice() {
        b"BEFORE" => true,
        b"AFTER" => false,
        _ => return Err(CacheError::Syntax),
    };
    let pivot = &args[3];
    let element = args[4].clone();

    match db.get_typed_mut(&args[1], "list")? {
        Some(entry) => match &mut entry.value {
            Value::List(list) => {
                let pos = list.iter().position(|v| v.as_slice() == pivot.as_slice());
                match pos {
                    None => Ok(Resp::Integer(-1)),
                    Some(idx) => {
                        let insert_at = if before { idx } else { idx + 1 };
                        list.insert(insert_at, element);
                        Ok(Resp::Integer(list.len() as i64))
                    }
                }
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── LREM ─────────────────────────────────────────────────────────────────────

pub fn cmd_lrem(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("lrem")); }
    let count = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let element = &args[3];

    match db.get_typed_mut(&args[1], "list")? {
        Some(entry) => match &mut entry.value {
            Value::List(list) => {
                let mut removed = 0i64;
                if count > 0 {
                    // Remove first N occurrences (head to tail)
                    let mut i = 0;
                    while i < list.len() && (count == 0 || removed < count) {
                        if list[i] == *element {
                            list.remove(i);
                            removed += 1;
                        } else {
                            i += 1;
                        }
                    }
                } else if count < 0 {
                    // Remove last N occurrences (tail to head)
                    let target = (-count) as usize;
                    let mut i = list.len();
                    while i > 0 && (removed as usize) < target {
                        i -= 1;
                        if list[i] == *element {
                            list.remove(i);
                            removed += 1;
                        }
                    }
                } else {
                    // Remove all occurrences
                    let before = list.len();
                    list.retain(|v| v.as_slice() != element.as_slice());
                    removed = (before - list.len()) as i64;
                }
                let is_empty = list.is_empty();
                if is_empty { db.remove(&args[1]); }
                Ok(Resp::Integer(removed))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── LTRIM ────────────────────────────────────────────────────────────────────

pub fn cmd_ltrim(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("ltrim")); }
    let start = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let stop = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;

    match db.get_typed_mut(&args[1], "list")? {
        Some(entry) => match &mut entry.value {
            Value::List(list) => {
                let len = list.len() as isize;
                let start = normalize_index(start as isize, len);
                let stop = normalize_index(stop as isize, len).min(list.len().saturating_sub(1));

                if start > stop || start >= list.len() {
                    list.clear();
                } else {
                    let trimmed: VecDeque<Vec<u8>> = list.iter().skip(start).take(stop - start + 1).cloned().collect();
                    *list = trimmed;
                }
                if list.is_empty() { db.remove(&args[1]); }
                Ok(Resp::ok())
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::ok()),
    }
}

// ── LPOS ─────────────────────────────────────────────────────────────────────

pub fn cmd_lpos(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("lpos")); }
    let element = &args[2];
    let mut rank: i64 = 1;
    let mut count: Option<usize> = None;
    let mut maxlen: usize = 0;

    let mut i = 3;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"RANK" => { i += 1; rank = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)?; }
            b"COUNT" => { i += 1; count = Some(bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize); }
            b"MAXLEN" => { i += 1; maxlen = bytes_to_i64(&args[i]).ok_or(CacheError::NotInteger)? as usize; }
            _ => return Err(CacheError::Syntax),
        }
        i += 1;
    }

    match db.get_typed(&args[1], "list")? {
        Some(e) => match &e.value {
            Value::List(list) => {
                let len = if maxlen == 0 { list.len() } else { maxlen.min(list.len()) };
                let mut positions = Vec::new();
                let mut match_count = 0i64;

                let iter: Box<dyn Iterator<Item = (usize, &Vec<u8>)>> = if rank < 0 {
                    Box::new(list.iter().enumerate().rev().take(len))
                } else {
                    Box::new(list.iter().enumerate().take(len))
                };

                for (idx, val) in iter {
                    if val.as_slice() == element.as_slice() {
                        match_count += 1;
                        if match_count.abs() >= rank.abs() {
                            positions.push(idx as i64);
                            if let Some(c) = count {
                                if positions.len() >= c { break; }
                            } else {
                                break;
                            }
                        }
                    }
                }

                if count.is_some() {
                    Ok(Resp::Array(Some(positions.into_iter().map(Resp::Integer).collect())))
                } else {
                    Ok(positions.first().map(|&p| Resp::Integer(p)).unwrap_or(Resp::nil()))
                }
            }
            _ => unreachable!(),
        },
        None => Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() }),
    }
}

// ── LMOVE ────────────────────────────────────────────────────────────────────

pub fn cmd_lmove(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 5 { return Err(CacheError::wrong_arity("lmove")); }
    let src = args[1].clone();
    let dst = args[2].clone();
    let from_left = match args[3].to_ascii_uppercase().as_slice() {
        b"LEFT" => true,
        b"RIGHT" => false,
        _ => return Err(CacheError::Syntax),
    };
    let to_left = match args[4].to_ascii_uppercase().as_slice() {
        b"LEFT" => true,
        b"RIGHT" => false,
        _ => return Err(CacheError::Syntax),
    };

    // Pop from source
    let element = match db.get_typed_mut(&src, "list")? {
        Some(entry) => match &mut entry.value {
            Value::List(list) => {
                let v = if from_left { list.pop_front() } else { list.pop_back() };
                let is_empty = list.is_empty();
                let v = v.ok_or(CacheError::Empty)?;
                if is_empty { db.remove(&src); }
                v
            }
            _ => unreachable!(),
        },
        None => return Ok(Resp::nil()),
    };

    // Push to destination
    match db.get_typed_mut(&dst, "list")? {
        Some(entry) => match &mut entry.value {
            Value::List(list) => {
                if to_left { list.push_front(element.clone()); }
                else { list.push_back(element.clone()); }
            }
            _ => return Err(CacheError::WrongType),
        },
        None => {
            let mut list = VecDeque::new();
            if to_left { list.push_front(element.clone()); }
            else { list.push_back(element.clone()); }
            db.insert(dst, Entry::new(Value::List(list)));
        }
    }

    Ok(Resp::BulkString(Some(element)))
}

// ── BLPOP / BRPOP (non-blocking path — blocking handled in server.rs) ─────────

/// Try immediate pop; if list is empty, register a waiter and return None.
/// The server layer handles awaiting the waiter channel.
pub async fn cmd_blpop(
    args: &[Vec<u8>],
    db: &mut Db,
    timeout: f64,
) -> CacheResult<Option<(Vec<u8>, Vec<u8>)>> {
    blpop_impl(args, db, true, timeout).await
}

pub async fn cmd_brpop(
    args: &[Vec<u8>],
    db: &mut Db,
    timeout: f64,
) -> CacheResult<Option<(Vec<u8>, Vec<u8>)>> {
    blpop_impl(args, db, false, timeout).await
}

async fn blpop_impl(
    args: &[Vec<u8>],
    db: &mut Db,
    left: bool,
    _timeout: f64,
) -> CacheResult<Option<(Vec<u8>, Vec<u8>)>> {
    let cmd = if left { "blpop" } else { "brpop" };
    if args.len() < 3 { return Err(CacheError::wrong_arity(cmd)); }

    let keys = &args[1..args.len() - 1];

    // Try immediate pop
    for key in keys {
        match db.get_typed_mut(key, "list")? {
            Some(entry) => match &mut entry.value {
                Value::List(list) => {
                    let v = if left { list.pop_front() } else { list.pop_back() };
                    if let Some(v) = v {
                        let is_empty = list.is_empty();
                        if is_empty { db.remove(key); }
                        return Ok(Some((key.clone(), v)));
                    }
                }
                _ => unreachable!(),
            },
            None => {}
        }
    }

    // If timeout == 0 or keys all empty: register waiters
    // In a real server the connection handler awaits; here we just return None
    Ok(None)
}

/// Synchronous (non-blocking) try-pop for BLPOP/BRPOP used inside execute_db_command.
/// Returns Some((key, value)) if an element was immediately available, None otherwise.
pub fn blpop_impl_sync(args: &[Vec<u8>], db: &mut Db, left: bool) -> CacheResult<Option<(Vec<u8>, Vec<u8>)>> {
    let cmd = if left { "blpop" } else { "brpop" };
    if args.len() < 3 { return Err(CacheError::wrong_arity(cmd)); }

    // Keys are all args except the last (timeout)
    let keys = &args[1..args.len() - 1];
    for key in keys {
        match db.get_typed_mut(key, "list")? {
            Some(entry) => match &mut entry.value {
                Value::List(list) => {
                    let v = if left { list.pop_front() } else { list.pop_back() };
                    if let Some(v) = v {
                        let is_empty = list.is_empty();
                        if is_empty { db.remove(key); }
                        return Ok(Some((key.clone(), v)));
                    }
                }
                _ => unreachable!(),
            },
            None => {}
        }
    }
    Ok(None)
}

/// Register blocking waiters for BLPOP/BRPOP.
pub fn register_blocked_pop(db: &mut Db, keys: &[Vec<u8>], tx: mpsc::Sender<(Vec<u8>, Vec<u8>)>, from_right: bool) {
    for key in keys {
        db.blocked_pops
            .entry(key.clone())
            .or_default()
            .push(BlockedPop { tx: tx.clone(), from_right });
    }
}

/// Check and wake blocked clients after a push. Returns how many were woken.
pub fn wake_blocked(db: &mut Db, key: &[u8]) -> usize {
    let waiters = match db.blocked_pops.get_mut(key) {
        Some(w) => w,
        None => return 0,
    };

    if waiters.is_empty() {
        return 0;
    }

    let waiter = waiters.remove(0);
    if waiters.is_empty() {
        db.blocked_pops.remove(key);
    }

    // Try to pop the value
    if let Some(entry) = db.keys.get_mut(key) {
        if let Value::List(list) = &mut entry.value {
            let v = if waiter.from_right { list.pop_back() } else { list.pop_front() };
            if let Some(v) = v {
                if list.is_empty() {
                    db.keys.remove(key);
                }
                let _ = waiter.tx.try_send((key.to_vec(), v));
                return 1;
            }
        }
    }
    0
}

// ── RPOPLPUSH (deprecated, kept for backcompat) ─────────────────────────────

/// RPOPLPUSH source destination — equivalent to LMOVE source destination RIGHT LEFT.
pub fn cmd_rpoplpush(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("rpoplpush")); }
    let lmove_args: Vec<Vec<u8>> = vec![
        b"LMOVE".to_vec(),
        args[1].clone(),
        args[2].clone(),
        b"RIGHT".to_vec(),
        b"LEFT".to_vec(),
    ];
    cmd_lmove(&lmove_args, db)
}

// ── SORT / SORT_RO ──────────────────────────────────────────────────────────

/// SORT key [ASC|DESC] [ALPHA] [LIMIT off cnt] [STORE dst]
///
/// Implements the most-common subset: numeric/alpha sort over a list/set/zset
/// with optional ASC/DESC, LIMIT, and STORE. Does NOT support BY/GET patterns.
pub fn cmd_sort(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("sort")); }
    let key = &args[1];

    let mut alpha = false;
    let mut desc = false;
    let mut offset: usize = 0;
    let mut count: usize = usize::MAX;
    let mut store: Option<Vec<u8>> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"ASC" => { desc = false; i += 1; }
            b"DESC" => { desc = true; i += 1; }
            b"ALPHA" => { alpha = true; i += 1; }
            b"LIMIT" => {
                if i + 2 >= args.len() { return Err(CacheError::Syntax); }
                let off = bytes_to_i64(&args[i + 1]).ok_or(CacheError::NotInteger)?;
                let cnt = bytes_to_i64(&args[i + 2]).ok_or(CacheError::NotInteger)?;
                offset = if off < 0 { 0 } else { off as usize };
                count = if cnt < 0 { usize::MAX } else { cnt as usize };
                i += 3;
            }
            b"STORE" => {
                if i + 1 >= args.len() { return Err(CacheError::Syntax); }
                store = Some(args[i + 1].clone());
                i += 2;
            }
            // BY / GET are accepted syntactically but treated as identity (no key lookup).
            b"BY" | b"GET" => {
                if i + 1 >= args.len() { return Err(CacheError::Syntax); }
                i += 2;
            }
            _ => i += 1,
        }
    }

    // Collect input elements
    let elements: Vec<Vec<u8>> = match db.get(key) {
        Some(e) => match &e.value {
            Value::List(l) => l.iter().cloned().collect(),
            Value::Set(s) => s.iter().cloned().collect(),
            Value::ZSet(z) => z.iter_asc().map(|(m, _)| m.clone()).collect(),
            Value::String(_) | Value::Hash(_) | Value::Stream(_) => {
                return Err(CacheError::WrongType);
            }
        },
        None => vec![],
    };

    // Sort
    let mut sorted: Vec<Vec<u8>> = elements;
    if alpha {
        sorted.sort();
    } else {
        // Numeric sort: parse each as f64; if any fails, return error like Redis.
        let mut parsed: Vec<(f64, Vec<u8>)> = Vec::with_capacity(sorted.len());
        for e in sorted.into_iter() {
            let n = bytes_to_f64(&e).ok_or_else(||
                CacheError::generic("ERR One or more scores can't be converted into double")
            )?;
            parsed.push((n, e));
        }
        parsed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        sorted = parsed.into_iter().map(|(_, v)| v).collect();
    }
    if desc { sorted.reverse(); }

    let result: Vec<Vec<u8>> = sorted.into_iter().skip(offset).take(count).collect();

    if let Some(dst) = store {
        let len = result.len() as i64;
        if result.is_empty() {
            db.remove(&dst);
        } else {
            let list: std::collections::VecDeque<Vec<u8>> = result.into_iter().collect();
            db.insert(dst, Entry::new(Value::List(list)));
        }
        Ok(Resp::Integer(len))
    } else {
        Ok(Resp::Array(Some(
            result.into_iter().map(|v| Resp::BulkString(Some(v))).collect()
        )))
    }
}

/// SORT_RO key ... — read-only variant; identical semantics minus STORE.
pub fn cmd_sort_ro(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("sort_ro")); }
    // Reject STORE clause
    for a in args.iter().skip(2) {
        if a.to_ascii_uppercase() == b"STORE" {
            return Err(CacheError::generic("ERR SORT_RO is read-only and cannot accept STORE"));
        }
    }
    cmd_sort(args, db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpoplpush_moves_right_to_left() {
        let mut db = Db::new();
        let mut l = std::collections::VecDeque::new();
        l.push_back(b"a".to_vec());
        l.push_back(b"b".to_vec());
        db.insert(b"src".to_vec(), Entry::new(Value::List(l)));
        let r = cmd_rpoplpush(
            &[b"RPOPLPUSH".to_vec(), b"src".to_vec(), b"dst".to_vec()],
            &mut db,
        ).unwrap();
        match r {
            Resp::BulkString(Some(b)) => assert_eq!(b, b"b"),
            _ => panic!(),
        }
        if let Some(e) = db.get(b"dst") {
            if let Value::List(l) = &e.value {
                assert_eq!(l.len(), 1);
                assert_eq!(&l[0], b"b");
            } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn sort_alpha_ascending() {
        let mut db = Db::new();
        let mut l = std::collections::VecDeque::new();
        l.push_back(b"banana".to_vec());
        l.push_back(b"apple".to_vec());
        l.push_back(b"cherry".to_vec());
        db.insert(b"k".to_vec(), Entry::new(Value::List(l)));

        let r = cmd_sort(
            &[b"SORT".to_vec(), b"k".to_vec(), b"ALPHA".to_vec()],
            &mut db,
        ).unwrap();
        match r {
            Resp::Array(Some(items)) => {
                assert_eq!(items.len(), 3);
                if let Resp::BulkString(Some(b)) = &items[0] { assert_eq!(b, b"apple"); }
                if let Resp::BulkString(Some(b)) = &items[2] { assert_eq!(b, b"cherry"); }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn sort_numeric_descending_with_limit_and_store() {
        let mut db = Db::new();
        let mut l = std::collections::VecDeque::new();
        for v in &["3", "1", "2", "5", "4"] {
            l.push_back(v.as_bytes().to_vec());
        }
        db.insert(b"k".to_vec(), Entry::new(Value::List(l)));

        let r = cmd_sort(
            &[
                b"SORT".to_vec(), b"k".to_vec(),
                b"DESC".to_vec(),
                b"LIMIT".to_vec(), b"0".to_vec(), b"3".to_vec(),
                b"STORE".to_vec(), b"dst".to_vec(),
            ],
            &mut db,
        ).unwrap();
        match r {
            Resp::Integer(n) => assert_eq!(n, 3),
            _ => panic!(),
        }
        if let Some(e) = db.get(b"dst") {
            if let Value::List(l) = &e.value {
                assert_eq!(l.len(), 3);
                assert_eq!(&l[0], b"5");
                assert_eq!(&l[1], b"4");
                assert_eq!(&l[2], b"3");
            } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn sort_ro_rejects_store() {
        let mut db = Db::new();
        db.insert(b"k".to_vec(), Entry::new(Value::List(std::collections::VecDeque::new())));
        let r = cmd_sort_ro(
            &[b"SORT_RO".to_vec(), b"k".to_vec(), b"STORE".to_vec(), b"dst".to_vec()],
            &mut db,
        );
        assert!(r.is_err());
    }
}
