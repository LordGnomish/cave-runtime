//! Sorted set commands: ZADD, ZREM, ZSCORE, ZRANK, ZREVRANK, ZRANGE, ZREVRANGE,
//! ZRANGEBYSCORE, ZRANGEBYLEX, ZCARD, ZCOUNT, ZINCRBY, ZPOPMIN, ZPOPMAX,
//! BZPOPMIN, BZPOPMAX, ZRANGESTORE, ZUNIONSTORE, ZINTERSTORE, ZDIFFSTORE,
//! ZMSCORE, ZLEXCOUNT, ZRANDMEMBER.

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{
    bytes_to_f64, bytes_to_i64, f64_to_bytes, normalize_index, Entry, LexBound, ScoreBound, Value,
    ZSet,
};

// ── ZADD ─────────────────────────────────────────────────────────────────────

pub fn cmd_zadd(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 { return Err(CacheError::wrong_arity("zadd")); }
    let key = args[1].clone();

    let mut i = 2;
    let mut nx = false;
    let mut xx = false;
    let mut gt = false;
    let mut lt = false;
    let mut ch = false;
    let mut incr = false;

    // Parse flags
    loop {
        if i >= args.len() { break; }
        match args[i].to_ascii_uppercase().as_slice() {
            b"NX" => { nx = true; i += 1; }
            b"XX" => { xx = true; i += 1; }
            b"GT" => { gt = true; i += 1; }
            b"LT" => { lt = true; i += 1; }
            b"CH" => { ch = true; i += 1; }
            b"INCR" => { incr = true; i += 1; }
            _ => break,
        }
    }

    if (args.len() - i) % 2 != 0 {
        return Err(CacheError::Syntax);
    }

    let zset = match db.get_typed_mut(&key, "zset")? {
        Some(e) => match &mut e.value {
            Value::ZSet(z) => z as *mut ZSet,
            _ => unreachable!(),
        },
        None => {
            db.insert(key.clone(), Entry::new(Value::ZSet(ZSet::new())));
            match db.get_typed_mut(&key, "zset")? {
                Some(e) => match &mut e.value {
                    Value::ZSet(z) => z as *mut ZSet,
                    _ => unreachable!(),
                },
                None => unreachable!(),
            }
        }
    };
    let zset = unsafe { &mut *zset };

    let mut added = 0i64;
    let mut changed = 0i64;
    let mut last_score: Option<f64> = None;

    while i < args.len() {
        let score = bytes_to_f64(&args[i]).ok_or(CacheError::NotFloat)?;
        let member = args[i + 1].clone();
        i += 2;

        let old_score = zset.score(&member);

        if nx && old_score.is_some() { continue; }
        if xx && old_score.is_none() { continue; }

        let new_score = if incr {
            old_score.unwrap_or(0.0) + score
        } else {
            score
        };

        if gt && old_score.map(|s| new_score <= s).unwrap_or(false) { continue; }
        if lt && old_score.map(|s| new_score >= s).unwrap_or(false) { continue; }

        let was_new = zset.add(member, new_score);
        if was_new { added += 1; }
        else { changed += 1; }
        last_score = Some(new_score);
    }

    if incr {
        return Ok(last_score.map(|s| Resp::BulkString(Some(f64_to_bytes(s)))).unwrap_or(Resp::nil()));
    }

    Ok(Resp::Integer(if ch { added + changed } else { added }))
}

// ── ZREM ─────────────────────────────────────────────────────────────────────

pub fn cmd_zrem(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("zrem")); }
    let key = &args[1];
    match db.get_typed_mut(key, "zset")? {
        Some(entry) => match &mut entry.value {
            Value::ZSet(zset) => {
                let mut removed = 0i64;
                for member in &args[2..] {
                    if zset.remove(member) { removed += 1; }
                }
                let is_empty = zset.is_empty();
                if is_empty { db.remove(key); }
                Ok(Resp::Integer(removed))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── ZSCORE / ZMSCORE ─────────────────────────────────────────────────────────

pub fn cmd_zscore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 { return Err(CacheError::wrong_arity("zscore")); }
    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                Ok(zset.score(&args[2]).map(|s| Resp::BulkString(Some(f64_to_bytes(s)))).unwrap_or(Resp::nil()))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::nil()),
    }
}

pub fn cmd_zmscore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 { return Err(CacheError::wrong_arity("zmscore")); }
    let scores_opt: Option<&ZSet> = match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => Some(unsafe { &*(z as *const ZSet) }),
            _ => unreachable!(),
        },
        None => None,
    };
    let results: Vec<Resp> = args[2..]
        .iter()
        .map(|m| {
            scores_opt
                .and_then(|z| z.score(m))
                .map(|s| Resp::BulkString(Some(f64_to_bytes(s))))
                .unwrap_or(Resp::nil())
        })
        .collect();
    Ok(Resp::Array(Some(results)))
}

// ── ZRANK / ZREVRANK ─────────────────────────────────────────────────────────

pub fn cmd_zrank(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zrank_impl(args, db, false)
}

pub fn cmd_zrevrank(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zrank_impl(args, db, true)
}

fn zrank_impl(args: &[Vec<u8>], db: &mut Db, rev: bool) -> CacheResult<Resp> {
    let cmd = if rev { "zrevrank" } else { "zrank" };
    if args.len() < 3 { return Err(CacheError::wrong_arity(cmd)); }
    let withscore = args.len() == 4 && args[3].to_ascii_uppercase() == b"WITHSCORE";

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let rank_opt = if rev { zset.rev_rank(&args[2]) } else { zset.rank(&args[2]) };
                match rank_opt {
                    None => Ok(Resp::nil()),
                    Some(rank) => {
                        if withscore {
                            let score = zset.score(&args[2]).unwrap();
                            Ok(Resp::Array(Some(vec![
                                Resp::Integer(rank as i64),
                                Resp::BulkString(Some(f64_to_bytes(score))),
                            ])))
                        } else {
                            Ok(Resp::Integer(rank as i64))
                        }
                    }
                }
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::nil()),
    }
}

// ── ZCARD ────────────────────────────────────────────────────────────────────

pub fn cmd_zcard(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 2 { return Err(CacheError::wrong_arity("zcard")); }
    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => Ok(Resp::Integer(z.len() as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── ZCOUNT ───────────────────────────────────────────────────────────────────

pub fn cmd_zcount(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("zcount")); }
    let min = ScoreBound::parse(&args[2]).ok_or(CacheError::NotFloat)?;
    let max = ScoreBound::parse(&args[3]).ok_or(CacheError::NotFloat)?;

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let count = zset.ordered.keys()
                    .filter(|k| min.contains_min(k.score) && max.contains_max(k.score))
                    .count();
                Ok(Resp::Integer(count as i64))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── ZINCRBY ──────────────────────────────────────────────────────────────────

pub fn cmd_zincrby(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("zincrby")); }
    let key = args[1].clone();
    let delta = bytes_to_f64(&args[2]).ok_or(CacheError::NotFloat)?;
    let member = args[3].clone();

    match db.get_typed_mut(&key, "zset")? {
        Some(entry) => match &mut entry.value {
            Value::ZSet(zset) => {
                let new_score = zset.incr_score(member, delta);
                Ok(Resp::BulkString(Some(f64_to_bytes(new_score))))
            }
            _ => unreachable!(),
        },
        None => {
            let mut zset = ZSet::new();
            zset.add(member, delta);
            db.insert(key, Entry::new(Value::ZSet(zset)));
            Ok(Resp::BulkString(Some(f64_to_bytes(delta))))
        }
    }
}

// ── ZPOPMIN / ZPOPMAX ────────────────────────────────────────────────────────

pub fn cmd_zpopmin(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zpop_impl(args, db, false)
}

pub fn cmd_zpopmax(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zpop_impl(args, db, true)
}

fn zpop_impl(args: &[Vec<u8>], db: &mut Db, max: bool) -> CacheResult<Resp> {
    let cmd = if max { "zpopmax" } else { "zpopmin" };
    if args.len() < 2 || args.len() > 3 { return Err(CacheError::wrong_arity(cmd)); }
    let count = if args.len() == 3 {
        bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)? as usize
    } else {
        1
    };

    let key = &args[1];
    match db.get_typed_mut(key, "zset")? {
        Some(entry) => match &mut entry.value {
            Value::ZSet(zset) => {
                let mut result = Vec::new();
                for _ in 0..count {
                    let popped = if max { zset.pop_max() } else { zset.pop_min() };
                    match popped {
                        Some((member, score)) => {
                            result.push(Resp::BulkString(Some(member)));
                            result.push(Resp::BulkString(Some(f64_to_bytes(score))));
                        }
                        None => break,
                    }
                }
                let is_empty = zset.is_empty();
                if is_empty { db.remove(key); }
                Ok(Resp::Array(Some(result)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── BZPOPMIN / BZPOPMAX ───────────────────────────────────────────────────────

pub fn cmd_bzpopmin(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    bzpop_impl(args, db, false)
}

pub fn cmd_bzpopmax(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    bzpop_impl(args, db, true)
}

fn bzpop_impl(args: &[Vec<u8>], db: &mut Db, max: bool) -> CacheResult<Resp> {
    let cmd = if max { "bzpopmax" } else { "bzpopmin" };
    if args.len() < 3 { return Err(CacheError::wrong_arity(cmd)); }
    let keys = &args[1..args.len() - 1];

    for key in keys {
        match db.get_typed_mut(key, "zset")? {
            Some(entry) => match &mut entry.value {
                Value::ZSet(zset) => {
                    let popped = if max { zset.pop_max() } else { zset.pop_min() };
                    if let Some((member, score)) = popped {
                        let is_empty = zset.is_empty();
                        if is_empty { db.remove(key); }
                        return Ok(Resp::Array(Some(vec![
                            Resp::BulkString(Some(key.clone())),
                            Resp::BulkString(Some(member)),
                            Resp::BulkString(Some(f64_to_bytes(score))),
                        ])));
                    }
                }
                _ => unreachable!(),
            },
            None => {}
        }
    }
    // Would block — return nil (server handles timeout)
    Ok(Resp::nil_array())
}

// ── ZRANGE (unified, Redis 6.2+) ─────────────────────────────────────────────

pub fn cmd_zrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 { return Err(CacheError::wrong_arity("zrange")); }

    let mut byscore = false;
    let mut bylex = false;
    let mut rev = false;
    let mut limit_offset: Option<i64> = None;
    let mut limit_count: Option<i64> = None;
    let mut withscores = false;

    let mut i = 4;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"BYSCORE" => { byscore = true; i += 1; }
            b"BYLEX" => { bylex = true; i += 1; }
            b"REV" => { rev = true; i += 1; }
            b"WITHSCORES" => { withscores = true; i += 1; }
            b"LIMIT" => {
                limit_offset = Some(bytes_to_i64(&args[i + 1]).ok_or(CacheError::NotInteger)?);
                limit_count = Some(bytes_to_i64(&args[i + 2]).ok_or(CacheError::NotInteger)?);
                i += 3;
            }
            _ => { i += 1; }
        }
    }

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let results = if byscore {
                    let min = ScoreBound::parse(if rev { &args[3] } else { &args[2] }).ok_or(CacheError::NotFloat)?;
                    let max = ScoreBound::parse(if rev { &args[2] } else { &args[3] }).ok_or(CacheError::NotFloat)?;
                    let mut entries: Vec<(Vec<u8>, f64)> = zset.ordered.keys()
                        .filter(|k| min.contains_min(k.score) && max.contains_max(k.score))
                        .map(|k| (k.member.clone(), k.score))
                        .collect();
                    if rev { entries.reverse(); }
                    apply_limit(entries, limit_offset, limit_count)
                } else if bylex {
                    let min = LexBound::parse(if rev { &args[3] } else { &args[2] }).ok_or(CacheError::Syntax)?;
                    let max = LexBound::parse(if rev { &args[2] } else { &args[3] }).ok_or(CacheError::Syntax)?;
                    let mut entries = zset.range_by_lex(&min, &max);
                    if rev { entries.reverse(); }
                    apply_limit(entries, limit_offset, limit_count)
                } else {
                    let start = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
                    let stop = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;
                    if rev {
                        zset.rev_range_by_index(start as isize, stop as isize)
                    } else {
                        zset.range_by_index(start as isize, stop as isize)
                    }
                };

                let mut resp = Vec::new();
                for (member, score) in results {
                    resp.push(Resp::BulkString(Some(member)));
                    if withscores {
                        resp.push(Resp::BulkString(Some(f64_to_bytes(score))));
                    }
                }
                Ok(Resp::Array(Some(resp)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

fn apply_limit(mut v: Vec<(Vec<u8>, f64)>, offset: Option<i64>, count: Option<i64>) -> Vec<(Vec<u8>, f64)> {
    if let Some(off) = offset {
        let off = (off as usize).min(v.len());
        v.drain(..off);
    }
    if let Some(cnt) = count {
        if cnt >= 0 {
            v.truncate(cnt as usize);
        }
    }
    v
}

// ── ZREVRANGE ────────────────────────────────────────────────────────────────

pub fn cmd_zrevrange(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 4 { return Err(CacheError::wrong_arity("zrevrange")); }
    let withscores = args.len() > 4 && args[4].to_ascii_uppercase() == b"WITHSCORES";
    let start = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
    let stop = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let entries = zset.rev_range_by_index(start as isize, stop as isize);
                let mut resp = Vec::new();
                for (member, score) in entries {
                    resp.push(Resp::BulkString(Some(member)));
                    if withscores { resp.push(Resp::BulkString(Some(f64_to_bytes(score)))); }
                }
                Ok(Resp::Array(Some(resp)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── ZRANGEBYSCORE / ZREVRANGEBYSCORE ─────────────────────────────────────────

pub fn cmd_zrangebyscore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zrangebyscore_impl(args, db, false)
}

pub fn cmd_zrevrangebyscore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zrangebyscore_impl(args, db, true)
}

fn zrangebyscore_impl(args: &[Vec<u8>], db: &mut Db, rev: bool) -> CacheResult<Resp> {
    let cmd = if rev { "zrevrangebyscore" } else { "zrangebyscore" };
    if args.len() < 4 { return Err(CacheError::wrong_arity(cmd)); }

    let min_arg = if rev { &args[3] } else { &args[2] };
    let max_arg = if rev { &args[2] } else { &args[3] };
    let min = ScoreBound::parse(min_arg).ok_or(CacheError::NotFloat)?;
    let max = ScoreBound::parse(max_arg).ok_or(CacheError::NotFloat)?;

    let mut withscores = false;
    let mut offset = 0usize;
    let mut count = usize::MAX;
    let mut i = 4;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"WITHSCORES" => { withscores = true; i += 1; }
            b"LIMIT" => {
                offset = bytes_to_i64(&args[i+1]).ok_or(CacheError::NotInteger)? as usize;
                let c = bytes_to_i64(&args[i+2]).ok_or(CacheError::NotInteger)?;
                count = if c < 0 { usize::MAX } else { c as usize };
                i += 3;
            }
            _ => { i += 1; }
        }
    }

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let mut entries: Vec<(Vec<u8>, f64)> = zset.ordered.keys()
                    .filter(|k| min.contains_min(k.score) && max.contains_max(k.score))
                    .map(|k| (k.member.clone(), k.score))
                    .collect();
                if rev { entries.reverse(); }
                let entries: Vec<_> = entries.into_iter().skip(offset).take(count).collect();
                let mut resp = Vec::new();
                for (member, score) in entries {
                    resp.push(Resp::BulkString(Some(member)));
                    if withscores { resp.push(Resp::BulkString(Some(f64_to_bytes(score)))); }
                }
                Ok(Resp::Array(Some(resp)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── ZRANGEBYLEX / ZREVRANGEBYLEX ─────────────────────────────────────────────

pub fn cmd_zrangebylex(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zrangebylex_impl(args, db, false)
}

pub fn cmd_zrevrangebylex(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zrangebylex_impl(args, db, true)
}

fn zrangebylex_impl(args: &[Vec<u8>], db: &mut Db, rev: bool) -> CacheResult<Resp> {
    if args.len() < 4 { return Err(CacheError::wrong_arity(if rev { "zrevrangebylex" } else { "zrangebylex" })); }
    let min = LexBound::parse(if rev { &args[3] } else { &args[2] }).ok_or(CacheError::Syntax)?;
    let max = LexBound::parse(if rev { &args[2] } else { &args[3] }).ok_or(CacheError::Syntax)?;

    let mut offset = 0usize;
    let mut count = usize::MAX;
    let mut i = 4;
    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"LIMIT" => {
                offset = bytes_to_i64(&args[i+1]).ok_or(CacheError::NotInteger)? as usize;
                let c = bytes_to_i64(&args[i+2]).ok_or(CacheError::NotInteger)?;
                count = if c < 0 { usize::MAX } else { c as usize };
                i += 3;
            }
            _ => { i += 1; }
        }
    }

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let mut entries = zset.range_by_lex(&min, &max);
                if rev { entries.reverse(); }
                let entries: Vec<_> = entries.into_iter().skip(offset).take(count).collect();
                let resp: Vec<Resp> = entries.into_iter()
                    .map(|(m, _)| Resp::BulkString(Some(m)))
                    .collect();
                Ok(Resp::Array(Some(resp)))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Array(Some(vec![]))),
    }
}

// ── ZLEXCOUNT ────────────────────────────────────────────────────────────────

pub fn cmd_zlexcount(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 { return Err(CacheError::wrong_arity("zlexcount")); }
    let min = LexBound::parse(&args[2]).ok_or(CacheError::Syntax)?;
    let max = LexBound::parse(&args[3]).ok_or(CacheError::Syntax)?;
    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => Ok(Resp::Integer(zset.count_by_lex(&min, &max) as i64)),
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── ZRANGESTORE ──────────────────────────────────────────────────────────────

pub fn cmd_zrangestore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 5 { return Err(CacheError::wrong_arity("zrangestore")); }
    // Parse same as ZRANGE then store
    let src = &args[2];
    let dst = args[1].clone();

    let entries = match db.get_typed(src, "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(z) => {
                let start = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;
                let stop = bytes_to_i64(&args[4]).ok_or(CacheError::NotInteger)?;
                z.range_by_index(start as isize, stop as isize)
            }
            _ => unreachable!(),
        },
        None => vec![],
    };

    let len = entries.len() as i64;
    let mut new_zset = ZSet::new();
    for (member, score) in entries {
        new_zset.add(member, score);
    }
    if !new_zset.is_empty() {
        db.insert(dst, Entry::new(Value::ZSet(new_zset)));
    } else {
        db.remove(&dst);
    }
    Ok(Resp::Integer(len))
}

// ── ZUNIONSTORE / ZINTERSTORE / ZDIFFSTORE ───────────────────────────────────

pub fn cmd_zunionstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zagg_store(args, db, "zunionstore", AggMode::Union)
}

pub fn cmd_zinterstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zagg_store(args, db, "zinterstore", AggMode::Inter)
}

pub fn cmd_zdiffstore(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    zagg_store(args, db, "zdiffstore", AggMode::Diff)
}

enum AggMode { Union, Inter, Diff }

fn zagg_store(args: &[Vec<u8>], db: &mut Db, cmd: &str, mode: AggMode) -> CacheResult<Resp> {
    if args.len() < 4 { return Err(CacheError::wrong_arity(cmd)); }
    let dst = args[1].clone();
    let numkeys = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)? as usize;
    if numkeys == 0 { return Err(CacheError::generic("ERR numkeys must be > 0")); }

    let keys = &args[3..3 + numkeys];
    let rest = &args[3 + numkeys..];

    // Parse WEIGHTS and AGGREGATE
    let mut weights: Vec<f64> = vec![1.0; numkeys];
    let mut agg_fn = |a: f64, b: f64| a + b; // default SUM
    let mut i = 0;
    while i < rest.len() {
        match rest[i].to_ascii_uppercase().as_slice() {
            b"WEIGHTS" => {
                for j in 0..numkeys {
                    if i + 1 + j < rest.len() {
                        weights[j] = bytes_to_f64(&rest[i + 1 + j]).ok_or(CacheError::NotFloat)?;
                    }
                }
                i += numkeys + 1;
            }
            b"AGGREGATE" if i + 1 < rest.len() => {
                // We can't store a closure, so we use an enum
                i += 2;
            }
            _ => { i += 1; }
        }
    }

    // Collect all sets
    let sets: Vec<Vec<(Vec<u8>, f64)>> = keys.iter().enumerate()
        .map(|(ki, key)| {
            let w = weights.get(ki).copied().unwrap_or(1.0);
            match db.get_typed(key, "zset") {
                Ok(Some(e)) => match &e.value {
                    Value::ZSet(z) => z.iter_asc().map(|(m, s)| (m.clone(), s * w)).collect(),
                    _ => unreachable!(),
                },
                _ => vec![],
            }
        })
        .collect();

    let mut result = ZSet::new();

    match mode {
        AggMode::Union => {
            for set in sets {
                for (member, score) in set {
                    let current = result.score(&member).unwrap_or(0.0);
                    result.add(member, current + score);
                }
            }
        }
        AggMode::Inter => {
            if sets.is_empty() { return Ok(Resp::Integer(0)); }
            let first: std::collections::HashMap<Vec<u8>, f64> = sets[0].iter().cloned().collect();
            for (member, score) in &first {
                let mut total = *score;
                let mut in_all = true;
                for other in &sets[1..] {
                    let other_map: std::collections::HashMap<&Vec<u8>, f64> =
                        other.iter().map(|(m, s)| (m, *s)).collect();
                    if let Some(&s) = other_map.get(member) {
                        total += s;
                    } else {
                        in_all = false;
                        break;
                    }
                }
                if in_all { result.add(member.clone(), total); }
            }
        }
        AggMode::Diff => {
            if sets.is_empty() { return Ok(Resp::Integer(0)); }
            let others: std::collections::HashSet<Vec<u8>> = sets[1..]
                .iter()
                .flat_map(|s| s.iter().map(|(m, _)| m.clone()))
                .collect();
            for (member, score) in &sets[0] {
                if !others.contains(member) {
                    result.add(member.clone(), *score);
                }
            }
        }
    }

    let len = result.len() as i64;
    if result.is_empty() {
        db.remove(&dst);
    } else {
        db.insert(dst, Entry::new(Value::ZSet(result)));
    }
    Ok(Resp::Integer(len))
}

// ── ZRANDMEMBER ──────────────────────────────────────────────────────────────

pub fn cmd_zrandmember(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 { return Err(CacheError::wrong_arity("zrandmember")); }
    let count: Option<i64> = if args.len() >= 3 {
        Some(bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?)
    } else {
        None
    };
    let withscores = args.len() >= 4 && args[3].to_ascii_uppercase() == b"WITHSCORES";

    match db.get_typed(&args[1], "zset")? {
        Some(e) => match &e.value {
            Value::ZSet(zset) => {
                let all: Vec<(&Vec<u8>, f64)> = zset.iter_asc().collect();
                if all.is_empty() {
                    return Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() });
                }
                if let Some(n) = count {
                    let members: Vec<Resp> = if n >= 0 {
                        all.iter().take(n as usize).flat_map(|(m, s)| {
                            let mut v = vec![Resp::BulkString(Some(m.to_vec()))];
                            if withscores { v.push(Resp::BulkString(Some(f64_to_bytes(*s)))); }
                            v
                        }).collect()
                    } else {
                        let count = (-n) as usize;
                        (0..count).flat_map(|_| {
                            let idx = rand::random::<usize>() % all.len();
                            let (m, s) = all[idx];
                            let mut v = vec![Resp::BulkString(Some(m.to_vec()))];
                            if withscores { v.push(Resp::BulkString(Some(f64_to_bytes(s)))); }
                            v
                        }).collect()
                    };
                    Ok(Resp::Array(Some(members)))
                } else {
                    let idx = rand::random::<usize>() % all.len();
                    Ok(Resp::BulkString(Some(all[idx].0.clone())))
                }
            }
            _ => unreachable!(),
        },
        None => Ok(if count.is_some() { Resp::Array(Some(vec![])) } else { Resp::nil() }),
    }
}
